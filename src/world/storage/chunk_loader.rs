use crate::gameplay::tile::TileEntity;
use crate::util::{Error, Result};
use crate::world::chunk::{Chunk, ChunkPos, ChunkState, CHUNK_HEIGHT, CHUNK_WIDTH};
use crate::world::storage::nbt::NbtTag;

pub struct ChunkLoader;

impl ChunkLoader {
    /// Java `ChunkLoader.a(Chunk, World, NBTTagCompound)`: the entities of the
    /// chunk are written into the list `Entities`.
    pub fn chunk_to_nbt(chunk: &Chunk, world_time: i64, entities: Vec<NbtTag>) -> NbtTag {
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

        chunk.write_blocks_layer(&mut blocks);
        chunk.write_nibble_layer(0, &mut data);
        chunk.write_nibble_layer(1, &mut block_light);
        chunk.write_nibble_layer(2, &mut sky_light);

        level.insert("Blocks", NbtTag::ByteArray(blocks));
        level.insert("Data", NbtTag::ByteArray(data));
        level.insert("SkyLight", NbtTag::ByteArray(sky_light));
        level.insert("BlockLight", NbtTag::ByteArray(block_light));
        level.insert("HeightMap", NbtTag::ByteArray(chunk.height_map.to_vec()));
        level.insert("Entities", NbtTag::List(entities));

        let mut tile_entities = Vec::new();
        for te in &chunk.tile_entities {
            tile_entities.push(te.to_nbt());
        }
        level.insert("TileEntities", NbtTag::List(tile_entities));

        let mut root = NbtTag::compound();
        root.insert("Level", level);
        root
    }

    /// Java `ChunkLoader.a(World, NBTTagCompound)`: the entities of the chunk
    /// are read from the list `Entities`.
    pub fn entities_from_nbt(root: &NbtTag) -> Vec<NbtTag> {
        let Some(level) = root.get("Level") else {
            return Vec::new();
        };
        match level.get("Entities") {
            Some(NbtTag::List(list)) => list.clone(),
            _ => Vec::new(),
        }
    }

    /// Java `ChunkLoader.read`: the chunk of the NBT.
    ///
    /// The `is_hell` flag is the Nether of the world ✓ — in the Nether the
    /// sky light is never computed ✓ (`Chunk.initLighting`, `Chunk.java:91-103`
    /// ✓).
    pub fn chunk_from_nbt(root: &NbtTag, is_hell: bool) -> Result<Chunk> {
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

        // Pre-detect uniform full-sky (0xFF) subchunks to avoid allocating NibbleLayer boxes
        let mut uniform_sky_15 = [false; 8];
        if sky_light.len() >= 16384 {
            for (sub, is_full) in uniform_sky_15.iter_mut().enumerate() {
                let nibble_y_base = sub * 8;
                let mut all_ff = true;
                'check: for x in 0..CHUNK_WIDTH {
                    for z in 0..CHUNK_WIDTH {
                        let col_base = (((x << 11) | (z << 7)) >> 1) + nibble_y_base;
                        if sky_light[col_base..col_base + 8] != [0xFF; 8] {
                            all_ff = false;
                            break 'check;
                        }
                    }
                }
                if all_ff {
                    chunk.set_subchunk_sky_light_fill(sub, 15);
                    *is_full = true;
                }
            }
        }

        for x in 0..CHUNK_WIDTH {
            for z in 0..CHUNK_WIDTH {
                for y in 0..CHUNK_HEIGHT {
                    let mcr_idx = (x << 11) | (z << 7) | y;
                    let b = blocks[mcr_idx];
                    if b != 0 {
                        chunk.set_block(x, y, z, b);
                    }

                    let nibble_idx = mcr_idx >> 1;
                    if nibble_idx < data.len() {
                        let byte = data[nibble_idx];
                        if byte != 0 {
                            let val = if y & 1 == 0 { byte & 15 } else { byte >> 4 };
                            if val != 0 {
                                chunk.set_metadata(x, y, z, val);
                            }
                        }
                    }
                    if !uniform_sky_15[y >> 4] && nibble_idx < sky_light.len() {
                        let byte = sky_light[nibble_idx];
                        if byte != 0 {
                            let val = if y & 1 == 0 { byte & 15 } else { byte >> 4 };
                            if val != 0 {
                                chunk.set_sky_light(x, y, z, val);
                            }
                        }
                    }
                    if nibble_idx < block_light.len() {
                        let byte = block_light[nibble_idx];
                        if byte != 0 {
                            let val = if y & 1 == 0 { byte & 15 } else { byte >> 4 };
                            if val != 0 {
                                chunk.set_block_light(x, y, z, val);
                            }
                        }
                    }
                }
            }
        }

        if let Some(height_map) = level.get_byte_array("HeightMap") {
            if height_map.len() == 256 {
                chunk.height_map.copy_from_slice(height_map);
            } else {
                chunk.init_lighting(is_hell);
            }
        } else {
            chunk.init_lighting(is_hell);
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

        let nbt = ChunkLoader::chunk_to_nbt(&chunk, 1000, Vec::new());
        let restored = ChunkLoader::chunk_from_nbt(&nbt, false).unwrap();

        assert_eq!(restored.pos, ChunkPos::new(-5, 12));
        assert_eq!(restored.state, ChunkState::Populated);
        assert_eq!(restored.get_block(3, 40, 7), 1);
        assert_eq!(restored.get_metadata(3, 40, 7), 2);
        assert_eq!(restored.get_sky_light(3, 40, 7), 14);
        assert_eq!(restored.get_block_light(3, 40, 7), 10);
        assert_eq!(restored.tile_entities.len(), 1);
    }

    /// The chunk of the Nether has no sky light.
    ///
    /// Java `Chunk.initLighting` (`Chunk.java:91-103`): the pass of the sky
    /// light is skipped in the Nether ✓ (`worldProvider.e` ✓); the height map
    /// is computed anyway ✓.
    #[test]
    fn the_chunk_of_the_nether_has_no_sky_light() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, 60, z, 1); // the stone
            }
        }
        let mut nbt = ChunkLoader::chunk_to_nbt(&chunk, 0, Vec::new());
        // The height map is missing: the chunk is initialised with
        // `Chunk.initLighting` ✓.
        if let NbtTag::Compound(root) = &mut nbt {
            if let Some(NbtTag::Compound(level)) = root.get_mut("Level") {
                level.shift_remove("HeightMap");
            }
        }

        let overworld = ChunkLoader::chunk_from_nbt(&nbt, false).unwrap();
        assert_eq!(overworld.get_height(0, 0), 61);
        assert_eq!(overworld.get_sky_light(0, 62, 0), 15);

        let nether = ChunkLoader::chunk_from_nbt(&nbt, true).unwrap();
        assert_eq!(nether.get_height(0, 0), 61);
        assert_eq!(nether.get_sky_light(0, 62, 0), 0);
    }
}
