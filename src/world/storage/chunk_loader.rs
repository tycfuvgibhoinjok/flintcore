use crate::gameplay::tile::TileEntity;
use crate::util::{Error, Result};
use crate::world::chunk::{Chunk, ChunkPos, ChunkState, CHUNK_HEIGHT, CHUNK_WIDTH};
use crate::world::storage::nbt::NbtTag;

pub struct ChunkLoader;

impl ChunkLoader {
    pub fn chunk_to_nbt(chunk: &Chunk, world_time: i64) -> NbtTag {
        let mut level = NbtTag::compound();
        level.insert("xPos", NbtTag::Int(chunk.pos.x));
        level.insert("zPos", NbtTag::Int(chunk.pos.z));
        level.insert("LastUpdate", NbtTag::Long(world_time));
        level.insert(
            "TerrainPopulated",
            NbtTag::Byte(if chunk.state == ChunkState::Populated {
                1
            } else {
                0
            }),
        );

        let mut blocks = vec![0u8; 32768];
        let mut data = vec![0u8; 16384];
        let mut sky_light = vec![0u8; 16384];
        let mut block_light = vec![0u8; 16384];

        for x in 0..CHUNK_WIDTH {
            for z in 0..CHUNK_WIDTH {
                for y in 0..CHUNK_HEIGHT {
                    let mcr_idx = (x << 11) | (z << 7) | y;
                    blocks[mcr_idx] = chunk.get_block(x, y, z);

                    let nibble_idx = mcr_idx >> 1;
                    let meta = chunk.get_metadata(x, y, z) & 15;
                    let sky = chunk.get_sky_light(x, y, z) & 15;
                    let blk = chunk.get_block_light(x, y, z) & 15;

                    if y & 1 == 0 {
                        data[nibble_idx] = (data[nibble_idx] & 0xF0) | meta;
                        sky_light[nibble_idx] = (sky_light[nibble_idx] & 0xF0) | sky;
                        block_light[nibble_idx] = (block_light[nibble_idx] & 0xF0) | blk;
                    } else {
                        data[nibble_idx] = (data[nibble_idx] & 0x0F) | (meta << 4);
                        sky_light[nibble_idx] = (sky_light[nibble_idx] & 0x0F) | (sky << 4);
                        block_light[nibble_idx] = (block_light[nibble_idx] & 0x0F) | (blk << 4);
                    }
                }
            }
        }

        level.insert("Blocks", NbtTag::ByteArray(blocks));
        level.insert("Data", NbtTag::ByteArray(data));
        level.insert("SkyLight", NbtTag::ByteArray(sky_light));
        level.insert("BlockLight", NbtTag::ByteArray(block_light));
        level.insert("HeightMap", NbtTag::ByteArray(chunk.height_map.to_vec()));
        level.insert("Entities", NbtTag::List(Vec::new()));

        let mut tile_entities = Vec::new();
        for te in &chunk.tile_entities {
            tile_entities.push(te.to_nbt());
        }
        level.insert("TileEntities", NbtTag::List(tile_entities));

        let mut root = NbtTag::compound();
        root.insert("Level", level);
        root
    }

    pub fn chunk_from_nbt(root: &NbtTag) -> Result<Chunk> {
        let level = root
            .get("Level")
            .ok_or(Error::Malformed("missing 'Level' compound in chunk NBT"))?;

        let x = match level.get("xPos") {
            Some(NbtTag::Int(v)) => *v,
            _ => return Err(Error::Malformed("missing xPos in chunk")),
        };
        let z = match level.get("zPos") {
            Some(NbtTag::Int(v)) => *v,
            _ => return Err(Error::Malformed("missing zPos in chunk")),
        };

        let mut chunk = Chunk::new(ChunkPos::new(x, z));

        let populated = match level.get("TerrainPopulated") {
            Some(NbtTag::Byte(v)) => *v != 0,
            _ => false,
        };
        chunk.state = if populated {
            ChunkState::Populated
        } else {
            ChunkState::Generated
        };

        let blocks = level
            .get_byte_array("Blocks")
            .ok_or(Error::Malformed("missing Blocks in chunk"))?;
        if blocks.len() != 32768 {
            return Err(Error::Malformed("invalid Blocks length in chunk"));
        }

        let data = level
            .get_byte_array("Data")
            .ok_or(Error::Malformed("missing Data in chunk"))?;
        let sky_light = level
            .get_byte_array("SkyLight")
            .ok_or(Error::Malformed("missing SkyLight in chunk"))?;
        let block_light = level
            .get_byte_array("BlockLight")
            .ok_or(Error::Malformed("missing BlockLight in chunk"))?;

        for x in 0..CHUNK_WIDTH {
            for z in 0..CHUNK_WIDTH {
                for y in 0..CHUNK_HEIGHT {
                    let mcr_idx = (x << 11) | (z << 7) | y;
                    chunk.set_block(x, y, z, blocks[mcr_idx]);

                    let nibble_idx = mcr_idx >> 1;
                    if nibble_idx < data.len() {
                        let byte = data[nibble_idx];
                        let val = if y & 1 == 0 { byte & 15 } else { byte >> 4 };
                        chunk.set_metadata(x, y, z, val);
                    }
                    if nibble_idx < sky_light.len() {
                        let byte = sky_light[nibble_idx];
                        let val = if y & 1 == 0 { byte & 15 } else { byte >> 4 };
                        chunk.set_sky_light(x, y, z, val);
                    }
                    if nibble_idx < block_light.len() {
                        let byte = block_light[nibble_idx];
                        let val = if y & 1 == 0 { byte & 15 } else { byte >> 4 };
                        chunk.set_block_light(x, y, z, val);
                    }
                }
            }
        }

        if let Some(height_map) = level.get_byte_array("HeightMap") {
            if height_map.len() == 256 {
                chunk.height_map.copy_from_slice(height_map);
            } else {
                chunk.init_lighting();
            }
        } else {
            chunk.init_lighting();
        }

        if let Some(tes) = level.get_list("TileEntities") {
            for te_tag in tes {
                if let Some(te) = TileEntity::from_nbt(te_tag) {
                    chunk.tile_entities.push(te);
                }
            }
        }

        chunk.dirty = false;
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_nbt_round_trip() {
        let mut chunk = Chunk::new(ChunkPos::new(-5, 12));
        chunk.state = ChunkState::Populated;
        chunk.set_block(3, 40, 7, 1); // stone
        chunk.set_metadata(3, 40, 7, 2);
        chunk.set_sky_light(3, 40, 7, 14);
        chunk.set_block_light(3, 40, 7, 10);
        chunk.tile_entities.push(TileEntity::new_sign(
            -5 * 16 + 3,
            41,
            12 * 16 + 7,
            ["Line 1".into(), "Line 2".into(), "".into(), "".into()],
        ));

        let nbt = ChunkLoader::chunk_to_nbt(&chunk, 1000);
        let restored = ChunkLoader::chunk_from_nbt(&nbt).unwrap();

        assert_eq!(restored.pos, ChunkPos::new(-5, 12));
        assert_eq!(restored.state, ChunkState::Populated);
        assert_eq!(restored.get_block(3, 40, 7), 1);
        assert_eq!(restored.get_metadata(3, 40, 7), 2);
        assert_eq!(restored.get_sky_light(3, 40, 7), 14);
        assert_eq!(restored.get_block_light(3, 40, 7), 10);
        assert_eq!(restored.tile_entities.len(), 1);
    }
}
