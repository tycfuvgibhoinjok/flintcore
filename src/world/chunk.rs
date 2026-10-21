use crate::gameplay::blocks;
use crate::gameplay::tile::TileEntity;

pub const CHUNK_WIDTH: usize = 16;
pub const CHUNK_HEIGHT: usize = 128;
pub const CHUNK_AREA: usize = CHUNK_WIDTH * CHUNK_WIDTH;
pub const CHUNK_VOLUME: usize = CHUNK_WIDTH * CHUNK_HEIGHT * CHUNK_WIDTH;
pub const SUB_CHUNK_SIZE: usize = 16;
pub const SUB_CHUNK_VOLUME: usize = SUB_CHUNK_SIZE * SUB_CHUNK_SIZE * SUB_CHUNK_SIZE;
pub const SUB_CHUNK_COUNT: usize = CHUNK_HEIGHT / SUB_CHUNK_SIZE;
pub const WATER_LEVEL: i32 = 64;
pub const PAYLOAD_LEN: usize = CHUNK_VOLUME + 3 * (CHUNK_VOLUME / 2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkState {
    Generating,
    Generated,
    Populating,
    Populated,
}

fn local_index(x: usize, y: usize, z: usize) -> usize {
    ((y & 15) * 256) + (z * 16) + x
}

fn wire_index(x: usize, y: usize, z: usize) -> usize {
    (x << 11) | (z << 7) | y
}

struct BlockLayer {
    fill: u8,
    data: Option<Box<[u8; SUB_CHUNK_VOLUME]>>,
}

impl BlockLayer {
    fn new(fill: u8) -> Self {
        Self { fill, data: None }
    }

    fn get(&self, index: usize) -> u8 {
        match &self.data {
            Some(data) => data[index],
            None => self.fill,
        }
    }

    fn set(&mut self, index: usize, value: u8) {
        match &mut self.data {
            Some(data) => data[index] = value,
            None => {
                if value == self.fill {
                    return;
                }
                let mut data = Box::new([self.fill; SUB_CHUNK_VOLUME]);
                data[index] = value;
                self.data = Some(data);
            }
        }
    }

    fn compact(&mut self) {
        if let Some(data) = &self.data {
            let first = data[0];
            if data.iter().all(|&value| value == first) {
                self.fill = first;
                self.data = None;
            }
        }
    }
}

struct NibbleLayer {
    fill: u8,
    data: Option<Box<[u8; SUB_CHUNK_VOLUME / 2]>>,
}

impl NibbleLayer {
    fn new(fill: u8) -> Self {
        Self { fill, data: None }
    }

    fn get(&self, index: usize) -> u8 {
        match &self.data {
            Some(data) => {
                let byte = data[index >> 1];
                if index & 1 == 0 {
                    byte & 15
                } else {
                    byte >> 4
                }
            }
            None => self.fill,
        }
    }

    fn set(&mut self, index: usize, value: u8) {
        let value = value & 15;
        match &mut self.data {
            Some(data) => {
                let byte = &mut data[index >> 1];
                if index & 1 == 0 {
                    *byte = (*byte & 240) | value;
                } else {
                    *byte = (*byte & 15) | (value << 4);
                }
            }
            None => {
                if value == self.fill {
                    return;
                }
                let mut data = Box::new([(self.fill << 4) | self.fill; SUB_CHUNK_VOLUME / 2]);
                let byte = &mut data[index >> 1];
                if index & 1 == 0 {
                    *byte = (*byte & 240) | value;
                } else {
                    *byte = (*byte & 15) | (value << 4);
                }
                self.data = Some(data);
            }
        }
    }

    fn compact(&mut self) {
        if let Some(data) = &self.data {
            let first = data[0] & 15;
            let uniform = data
                .iter()
                .all(|&byte| (byte & 15) == first && (byte >> 4) == first);
            if uniform {
                self.fill = first;
                self.data = None;
            }
        }
    }
}

struct SubChunk {
    blocks: BlockLayer,
    metadata: NibbleLayer,
    block_light: NibbleLayer,
    sky_light: NibbleLayer,
}

impl SubChunk {
    fn new() -> Self {
        Self {
            blocks: BlockLayer::new(blocks::AIR),
            metadata: NibbleLayer::new(0),
            block_light: NibbleLayer::new(0),
            sky_light: NibbleLayer::new(0),
        }
    }
}

pub struct Chunk {
    pub pos: ChunkPos,
    pub state: ChunkState,
    pub dirty: bool,
    pub height_map: [u8; CHUNK_AREA],
    pub biomes: [u8; CHUNK_AREA],
    pub tile_entities: Vec<TileEntity>,
    sub_chunks: [SubChunk; SUB_CHUNK_COUNT],
}

impl Chunk {
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            state: ChunkState::Generating,
            dirty: false,
            height_map: [0; CHUNK_AREA],
            biomes: [0; CHUNK_AREA],
            tile_entities: Vec::new(),
            sub_chunks: std::array::from_fn(|_| SubChunk::new()),
        }
    }

    pub fn get_tile_entity(&self, x: i32, y: i32, z: i32) -> Option<&TileEntity> {
        self.tile_entities
            .iter()
            .find(|te| te.x == x && te.y == y && te.z == z)
    }

    pub fn set_tile_entity(&mut self, te: TileEntity) {
        self.remove_tile_entity(te.x, te.y, te.z);
        self.tile_entities.push(te);
        self.dirty = true;
    }

    pub fn remove_tile_entity(&mut self, x: i32, y: i32, z: i32) {
        if let Some(idx) = self
            .tile_entities
            .iter()
            .position(|te| te.x == x && te.y == y && te.z == z)
        {
            self.tile_entities.swap_remove(idx);
            self.dirty = true;
        }
    }

    pub fn get_block(&self, x: usize, y: usize, z: usize) -> u8 {
        if y >= CHUNK_HEIGHT {
            return blocks::AIR;
        }
        self.sub_chunks[y >> 4].blocks.get(local_index(x, y, z))
    }

    pub fn set_block(&mut self, x: usize, y: usize, z: usize, id: u8) {
        if y >= CHUNK_HEIGHT {
            return;
        }
        self.sub_chunks[y >> 4].blocks.set(local_index(x, y, z), id);
    }

    pub fn get_metadata(&self, x: usize, y: usize, z: usize) -> u8 {
        if y >= CHUNK_HEIGHT {
            return 0;
        }
        self.sub_chunks[y >> 4].metadata.get(local_index(x, y, z))
    }

    pub fn set_metadata(&mut self, x: usize, y: usize, z: usize, value: u8) {
        if y >= CHUNK_HEIGHT {
            return;
        }
        self.sub_chunks[y >> 4]
            .metadata
            .set(local_index(x, y, z), value);
    }

    pub fn get_block_light(&self, x: usize, y: usize, z: usize) -> u8 {
        if y >= CHUNK_HEIGHT {
            return 0;
        }
        self.sub_chunks[y >> 4]
            .block_light
            .get(local_index(x, y, z))
    }

    pub fn set_block_light(&mut self, x: usize, y: usize, z: usize, value: u8) {
        if y >= CHUNK_HEIGHT {
            return;
        }
        self.sub_chunks[y >> 4]
            .block_light
            .set(local_index(x, y, z), value);
    }

    pub fn get_sky_light(&self, x: usize, y: usize, z: usize) -> u8 {
        if y >= CHUNK_HEIGHT {
            return 15;
        }
        self.sub_chunks[y >> 4].sky_light.get(local_index(x, y, z))
    }

    pub fn set_sky_light(&mut self, x: usize, y: usize, z: usize, value: u8) {
        if y >= CHUNK_HEIGHT {
            return;
        }
        self.sub_chunks[y >> 4]
            .sky_light
            .set(local_index(x, y, z), value);
    }

    pub fn get_height(&self, x: usize, z: usize) -> u8 {
        self.height_map[(z << 4) | x]
    }

    pub fn can_block_see_sky(&self, x: usize, y: usize, z: usize) -> bool {
        y >= self.get_height(x, z) as usize
    }

    pub fn get_block_light_value(&self, x: usize, y: usize, z: usize, sky_subtracted: i32) -> i32 {
        let mut light = self.get_sky_light(x, y, z) as i32 - sky_subtracted;
        let block_light = self.get_block_light(x, y, z) as i32;
        if block_light > light {
            light = block_light;
        }
        light
    }

    pub fn init_lighting(&mut self) {
        crate::world::lighting::init_chunk_lighting(self);
    }

    pub fn set_subchunk_sky_light_fill(&mut self, sub_idx: usize, fill: u8) {
        if sub_idx < SUB_CHUNK_COUNT {
            self.sub_chunks[sub_idx].sky_light.fill = fill & 15;
            self.sub_chunks[sub_idx].sky_light.data = None;
        }
    }

    pub fn compact(&mut self) {
        self.tile_entities.shrink_to_fit();
        for section in &mut self.sub_chunks {
            section.blocks.compact();
            section.metadata.compact();
            section.block_light.compact();
            section.sky_light.compact();
        }
    }

    pub fn write_nibble_layer(&self, kind: u8, out_slice: &mut [u8]) {
        for (sub, section) in self.sub_chunks.iter().enumerate() {
            let layer = match kind {
                0 => &section.metadata,
                1 => &section.block_light,
                _ => &section.sky_light,
            };
            let nibble_y_base = sub * 8;
            match &layer.data {
                None => {
                    let f = layer.fill & 15;
                    if f != 0 {
                        let packed = (f << 4) | f;
                        for x in 0..CHUNK_WIDTH {
                            for z in 0..CHUNK_WIDTH {
                                let col_base = (((x << 11) | (z << 7)) >> 1) + nibble_y_base;
                                out_slice[col_base..col_base + 8].fill(packed);
                            }
                        }
                    }
                }
                Some(_) => {
                    for x in 0..CHUNK_WIDTH {
                        for z in 0..CHUNK_WIDTH {
                            let col_base = (((x << 11) | (z << 7)) >> 1) + nibble_y_base;
                            let zx = (z << 4) | x;
                            for pair in 0..8 {
                                let ly0 = pair * 2;
                                let v0 = layer.get((ly0 << 8) | zx) & 15;
                                let v1 = layer.get(((ly0 + 1) << 8) | zx) & 15;
                                out_slice[col_base + pair] = v0 | (v1 << 4);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn write_blocks_layer(&self, out_slice: &mut [u8]) {
        for (sub, section) in self.sub_chunks.iter().enumerate() {
            let y_base = sub * 16;
            match &section.blocks.data {
                None => {
                    let f = section.blocks.fill;
                    if f != 0 {
                        for x in 0..CHUNK_WIDTH {
                            for z in 0..CHUNK_WIDTH {
                                let col_base = (x << 11) | (z << 7) | y_base;
                                out_slice[col_base..col_base + 16].fill(f);
                            }
                        }
                    }
                }
                Some(data) => {
                    for x in 0..CHUNK_WIDTH {
                        for z in 0..CHUNK_WIDTH {
                            let col_base = (x << 11) | (z << 7) | y_base;
                            let zx = (z << 4) | x;
                            for ly in 0..16 {
                                out_slice[col_base + ly] = data[(ly << 8) | zx];
                            }
                        }
                    }
                }
            }
        }
    }

    /// Java `Chunk.getData(byte[], int, int, int, int, int, int, int)`:
    /// the data of the region of the chunk.
    ///
    /// The blocks come first, then the metadata, the light of the blocks and
    /// the light of the sky — every one of them as the bytes of the nibbles.
    #[allow(clippy::too_many_arguments)]
    pub fn write_region_payload(
        &self,
        x1: usize,
        y1: usize,
        z1: usize,
        x2: usize,
        y2: usize,
        z2: usize,
        out: &mut Vec<u8>,
    ) {
        // The blocks.
        for x in x1..x2 {
            for z in z1..z2 {
                for y in y1..y2 {
                    out.push(self.get_block(x, y, z));
                }
            }
        }
        // The metadata, the light of the blocks and the light of the sky.
        for layer in 0..3u8 {
            for x in x1..x2 {
                for z in z1..z2 {
                    let mut y = y1 & !1;
                    while y < y2 {
                        let low = self.nibble_value(layer, x, y, z);
                        let high = if y + 1 < y2 {
                            self.nibble_value(layer, x, y + 1, z)
                        } else {
                            0
                        };
                        out.push(low | (high << 4));
                        y += 2;
                    }
                }
            }
        }
    }

    /// The value of one of the layers of the nibbles.
    fn nibble_value(&self, layer: u8, x: usize, y: usize, z: usize) -> u8 {
        match layer {
            0 => self.get_metadata(x, y, z),
            1 => self.get_block_light(x, y, z),
            _ => self.get_sky_light(x, y, z),
        }
    }

    pub fn write_payload(&self, out: &mut Vec<u8>) {
        out.clear();
        out.resize(PAYLOAD_LEN, 0);
        let _ = wire_index(0, 0, 0);

        self.write_blocks_layer(&mut out[0..CHUNK_VOLUME]);
        let half = CHUNK_VOLUME / 2;
        self.write_nibble_layer(0, &mut out[CHUNK_VOLUME..CHUNK_VOLUME + half]);
        self.write_nibble_layer(1, &mut out[CHUNK_VOLUME + half..CHUNK_VOLUME + 2 * half]);
        self.write_nibble_layer(
            2,
            &mut out[CHUNK_VOLUME + 2 * half..CHUNK_VOLUME + 3 * half],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_layers_materialize_and_compact() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        assert_eq!(chunk.get_block(3, 40, 7), blocks::AIR);
        chunk.set_block(3, 40, 7, blocks::STONE);
        assert_eq!(chunk.get_block(3, 40, 7), blocks::STONE);
        assert_eq!(chunk.get_block(3, 40, 8), blocks::AIR);
        chunk.set_block(3, 40, 7, blocks::AIR);
        chunk.compact();
        assert_eq!(chunk.get_block(3, 40, 7), blocks::AIR);
    }

    /// The data of the region of the chunk.
    ///
    /// Java `Chunk.getData(byte[], ...)`: the blocks, the metadata, the
    /// light of the blocks and the light of the sky.
    #[test]
    fn the_region_of_the_chunk_holds_the_blocks_and_the_light() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        chunk.set_block(1, 64, 2, blocks::STONE);
        chunk.set_metadata(1, 64, 2, 5);
        chunk.set_block_light(1, 64, 2, 11);
        chunk.set_sky_light(1, 64, 2, 13);

        let mut payload = Vec::new();
        chunk.write_region_payload(1, 64, 2, 2, 66, 3, &mut payload);
        // The blocks of the region.
        assert_eq!(payload[0], blocks::STONE);
        assert_eq!(payload[1], blocks::AIR);
        // The metadata: the low nibble is the value of the even height.
        assert_eq!(payload[2] & 15, 5, "the metadata of the block");
        assert_eq!(payload[2] >> 4, 0, "the metadata of the block above");
        // The light of the blocks.
        assert_eq!(payload[3] & 15, 11, "the light of the block");
        // The light of the sky.
        assert_eq!(payload[4] & 15, 13, "the light of the sky");
    }

    #[test]
    fn nibble_layers_pack_low_then_high() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        chunk.set_metadata(0, 0, 0, 5);
        chunk.set_metadata(1, 0, 0, 9);
        assert_eq!(chunk.get_metadata(0, 0, 0), 5);
        assert_eq!(chunk.get_metadata(1, 0, 0), 9);
        assert_eq!(chunk.get_metadata(2, 0, 0), 0);
    }

    #[test]
    fn sky_light_defaults_to_dark_until_lit() {
        let chunk = Chunk::new(ChunkPos::new(0, 0));
        assert_eq!(chunk.get_sky_light(5, 100, 5), 0);
    }

    #[test]
    fn payload_has_fixed_length_and_order() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        chunk.set_block(0, 0, 0, blocks::STONE);
        chunk.set_sky_light(0, 0, 0, 15);
        let mut payload = Vec::new();
        chunk.write_payload(&mut payload);
        assert_eq!(payload.len(), PAYLOAD_LEN);
        assert_eq!(payload[0], blocks::STONE);
        assert_eq!(payload[1], blocks::AIR);
        assert_eq!(payload[CHUNK_VOLUME], 0);
        // index 0 is even -> low nibble of the sky light byte
        assert_eq!(
            payload[CHUNK_VOLUME + CHUNK_VOLUME / 2 + CHUNK_VOLUME / 2] & 15,
            15
        );
    }

    #[test]
    fn init_lighting_computes_height_and_skylight() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        for x in 0..CHUNK_WIDTH {
            for z in 0..CHUNK_WIDTH {
                chunk.set_block(x, 0, z, blocks::BEDROCK);
                for y in 1..64 {
                    chunk.set_block(x, y, z, blocks::STONE);
                }
                chunk.set_block(x, 64, z, blocks::GRASS);
            }
        }
        chunk.init_lighting();
        assert_eq!(chunk.get_height(0, 0), 65);
        assert_eq!(chunk.get_sky_light(0, 65, 0), 15);
        assert_eq!(chunk.get_sky_light(0, 63, 0), 0);
    }
}
