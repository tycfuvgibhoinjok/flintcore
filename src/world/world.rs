use std::cell::RefCell;
use std::collections::HashMap;

use rayon::prelude::*;

use crate::config::WorldConfig;
use crate::gameplay::blocks;
use crate::net::packets::chunk::{MapChunk, PreChunk};
use crate::util::{Error, Result};
use crate::world::biome::Biome;
use crate::world::chunk::{Chunk, ChunkPos, ChunkState, CHUNK_HEIGHT};
use crate::world::decorate::GenWorld;
use crate::world::generator::Generator;

thread_local! {
    static GENERATOR: RefCell<Option<(i64, Generator)>> = const { RefCell::new(None) };
}

fn with_generator<R>(seed: i64, f: impl FnOnce(&mut Generator) -> R) -> R {
    GENERATOR.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.as_ref().map(|(s, _)| *s) != Some(seed) {
            *opt = Some((seed, Generator::new(seed)));
        }
        let (_, generator) = opt.as_mut().unwrap();
        f(generator)
    })
}

pub struct World {
    config: WorldConfig,
    chunks: HashMap<ChunkPos, Chunk>,
}

impl World {
    pub fn new(config: WorldConfig) -> Self {
        Self {
            config,
            chunks: HashMap::new(),
        }
    }

    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    pub fn seed(&self) -> i64 {
        self.config.seed
    }

    pub fn spawn_point(&self) -> (i32, i32, i32) {
        (
            self.config.spawn_x,
            self.config.spawn_y,
            self.config.spawn_z,
        )
    }

    pub fn safe_spawn_point(&mut self) -> (i32, i32, i32) {
        let (x, default_y, z) = self.spawn_point();
        let chunk_pos = ChunkPos::new(x >> 4, z >> 4);
        self.populate_chunk(chunk_pos);
        let ground = self.highest_solid_y(x, z);
        let y = if ground > 0 {
            ground
        } else {
            default_y.max(64)
        };
        (x, y, z)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    pub fn get_chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn get_chunk_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.chunks.get_mut(&pos)
    }

    pub fn ensure_chunk_base(&mut self, pos: ChunkPos) -> &mut Chunk {
        if !self.chunks.contains_key(&pos) {
            let seed = self.config.seed;
            let chunk = with_generator(seed, |gen| gen.generate_base(pos.x, pos.z));
            self.chunks.insert(pos, chunk);
        }
        self.chunks.get_mut(&pos).unwrap()
    }

    pub fn populate_chunk(&mut self, pos: ChunkPos) {
        if let Some(chunk) = self.chunks.get(&pos) {
            if chunk.state == ChunkState::Populated {
                return;
            }
        }

        // Ensure 5x5 region around target chunk has base terrain generated
        // so decorators extending slightly over borders find valid chunks.
        for dz in -2..=2 {
            for dx in -2..=2 {
                self.ensure_chunk_base(ChunkPos::new(pos.x + dx, pos.z + dz));
            }
        }

        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.state = ChunkState::Populating;
        }

        let seed = self.config.seed;
        with_generator(seed, |gen| {
            gen.populate(self, pos.x, pos.z);
        });

        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.state = ChunkState::Populated;
        }
    }

    pub fn generate_base_chunks_parallel(&mut self, positions: &[ChunkPos]) {
        let needed: Vec<ChunkPos> = positions
            .iter()
            .copied()
            .filter(|pos| !self.chunks.contains_key(pos))
            .collect();

        if needed.is_empty() {
            return;
        }

        let seed = self.config.seed;
        let generated: Vec<(ChunkPos, Chunk)> = needed
            .par_iter()
            .map(|&pos| {
                let chunk = with_generator(seed, |gen| gen.generate_base(pos.x, pos.z));
                (pos, chunk)
            })
            .collect();

        for (pos, chunk) in generated {
            self.chunks.insert(pos, chunk);
        }
    }

    pub fn prepare_chunk_packets(&self, pos: ChunkPos) -> Result<(PreChunk, MapChunk)> {
        let chunk = self
            .chunks
            .get(&pos)
            .ok_or(Error::InvalidState("chunk not loaded for packet prep"))?;
        let pre_chunk = PreChunk {
            x: pos.x,
            z: pos.z,
            mode: true,
        };
        let map_chunk = MapChunk::from_chunk(chunk)?;
        Ok((pre_chunk, map_chunk))
    }

    pub fn compress_chunks_parallel(
        &self,
        positions: &[ChunkPos],
    ) -> Result<Vec<(PreChunk, MapChunk)>> {
        let mut chunks = Vec::with_capacity(positions.len());
        for &pos in positions {
            let chunk = self.chunks.get(&pos).ok_or(Error::InvalidState(
                "chunk not loaded for parallel compression",
            ))?;
            chunks.push(chunk);
        }

        chunks
            .par_iter()
            .map(|&chunk| {
                let pre = PreChunk {
                    x: chunk.pos.x,
                    z: chunk.pos.z,
                    mode: true,
                };
                let map = MapChunk::from_chunk(chunk)?;
                Ok((pre, map))
            })
            .collect()
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return blocks::AIR;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_block((x & 15) as usize, y as usize, (z & 15) as usize)
        } else {
            blocks::AIR
        }
    }

    pub fn get_data(&self, x: i32, y: i32, z: i32) -> u8 {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return 0;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_metadata((x & 15) as usize, y as usize, (z & 15) as usize)
        } else {
            0
        }
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return;
        }
        let cx = x >> 4;
        let cz = z >> 4;
        let pos = ChunkPos::new(cx, cz);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.set_block((x & 15) as usize, y as usize, (z & 15) as usize, id);
        }
        self.recalc_height(cx, cz, (x & 15) as usize, (z & 15) as usize);
    }

    pub fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return;
        }
        let cx = x >> 4;
        let cz = z >> 4;
        let pos = ChunkPos::new(cx, cz);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.set_block((x & 15) as usize, y as usize, (z & 15) as usize, id);
            chunk.set_metadata((x & 15) as usize, y as usize, (z & 15) as usize, data);
        }
        self.recalc_height(cx, cz, (x & 15) as usize, (z & 15) as usize);
    }

    fn recalc_height(&mut self, cx: i32, cz: i32, lx: usize, lz: usize) {
        if let Some(chunk) = self.chunks.get_mut(&ChunkPos::new(cx, cz)) {
            let mut height = CHUNK_HEIGHT as i32 - 1;
            while height > 0
                && blocks::light_opacity(chunk.get_block(lx, height as usize - 1, lz)) == 0
            {
                height -= 1;
            }
            chunk.height_map[(lz << 4) | lx] = height as u8;
        }
    }
}

