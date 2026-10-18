use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::config::WorldConfig;
use crate::gameplay::blocks;
use crate::net::packets::chunk::{MapChunk, PreChunk};
use crate::util::rng::JavaRandom;
use crate::util::{Error, Result};
use crate::world::biome::{Biome, WorldChunkManager};
use crate::world::chunk::{Chunk, ChunkPos, ChunkState, CHUNK_HEIGHT};
use crate::world::decorate::GenWorld;
use crate::world::generator::Generator;
use crate::world::lighting::{LightType, LightingWorld};
use crate::world::storage::{ChunkLoader, LevelData, RegionFileCache, SessionLock};

thread_local! {
    static GENERATOR: RefCell<Option<(i64, Generator)>> = const { RefCell::new(None) };
}

/// Debug tracing of the Java block-write pipeline. Set `FLINTCORE_TRACE` to
/// `x0,y0,z0,x1,y1,z1` to print every pipeline event inside that box. The
/// output matches the instrumented Poseidon reference dump byte for byte and
/// is used to locate divergences between the two generators.
pub(crate) fn trace_region() -> Option<(i32, i32, i32, i32, i32, i32)> {
    use std::sync::OnceLock;
    static REGION: OnceLock<Option<(i32, i32, i32, i32, i32, i32)>> = OnceLock::new();
    *REGION.get_or_init(|| {
        std::env::var("FLINTCORE_TRACE").ok().and_then(|s| {
            let p: Vec<i32> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if p.len() == 6 {
                Some((p[0], p[1], p[2], p[3], p[4], p[5]))
            } else {
                None
            }
        })
    })
}

fn trace_in(x: i32, y: i32, z: i32) -> bool {
    trace_region()
        .map(|(x0, y0, z0, x1, y1, z1)| {
            x >= x0 && x <= x1 && y >= y0 && y <= y1 && z >= z0 && z <= z1
        })
        .unwrap_or(false)
}

fn trace_g_in(x: i32, z: i32) -> bool {
    trace_region()
        .map(|(x0, _, z0, x1, _, z1)| x >= x0 && x <= x1 && z >= z0 && z <= z1)
        .unwrap_or(false)
}

macro_rules! trace_a4 {
    ($x:expr, $y:expr, $z:expr, $old:expr, $new:expr) => {
        if trace_in($x, $y, $z) {
            println!("A4 {} {} {} {}->{}", $x, $y, $z, $old, $new);
        }
    };
}

macro_rules! trace_a5 {
    ($x:expr, $y:expr, $z:expr, $old:expr, $oldd:expr, $new:expr, $newd:expr) => {
        if trace_in($x, $y, $z) {
            println!(
                "A5 {} {} {} {}.{}->{}.{}",
                $x, $y, $z, $old, $oldd, $new, $newd
            );
        }
    };
}

macro_rules! trace_b {
    ($x:expr, $y:expr, $z:expr, $oldd:expr, $newd:expr) => {
        if trace_in($x, $y, $z) {
            println!("B {} {} {} {}->{}", $x, $y, $z, $oldd, $newd);
        }
    };
}

macro_rules! trace_g {
    ($x:expr, $z:expr, $oldh:expr, $newh:expr) => {
        if trace_g_in($x, $z) {
            println!("G {} {} {}->{}", $x, $z, $oldh, $newh);
        }
    };
}

macro_rules! trace_phys {
    ($x:expr, $y:expr, $z:expr, $l:expr, $id:expr) => {
        if trace_in($x, $y, $z) {
            println!("PHYS {} {} {} l={} id={}", $x, $y, $z, $l, $id);
        }
    };
}

macro_rules! trace_fa {
    ($x:expr, $y:expr, $z:expr, $id:expr, $data:expr) => {
        if trace_in($x, $y, $z) {
            println!("FA {} {} {} id={} d={}", $x, $y, $z, $id, $data);
        }
    };
}

macro_rules! trace_conv {
    ($x:expr, $y:expr, $z:expr, $oldd:expr, $new:expr) => {
        if trace_in($x, $y, $z) {
            println!("CONV {} {} {} d={} ->{}", $x, $y, $z, $oldd, $new);
        }
    };
}

macro_rules! trace_rng {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("RNG {} {} {}", $x, $y, $z);
        }
    };
}

macro_rules! trace_rng2 {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("RNG2 {} {} {}", $x, $y, $z);
        }
    };
}

macro_rules! trace_rng3 {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("RNG3 {} {} {}", $x, $y, $z);
        }
    };
}

macro_rules! trace_j {
    ($x:expr, $y:expr, $z:expr, $c:expr, $b:expr) => {
        if trace_in($x, $y, $z) {
            println!(
                "J {} {} {} c={},{},{},{} b={}{}{}{}",
                $x,
                $y,
                $z,
                $c[0],
                $c[1],
                $c[2],
                $c[3],
                $b[0] as i32,
                $b[1] as i32,
                $b[2] as i32,
                $b[3] as i32
            );
        }
    };
}