impl GenWorld for World {
    fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        self.get_block(x, y, z)
    }

    fn get_data(&self, x: i32, y: i32, z: i32) -> u8 {
        self.get_data(x, y, z)
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.set_block(x, y, z, id);
    }

    fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.set_block_and_data(x, y, z, id, data);
    }

    fn highest_block_y(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_height((x & 15) as usize, (z & 15) as usize) as i32
        } else {
            0
        }
    }

    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        if y < 0 {
            return 0;
        }
        if y >= CHUNK_HEIGHT as i32 {
            return 15;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_sky_light((x & 15) as usize, y as usize, (z & 15) as usize) as i32
        } else {
            15
        }
    }

    fn light_value(&self, x: i32, y: i32, z: i32) -> i32 {
        if y < 0 {
            return 0;
        }
        if y >= CHUNK_HEIGHT as i32 {
            return 15;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_block_light_value((x & 15) as usize, y as usize, (z & 15) as usize, 0)
        } else {
            15
        }
    }

    fn see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        if y < 0 {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.can_block_see_sky((x & 15) as usize, y as usize, (z & 15) as usize)
        } else {
            true
        }
    }

    fn highest_solid_y(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            for y in (1..CHUNK_HEIGHT as i32).rev() {
                let id = chunk.get_block((x & 15) as usize, y as usize, (z & 15) as usize);
                if blocks::is_solid(id) || blocks::is_liquid(id) {
                    return y + 1;
                }
            }
        }
        -1
    }

    fn is_normal_solid(&self, x: i32, y: i32, z: i32) -> bool {
        blocks::is_normal_solid(self.get_block(x, y, z))
    }

    fn biome_at(&self, _x: i32, _z: i32) -> Biome {
        Biome::Tundra
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_creates_and_populates_chunk() {
        let config = WorldConfig {
            seed: 12345,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        let pos = ChunkPos::new(0, 0);
        world.populate_chunk(pos);

        let chunk = world.get_chunk(pos).unwrap();
        assert_eq!(chunk.state, ChunkState::Populated);
        assert!(world.chunk_count() >= 9);

        let (pre, map) = world.prepare_chunk_packets(pos).unwrap();
        assert_eq!(pre.x, 0);
        assert_eq!(pre.z, 0);
        assert!(pre.mode);
        assert_eq!(map.x, 0);
        assert_eq!(map.z, 0);
        assert!(!map.compressed.is_empty());
    }

    #[test]
    fn parallel_generation_and_compression_match() {
        let config = WorldConfig {
            seed: 12345,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        let positions = [
            ChunkPos::new(0, 0),
            ChunkPos::new(1, 0),
            ChunkPos::new(0, 1),
            ChunkPos::new(-1, -1),
        ];

        world.generate_base_chunks_parallel(&positions);
        for &pos in &positions {
            assert!(world.is_chunk_loaded(pos));
        }

        let packets = world.compress_chunks_parallel(&positions).unwrap();
        assert_eq!(packets.len(), positions.len());
        for (i, (pre, map)) in packets.iter().enumerate() {
            assert_eq!(pre.x, positions[i].x);
            assert_eq!(pre.z, positions[i].z);
            assert_eq!(map.x, positions[i].x * 16);
            assert_eq!(map.z, positions[i].z * 16);
            assert!(!map.compressed.is_empty());
        }
    }
}