macro_rules! trace_flow {
    ($x:expr, $y:expr, $z:expr, $l:expr) => {
        if trace_in($x, $y, $z) {
            println!("FLOW {} {} {} l={}", $x, $y, $z, $l);
        }
    };
}

macro_rules! trace_down {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("DOWN {} {} {}", $x, $y, $z);
        }
    };
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
    world_dir: PathBuf,
    chunks: HashMap<ChunkPos, Chunk>,
    packet_cache: RefCell<HashMap<ChunkPos, (PreChunk, MapChunk)>>,
    chunk_mgr: RefCell<WorldChunkManager>,
    storage: Option<RefCell<RegionFileCache>>,
    level_data: RefCell<LevelData>,
    _session_lock: Option<SessionLock>,
    suppress_physics: bool,
    /// Java `World.a`: while true, `World.c`/`World.d` executes scheduled
    /// block ticks immediately (recursively). `WorldGenLiquids` enables it
    /// around the direct fluid tick, which cascades through flowed blocks.
    insta_tick: bool,
    /// Java `World.random` (a fresh `java.util.Random`) — only reachable during
    /// instant-tick cascades (lava flow randomness). Kept here so the port
    /// mirrors the reference implementation.
    world_random: JavaRandom,
    /// `BlockFlowing.b` — the per-direction flow flags returned by `j()`. In
    /// Java this is a mutable field on the shared `BlockFlowing` singleton:
    /// nested cascade ticks call `j()` again and overwrite it, and the outer
    /// flow loop then reads the clobbered flags. Indexed by [lava][direction].
    fluid_sides: [[bool; 4]; 2],
}

impl World {
    pub fn new(mut config: WorldConfig) -> Self {
        let world_dir = PathBuf::from(&config.level_name);

        let (_session_lock, storage, level_data) = if config.save_enabled {
            let lock = SessionLock::acquire(&world_dir).ok();
            let loaded = LevelData::load_from_dir(&world_dir).ok().flatten();
            let level = if let Some(lvl) = loaded {
                config.seed = lvl.seed;
                config.spawn_x = lvl.spawn_x;
                config.spawn_y = lvl.spawn_y;
                config.spawn_z = lvl.spawn_z;
                lvl
            } else {
                LevelData {
                    seed: config.seed,
                    spawn_x: config.spawn_x,
                    spawn_y: config.spawn_y,
                    spawn_z: config.spawn_z,
                    level_name: config.level_name.clone(),
                    ..LevelData::default()
                }
            };
            let cache = RegionFileCache::new(&world_dir);
            (lock, Some(RefCell::new(cache)), RefCell::new(level))
        } else {
            let level = LevelData {
                seed: config.seed,
                spawn_x: config.spawn_x,
                spawn_y: config.spawn_y,
                spawn_z: config.spawn_z,
                level_name: config.level_name.clone(),
                ..LevelData::default()
            };
            (None, None, RefCell::new(level))
        };

        let chunk_mgr = RefCell::new(WorldChunkManager::new(config.seed));

        Self {
            config,
            world_dir,
            chunks: HashMap::new(),
            packet_cache: RefCell::new(HashMap::new()),
            chunk_mgr,
            storage,
            level_data,
            _session_lock,
            suppress_physics: false,
            insta_tick: false,
            world_random: JavaRandom::new(0),
            fluid_sides: [[false; 4]; 2],
        }
    }

    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    pub fn world_dir(&self) -> &Path {
        &self.world_dir
    }

    pub fn world_time(&self) -> i64 {
        self.level_data.borrow().time
    }

    pub fn increment_time(&self) -> i64 {
        let mut data = self.level_data.borrow_mut();
        data.time += 1;
        data.time
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
            let loaded_chunk = if let Some(storage) = &self.storage {
                match storage.borrow_mut().read_chunk(pos.x, pos.z) {
                    Ok(Some(nbt)) => ChunkLoader::chunk_from_nbt(&nbt).ok(),
                    _ => None,
                }
            } else {
                None
            };

            let chunk = match loaded_chunk {
                Some(c) => c,
                None => {
                    let seed = self.config.seed;
                    with_generator(seed, |gen| gen.generate_base(pos.x, pos.z))
                }
            };
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

        // Ensure surrounding chunks have base terrain generated
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
            chunk.dirty = true;
        }
        self.packet_cache.borrow_mut().remove(&pos);
    }

    pub fn save_all(&mut self) -> Result<()> {
        if let Some(storage) = &self.storage {
            let time = self.level_data.borrow().time;
            for (&pos, chunk) in &mut self.chunks {
                if chunk.dirty {
                    let nbt = ChunkLoader::chunk_to_nbt(chunk, time);
                    storage.borrow_mut().write_chunk(pos.x, pos.z, &nbt)?;
                    chunk.dirty = false;
                }
            }
            storage.borrow_mut().flush_all()?;
            self.level_data.borrow_mut().save_to_dir(&self.world_dir)?;
        }
        Ok(())
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
        if let Some(cached) = self.packet_cache.borrow().get(&pos) {
            return Ok(cached.clone());
        }

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
        let result = (pre_chunk, map_chunk);
        self.packet_cache.borrow_mut().insert(pos, result.clone());
        Ok(result)
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

    // ---------------------------------------------------------------------
    // Java block-write pipeline (Chunk.a / World.setTypeId semantics).
    //
    // The generator must reproduce Poseidon's exact write path: `Block.remove`
    // callbacks, height-map updates (`Chunk.g`, including its direct sky-light
    // writes), `Block.c` on-place callbacks and physics propagation
    // (`World.update` -> `applyPhysics` -> `Block.doPhysics`).
    // ---------------------------------------------------------------------

    /// Java `Chunk.a(i, j, k, l)` (4-arg form): raw type write with callbacks.
    pub fn java_set_raw_type_id(&mut self, x: i32, y: i32, z: i32, id: u8) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let lx = (x & 15) as usize;
        let lz = (z & 15) as usize;
        let (old_id, old_height) = match self.chunks.get(&pos) {
            Some(chunk) => (
                chunk.get_block(lx, y as usize, lz),
                chunk.height_map[(lz << 4) | lx] as i32,
            ),
            None => return false,
        };
        if old_id == id {
            return false;
        }
        trace_a4!(x, y, z, old_id, id);

        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_block(lx, y as usize, lz, id);
            chunk.dirty = true;
        }
        self.packet_cache.borrow_mut().remove(&pos);

        // `Block.remove(old)` runs before the data reset, so a removal callback
        // (leaves/log marking) still sees the previous metadata of this block.
        if old_id != 0 {
            self.block_remove(old_id, x, y, z);
        }
        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_metadata(lx, y as usize, lz, 0);
        }

        if blocks::light_opacity(id) != 0 {
            if y >= old_height {
                self.chunk_height_update(pos.x, pos.z, lx, y + 1, lz);
            }
        } else if y == old_height - 1 {
            self.chunk_height_update(pos.x, pos.z, lx, y, lz);
        }

        if id != 0 {
            self.block_on_place(id, x, y, z);
        }
        true
    }

    /// Java `Chunk.a(i, j, k, l, i1)` (5-arg form): raw type+data write.
    pub fn java_set_raw_type_id_and_data(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        id: u8,
        data: u8,
    ) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let lx = (x & 15) as usize;
        let lz = (z & 15) as usize;
        let (old_id, old_data, old_height) = match self.chunks.get(&pos) {
            Some(chunk) => (
                chunk.get_block(lx, y as usize, lz),
                chunk.get_metadata(lx, y as usize, lz),
                chunk.height_map[(lz << 4) | lx] as i32,
            ),
            None => return false,
        };
        if old_id == id && old_data == data {
            return false;
        }
        trace_a5!(x, y, z, old_id, old_data, id, data);

        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_block(lx, y as usize, lz, id);
            chunk.set_metadata(lx, y as usize, lz, data);
            chunk.dirty = true;
        }
        self.packet_cache.borrow_mut().remove(&pos);

        if old_id != 0 {
            self.block_remove(old_id, x, y, z);
        }

        if blocks::light_opacity(id) != 0 {
            if y >= old_height {
                self.chunk_height_update(pos.x, pos.z, lx, y + 1, lz);
            }
        } else if y == old_height - 1 {
            self.chunk_height_update(pos.x, pos.z, lx, y, lz);
        }

        if id != 0 {
            self.block_on_place(id, x, y, z);
        }
        true
    }

    /// Java `World.setRawData(i, j, k, l)`: metadata write only.
    pub fn java_set_raw_data(&mut self, x: i32, y: i32, z: i32, data: u8) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            let old = chunk.get_metadata((x & 15) as usize, y as usize, (z & 15) as usize);
            trace_b!(x, y, z, old, data);
            chunk.set_metadata((x & 15) as usize, y as usize, (z & 15) as usize, data);
            chunk.dirty = true;
            self.packet_cache.borrow_mut().remove(&pos);
            true
        } else {
            false
        }
    }

    /// Java `World.setData(i, j, k, l)`: metadata write followed by physics.
    pub fn java_set_data(&mut self, x: i32, y: i32, z: i32, data: u8) -> bool {
        if self.java_set_raw_data(x, y, z, data) {
            let id = self.get_block(x, y, z);
            self.java_apply_physics(x, y, z, id as i32);
            true
        } else {
            false
        }
    }

    /// Java `World.setTypeId(i, j, k, l)`.
    pub fn java_set_type_id(&mut self, x: i32, y: i32, z: i32, id: u8) -> bool {
        let old = self.get_block(x, y, z);
        if self.java_set_raw_type_id(x, y, z, id) {
            self.java_update(x, y, z, if id == 0 { old as i32 } else { id as i32 });
            true
        } else {
            false
        }
    }

    /// Java `World.setTypeIdAndData(i, j, k, l, i1)`.
    pub fn java_set_type_id_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) -> bool {
        let old = self.get_block(x, y, z);
        if self.java_set_raw_type_id_and_data(x, y, z, id, data) {
            self.java_update(x, y, z, if id == 0 { old as i32 } else { id as i32 });
            true
        } else {
            false
        }
    }

    fn java_update(&mut self, x: i32, y: i32, z: i32, l: i32) {
        // `notify` only informs IWorldAccess listeners (client updates); no-op here.
        self.java_apply_physics(x, y, z, l);
    }

    fn java_apply_physics(&mut self, x: i32, y: i32, z: i32, l: i32) {
        self.java_neighbor_physics(x - 1, y, z, l);
        self.java_neighbor_physics(x + 1, y, z, l);
        self.java_neighbor_physics(x, y - 1, z, l);
        self.java_neighbor_physics(x, y + 1, z, l);
        self.java_neighbor_physics(x, y, z - 1, l);
        self.java_neighbor_physics(x, y, z + 1, l);
    }

    /// Java `World.k(i, j, k, l)` — dispatch `Block.doPhysics` to one neighbor.
    fn java_neighbor_physics(&mut self, x: i32, y: i32, z: i32, l: i32) {
        if self.suppress_physics {
            return;
        }
        let id = self.get_block(x, y, z);
        trace_phys!(x, y, z, l, id as i32);
        match id {
            blocks::WATER | blocks::STATIONARY_WATER | blocks::LAVA | blocks::STATIONARY_LAVA => {
                // BlockFluids.doPhysics -> fizz (lava only).
                self.fluid_fizz(x, y, z, id);
                if id == blocks::STATIONARY_WATER || id == blocks::STATIONARY_LAVA {
                    // BlockStationary.doPhysics tail: convert to flowing (raw).
                    if self.get_block(x, y, z) == id {
                        self.stationary_to_flowing(x, y, z, id);
                    }
                }
            }
            blocks::SAPLING
            | blocks::TALL_GRASS
            | blocks::DEAD_BUSH
            | blocks::YELLOW_FLOWER
            | blocks::RED_ROSE
            | blocks::BROWN_MUSHROOM
            | blocks::RED_MUSHROOM => {
                // BlockFlower.doPhysics -> destroy when the plant can't survive.
                if !self.plant_can_survive(id, x, y, z) {
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            blocks::CACTUS => {
                if !self.cactus_can_survive(x, y, z) {
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            blocks::REED => {
                if !self.reed_can_survive(x, y, z) {
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            // BlockSand/BlockGravel schedule a tick (instant only while
            // `insta_tick` is set; otherwise the tick is queued and never runs).
            blocks::SAND | blocks::GRAVEL => self.java_schedule_tick(x, y, z, id),
            _ => {}
        }
    }

    /// Java `World.c(i, j, k, l, i1)`: schedule a block tick. During
    /// generation the queued branch never executes; while `insta_tick`
    /// (Java `World.a`) is set the tick runs immediately and recursively.
    fn java_schedule_tick(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if !self.insta_tick {
            return;
        }
        // `World.a(i - 8, j - 8, k - 8, i + 8, j + 8, k + 8)` guard.
        if !self.chunks_area_loaded(x - 8, y - 8, z - 8, x + 8, y + 8, z + 8) {
            return;
        }
        let cur = self.get_block(x, y, z);
        if cur == id && cur > 0 {
            self.java_dispatch_tick(x, y, z, cur);
        }
    }

    /// Dispatch a scheduled tick to the current block, mirroring
    /// `Block.byId[id].a(world, x, y, z, world.random)`.
    fn java_dispatch_tick(&mut self, x: i32, y: i32, z: i32, id: u8) {
        match id {
            blocks::WATER | blocks::LAVA => {
                let lava = id == blocks::LAVA;
                let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
                self.java_fluid_tick(x, y, z, &mut rng, lava);
                self.world_random = rng;
            }
            blocks::SAND | blocks::GRAVEL => self.java_sand_fall(x, y, z, id),
            _ => {}
        }
    }

    /// `BlockSand.c_`: can this block fall through the block below?
    fn sand_can_fall_through(&self, x: i32, y: i32, z: i32) -> bool {
        let id = self.get_block(x, y, z);
        if id == blocks::AIR || id == blocks::FIRE {
            return true;
        }
        blocks::is_water(id) || blocks::is_lava(id)
    }

    /// `BlockSand.g` with `BlockSand.instaFall == true` (set for the whole
    /// populate): the sand/gravel teleports down without spawning an entity.
    fn java_sand_fall(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if !self.sand_can_fall_through(x, y - 1, z) || y < 0 {
            return;
        }
        self.java_set_type_id(x, y, z, 0);
        let mut j = y;
        while self.sand_can_fall_through(x, j - 1, z) && j > 0 {
            j -= 1;
        }
        if j > 0 {
            self.java_set_type_id(x, j, z, id);
        }
    }

    /// BlockFluids fizz: lava touching water turns into obsidian/cobblestone.
    fn fluid_fizz(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if self.get_block(x, y, z) != id {
            return;
        }
        if !blocks::is_lava(id) {
            return;
        }
        let touches_water = self.is_water_at(x, y, z - 1)
            || self.is_water_at(x, y, z + 1)
            || self.is_water_at(x - 1, y, z)
            || self.is_water_at(x + 1, y, z)
            || self.is_water_at(x, y + 1, z);
        if !touches_water {
            return;
        }
        let data = self.get_data(x, y, z);
        if data == 0 {
            self.java_set_type_id(x, y, z, blocks::OBSIDIAN);
        } else if data <= 4 {
            self.java_set_type_id(x, y, z, blocks::COBBLESTONE);
        }
        // BlockFluids.h plays the fizz sound, consuming `world.random` twice.
        trace_rng2!(x, y, z);
    }

    /// BlockStationary: raw conversion to the flowing counterpart.
    fn stationary_to_flowing(&mut self, x: i32, y: i32, z: i32, id: u8) {
        let data = self.get_data(x, y, z);
        trace_conv!(x, y, z, data, id - 1);
        self.suppress_physics = true;
        // The raw write runs `Block.byId[id - 1].c` (fizz + schedule inside
        // BlockFlowing.c), then `BlockStationary.i` schedules again explicitly.
        self.java_set_raw_type_id_and_data(x, y, z, id - 1, data);
        self.java_schedule_tick(x, y, z, id - 1);
        self.suppress_physics = false;
    }

    fn is_water_at(&self, x: i32, y: i32, z: i32) -> bool {
        blocks::is_water(self.get_block(x, y, z))
    }

    fn is_same_fluid_at(&self, x: i32, y: i32, z: i32, lava: bool) -> bool {
        let id = self.get_block(x, y, z);
        if lava {
            blocks::is_lava(id)
        } else {
            blocks::is_water(id)
        }
    }

    /// Java `BlockFlower.f` / BlockMushroom.f / BlockDeadBush.f.
    fn plant_can_survive(&self, id: u8, x: i32, y: i32, z: i32) -> bool {
        let below = self.get_block(x, y - 1, z);
        match id {
            blocks::BROWN_MUSHROOM | blocks::RED_MUSHROOM => {
                (0..CHUNK_HEIGHT as i32).contains(&y)
                    && self.light_at(x, y, z) < 13
                    && blocks::is_opaque_cube(below)
            }
            blocks::DEAD_BUSH => {
                (self.light_at(x, y, z) >= 8 || self.can_see_sky(x, y, z)) && below == blocks::SAND
            }
            _ => {
                (self.light_at(x, y, z) >= 8 || self.can_see_sky(x, y, z))
                    && matches!(below, blocks::GRASS | blocks::DIRT | blocks::FARMLAND)
            }
        }
    }

    /// Java `BlockCactus.f`.
    fn cactus_can_survive(&self, x: i32, y: i32, z: i32) -> bool {
        if blocks::material_is_buildable(self.get_block(x - 1, y, z))
            || blocks::material_is_buildable(self.get_block(x + 1, y, z))
            || blocks::material_is_buildable(self.get_block(x, y, z - 1))
            || blocks::material_is_buildable(self.get_block(x, y, z + 1))
        {
            return false;
        }
        matches!(self.get_block(x, y - 1, z), blocks::CACTUS | blocks::SAND)
    }

    /// Java `BlockReed.f` -> `canPlace`.
    fn reed_can_survive(&self, x: i32, y: i32, z: i32) -> bool {
        let below = self.get_block(x, y - 1, z);
        if below == blocks::REED {
            return true;
        }
        if below != blocks::GRASS && below != blocks::DIRT {
            return false;
        }
        self.is_water_at(x - 1, y - 1, z)
            || self.is_water_at(x + 1, y - 1, z)
            || self.is_water_at(x, y - 1, z - 1)
            || self.is_water_at(x, y - 1, z + 1)
    }

    /// `world.k(i, j, k)` — max(sky, block) light with y clamped to [0, 127].
    fn light_at(&self, x: i32, y: i32, z: i32) -> i32 {
        if y < 0 {
            return 0;
        }
        let y = y.min(CHUNK_HEIGHT as i32 - 1);
        let pos = ChunkPos::new(x >> 4, z >> 4);
        match self.chunks.get(&pos) {
            Some(chunk) => {
                chunk.get_block_light_value((x & 15) as usize, y as usize, (z & 15) as usize, 0)
            }
            None => 15,
        }
    }

    /// `world.isChunkLoaded(i, j, k)` as used by `BlockFlower.f` — it is really
    /// `Chunk.c(i, j, k)`: the block is at or above the height map.
    fn can_see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        match self.chunks.get(&pos) {
            Some(chunk) => y >= chunk.get_height((x & 15) as usize, (z & 15) as usize) as i32,
            None => true,
        }
    }

    /// Java `Chunk.g(i, j, k)`: recompute one height-map column and write the
    /// new sky-light values directly into the light array.
    fn chunk_height_update(&mut self, cx: i32, cz: i32, lx: usize, j: i32, lz: usize) {
        let pos = ChunkPos::new(cx, cz);
        let Some(chunk) = self.chunks.get_mut(&pos) else {
            return;
        };
        let l = chunk.height_map[(lz << 4) | lx] as i32;
        let mut i1 = l;
        if j > l {
            i1 = j;
        }
        while i1 > 0 && blocks::light_opacity(chunk.get_block(lx, (i1 - 1) as usize, lz)) == 0 {
            i1 -= 1;
        }
        if i1 != l {
            trace_g!(cx * 16 + lx as i32, cz * 16 + lz as i32, l, i1);
            chunk.height_map[(lz << 4) | lx] = i1 as u8;
            if i1 < l {
                for i2 in i1..l {
                    chunk.set_sky_light(lx, i2 as usize, lz, 15);
                }
            } else {
                for i2 in l..i1 {
                    chunk.set_sky_light(lx, i2 as usize, lz, 0);
                }
            }
            let mut i2 = 15i32;
            let mut y1 = i1;
            while y1 > 0 && i2 > 0 {
                y1 -= 1;
                let mut k2 = blocks::light_opacity(chunk.get_block(lx, y1 as usize, lz)) as i32;
                if k2 == 0 {
                    k2 = 1;
                }
                i2 -= k2;
                if i2 < 0 {
                    i2 = 0;
                }
                chunk.set_sky_light(lx, y1 as usize, lz, i2 as u8);
            }
            chunk.dirty = true;
        }
    }

    /// `Block.remove` dispatch for the replaced block.
    fn block_remove(&mut self, id: u8, x: i32, y: i32, z: i32) {
        match id {
            blocks::LEAVES => self.mark_leaves_around(x, y, z, 1, false),
            blocks::LOG => self.mark_leaves_around(x, y, z, 4, true),
            _ => {}
        }
    }

    /// BlockLeaves.remove / BlockLog.remove: mark nearby leaves with data bit 8.
    fn mark_leaves_around(&mut self, x: i32, y: i32, z: i32, radius: i32, skip_marked: bool) {
        let check = radius + 1;
        if !self.chunks_area_loaded(
            x - check,
            y - check,
            z - check,
            x + check,
            y + check,
            z + check,
        ) {
            return;
        }
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let (ix, iy, iz) = (x + dx, y + dy, z + dz);
                    if self.get_block(ix, iy, iz) != blocks::LEAVES {
                        continue;
                    }
                    let data = self.get_data(ix, iy, iz);
                    if skip_marked && (data & 8) != 0 {
                        continue;
                    }
                    self.java_set_raw_data(ix, iy, iz, data | 8);
                }
            }
        }
    }

    /// Java `World.a(i, j, k, l, i1, j1)`: are all chunks in the box loaded?
    fn chunks_area_loaded(&self, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> bool {
        if y1 < 0 || y0 >= CHUNK_HEIGHT as i32 {
            return false;
        }
        for cx in (x0 >> 4)..=(x1 >> 4) {
            for cz in (z0 >> 4)..=(z1 >> 4) {
                if !self.chunks.contains_key(&ChunkPos::new(cx, cz)) {
                    return false;
                }
            }
        }
        true
    }

    /// `Block.c` on-place dispatch for the freshly written block.
    fn block_on_place(&mut self, id: u8, x: i32, y: i32, z: i32) {
        match id {
            blocks::WATER | blocks::LAVA => {
                // BlockFlowing.c: super.c (fizz) then schedule a tick when the
                // block is still flowing.
                self.fluid_fizz(x, y, z, id);
                if self.get_block(x, y, z) == id {
                    self.java_schedule_tick(x, y, z, id);
                }
            }
            blocks::STATIONARY_WATER | blocks::STATIONARY_LAVA => {
                // BlockFluids.c -> fizz.
                self.fluid_fizz(x, y, z, id);
            }
            blocks::SAND | blocks::GRAVEL => {
                // BlockSand.c: schedule a fall check.
                self.java_schedule_tick(x, y, z, id);
            }
            _ => {}
        }
    }

    // ----- BlockFlowing helpers (fluids spread only when ticked directly) -----

    /// BlockFluids.g: the fluid data at a position, or -1 when another material.
    fn fluid_data_at(&self, x: i32, y: i32, z: i32, lava: bool) -> i32 {
        if self.is_same_fluid_at(x, y, z, lava) {
            self.get_data(x, y, z) as i32
        } else {
            -1
        }
    }

    /// BlockFlowing.k: does a solid-ish block stop the flow here?
    fn flowing_blocked(&self, x: i32, y: i32, z: i32) -> bool {
        let l = self.get_block(x, y, z);
        if matches!(
            l,
            blocks::WOODEN_DOOR | blocks::IRON_DOOR | blocks::SIGN | blocks::LADDER | blocks::REED
        ) {
            return true;
        }
        if l == 0 {
            return false;
        }
        blocks::material_is_solid(l)
    }

    /// BlockFlowing.l: can the fluid spread into this position?
    fn flowing_can_flow_into(&self, x: i32, y: i32, z: i32, lava: bool) -> bool {
        if self.is_same_fluid_at(x, y, z, lava) {
            return false;
        }
        if blocks::is_lava(self.get_block(x, y, z)) {
            return false;
        }
        !self.flowing_blocked(x, y, z)
    }

    /// BlockFlowing.b: distance to a fully spread level in one direction.
    fn flowing_distance(&self, x: i32, y: i32, z: i32, l: i32, avoid: i32, lava: bool) -> i32 {
        let mut best = 1000;
        for k1 in 0..4 {
            if (k1 == 0 && avoid == 1)
                || (k1 == 1 && avoid == 0)
                || (k1 == 2 && avoid == 3)
                || (k1 == 3 && avoid == 2)
            {
                continue;
            }
            let mut l1 = x;
            let mut i2 = z;
            if k1 == 0 {
                l1 = x - 1;
            }
            if k1 == 1 {
                l1 += 1;
            }
            if k1 == 2 {
                i2 = z - 1;
            }
            if k1 == 3 {
                i2 += 1;
            }
            if self.flowing_blocked(l1, y, i2)
                || (self.is_same_fluid_at(l1, y, i2, lava) && self.get_data(l1, y, i2) == 0)
            {
                continue;
            }
            if !self.flowing_blocked(l1, y - 1, i2) {
                return l;
            }
            if l < 4 {
                let j2 = self.flowing_distance(l1, y, i2, l + 1, k1, lava);
                if j2 < best {
                    best = j2;
                }
            }
        }
        best
    }

    /// BlockFlowing.j: choose the directions the fluid spreads to. Stores the
    /// result in the shared `fluid_sides` field (Java `this.b`) exactly like
    /// the reference singleton — nested ticks overwrite it mid-loop.
    fn flowing_sides(&mut self, x: i32, y: i32, z: i32, lava: bool) {
        let li = if lava { 1 } else { 0 };
        let mut c = [1000i32; 4];
        for l in 0..4usize {
            let mut i1 = x;
            let mut j1 = z;
            if l == 0 {
                i1 = x - 1;
            }
            if l == 1 {
                i1 += 1;
            }
            if l == 2 {
                j1 = z - 1;
            }
            if l == 3 {
                j1 += 1;
            }
            if !self.flowing_blocked(i1, y, j1)
                && (!self.is_same_fluid_at(i1, y, j1, lava) || self.get_data(i1, y, j1) != 0)
            {
                if !self.flowing_blocked(i1, y - 1, j1) {
                    c[l] = 0;
                } else {
                    c[l] = self.flowing_distance(i1, y, j1, 1, l as i32, lava);
                }
            }
        }
        let mut min = c[0];
        for &value in c.iter().skip(1) {
            if value < min {
                min = value;
            }
        }
        for (i, &value) in c.iter().enumerate() {
            self.fluid_sides[li][i] = value == min;
        }
        trace_j!(x, y, z, c, self.fluid_sides[li]);
    }

    /// BlockFlowing.f: running minimum of neighboring fluid levels.
    fn flowing_min_side(
        &self,
        x: i32,
        y: i32,
        z: i32,
        l: i32,
        lava: bool,
        counter: &mut i32,
    ) -> i32 {
        let i1 = self.fluid_data_at(x, y, z, lava);
        if i1 < 0 {
            return l;
        }
        if i1 == 0 {
            *counter += 1;
        }
        let i1 = if i1 >= 8 { 0 } else { i1 };
        if l >= 0 && i1 >= l {
            l
        } else {
            i1
        }
    }

    /// BlockFlowing.flow: actually spread the fluid into a position.
    fn fluid_flow(&mut self, x: i32, y: i32, z: i32, level: i32, lava: bool) {
        trace_flow!(x, y, z, level);
        if !self.flowing_can_flow_into(x, y, z, lava) {
            return;
        }
        let id = if lava { blocks::LAVA } else { blocks::WATER };
        let target = self.get_block(x, y, z);
        if target > 0 {
            if lava {
                // BlockFluids.h fizz sound (world.random x2).
                trace_rng2!(x, y, z);
            } else {
                // Block.g -> dropNaturally (world.random; drops are entities).
                trace_rng3!(x, y, z);
            }
        }
        self.java_set_type_id_and_data(x, y, z, id, level as u8);
    }

    /// `BlockFlowing.a(World, i, j, k, Random)` — invoked directly by
    /// WorldGenLiquids after the fluid is placed (scheduled ticks never run
    /// during generation).
    pub fn java_fluid_tick(&mut self, x: i32, y: i32, z: i32, random: &mut JavaRandom, lava: bool) {
        let this_id = if lava { blocks::LAVA } else { blocks::WATER };
        let b0: i32 = if lava { 2 } else { 1 };
        let mut l = self.fluid_data_at(x, y, z, lava);
        trace_fa!(x, y, z, this_id as i32, l);
        let mut flag = true;
        let mut i1;

        if l > 0 {
            let mut counter = 0i32;
            let mut j1 = self.flowing_min_side(x - 1, y, z, -100, lava, &mut counter);
            j1 = self.flowing_min_side(x + 1, y, z, j1, lava, &mut counter);
            j1 = self.flowing_min_side(x, y, z - 1, j1, lava, &mut counter);
            j1 = self.flowing_min_side(x, y, z + 1, j1, lava, &mut counter);
            i1 = j1 + b0;
            if i1 >= 8 || j1 < 0 {
                i1 = -1;
            }
            let above = self.fluid_data_at(x, y + 1, z, lava);
            if above >= 0 {
                i1 = if above >= 8 { above } else { above + 8 };
            }
            if counter >= 2 && !lava {
                if blocks::material_is_buildable(self.get_block(x, y - 1, z)) {
                    i1 = 0;
                } else if self.is_same_fluid_at(x, y - 1, z, lava) && self.get_data(x, y, z) == 0 {
                    i1 = 0;
                }
            }
            if lava && l < 8 && i1 < 8 && i1 > l {
                trace_rng!(x, y, z);
            }
            if lava && l < 8 && i1 < 8 && i1 > l && random.next_i32_bound(4) != 0 {
                // Poseidon flowing-lava fix is enabled by default: keep `i1`,
                // but the RNG draw still happens and `flag` still clears.
                flag = false;
            }
            if i1 != l {
                l = i1;
                if i1 < 0 {
                    self.java_set_type_id(x, y, z, 0);
                } else {
                    self.java_set_data(x, y, z, i1 as u8);
                    self.java_schedule_tick(x, y, z, this_id);
                    self.java_apply_physics(x, y, z, this_id as i32);
                }
            } else if flag {
                let data = self.get_data(x, y, z);
                trace_conv!(x, y, z, data, this_id + 1);
                self.java_set_raw_type_id_and_data(x, y, z, this_id + 1, data);
            }
        } else {
            let data = self.get_data(x, y, z);
            trace_conv!(x, y, z, data, this_id + 1);
            self.java_set_raw_type_id_and_data(x, y, z, this_id + 1, data);
        }

        if self.flowing_can_flow_into(x, y - 1, z, lava) {
            trace_down!(x, y - 1, z);
            if l >= 8 {
                self.java_set_type_id_and_data(x, y - 1, z, this_id, l as u8);
            } else {
                self.java_set_type_id_and_data(x, y - 1, z, this_id, (l + 8) as u8);
            }
        } else if l >= 0 && (l == 0 || self.flowing_blocked(x, y - 1, z)) {
            self.flowing_sides(x, y, z, lava);
            i1 = l + b0;
            if l >= 8 {
                i1 = 1;
            }
            if i1 >= 8 {
                return;
            }
            // CraftBukkit maps the movement-flag indices to BlockFace in the
            // order NORTH(x-1), SOUTH(x+1), EAST(z-1), WEST(z+1).
            let li = if lava { 1 } else { 0 };
            let offsets = [(-1, 0), (1, 0), (0, -1), (0, 1)];
            for (idx, &(dx, dz)) in offsets.iter().enumerate() {
                // Read the flag fresh: nested ticks may have overwritten the
                // shared BlockFlowing.b array (Java aliasing quirk).
                if self.fluid_sides[li][idx] {
                    self.fluid_flow(x + dx, y, z + dz, i1, lava);
                }
            }
        }
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.java_set_raw_type_id(x, y, z, id);
    }

    pub fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.java_set_raw_type_id_and_data(x, y, z, id, data);
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

    fn set_block_physics(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.java_set_type_id(x, y, z, id);
    }

    fn set_block_and_data_physics(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.java_set_type_id_and_data(x, y, z, id, data);
    }

    fn fluid_tick(&mut self, x: i32, y: i32, z: i32, lava: bool, random: &mut JavaRandom) {
        // WorldGenLiquids: `world.a = true` around the direct tick, which makes
        // every tick scheduled by the cascade below run immediately.
        self.insta_tick = true;
        self.java_fluid_tick(x, y, z, random, lava);
        self.insta_tick = false;
    }

    fn set_tile_entity(&mut self, te: crate::gameplay::tile::TileEntity) {
        let pos = ChunkPos::new(te.x >> 4, te.z >> 4);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.set_tile_entity(te);
        }
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

    fn biome_at(&self, x: i32, z: i32) -> Biome {
        self.chunk_mgr.borrow_mut().biome_at(x, z)
    }
}

impl LightingWorld for World {
    fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        self.get_block(x, y, z)
    }

    fn get_light(&self, light_type: LightType, x: i32, y: i32, z: i32) -> u8 {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return if light_type == LightType::Sky { 15 } else { 0 };
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            match light_type {
                LightType::Sky => {
                    chunk.get_sky_light((x & 15) as usize, y as usize, (z & 15) as usize)
                }
                LightType::Block => {
                    chunk.get_block_light((x & 15) as usize, y as usize, (z & 15) as usize)
                }
            }
        } else if light_type == LightType::Sky {
            15
        } else {
            0
        }
    }

    fn set_light(&mut self, light_type: LightType, x: i32, y: i32, z: i32, val: u8) {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            match light_type {
                LightType::Sky => {
                    chunk.set_sky_light((x & 15) as usize, y as usize, (z & 15) as usize, val);
                }
                LightType::Block => {
                    chunk.set_block_light((x & 15) as usize, y as usize, (z & 15) as usize, val);
                }
            }
            chunk.dirty = true;
        }
        self.packet_cache.borrow_mut().remove(&pos);
    }

    fn can_see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        self.see_sky(x, y, z)
    }

    fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool {
        self.is_chunk_loaded(ChunkPos::new(cx, cz))
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
