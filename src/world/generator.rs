use std::sync::atomic::{AtomicBool, Ordering};

use crate::gameplay::blocks;
use crate::util::noise::OctavesNoise;
use crate::util::rng::JavaRandom;
use crate::world::biome::{Biome, TreeKind, WorldChunkManager};
use crate::world::caves::MapGenCaves;
use crate::world::chunk::{Chunk, ChunkPos, CHUNK_HEIGHT, CHUNK_WIDTH};
use crate::world::decorate::{self, GenWorld};

static GENERATE_SANDSTONE: AtomicBool = AtomicBool::new(true);

pub fn set_generate_sandstone(enabled: bool) {
    GENERATE_SANDSTONE.store(enabled, Ordering::Relaxed);
}

const BLOCKS: usize = CHUNK_WIDTH * CHUNK_HEIGHT * CHUNK_WIDTH;
pub const CHUNK_SEED_X: i64 = 341873128712;
pub const CHUNK_SEED_Z: i64 = 132897987541;

fn block_index(x: usize, y: usize, z: usize) -> usize {
    (x << 11) | (z << 7) | y
}

pub struct Generator {
    seed: i64,
    random: JavaRandom,
    noise_k: OctavesNoise,
    noise_l: OctavesNoise,
    noise_m: OctavesNoise,
    noise_n: OctavesNoise,
    noise_o: OctavesNoise,
    selector_512: OctavesNoise,
    selector_8000: OctavesNoise,
    tree_density: OctavesNoise,
    manager: WorldChunkManager,
    caves: MapGenCaves,
}

impl Generator {
    pub fn new(seed: i64) -> Self {
        let mut random = JavaRandom::new(seed);
        let noise_k = OctavesNoise::new(&mut random, 16);
        let noise_l = OctavesNoise::new(&mut random, 16);
        let noise_m = OctavesNoise::new(&mut random, 8);
        let noise_n = OctavesNoise::new(&mut random, 4);
        let noise_o = OctavesNoise::new(&mut random, 4);
        let selector_512 = OctavesNoise::new(&mut random, 10);
        let selector_8000 = OctavesNoise::new(&mut random, 16);
        let tree_density = OctavesNoise::new(&mut random, 8);

        Self {
            seed,
            random,
            noise_k,
            noise_l,
            noise_m,
            noise_n,
            noise_o,
            selector_512,
            selector_8000,
            tree_density,
            manager: WorldChunkManager::new(seed),
            caves: MapGenCaves::new(),
        }
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    pub fn manager_mut(&mut self) -> &mut WorldChunkManager {
        &mut self.manager
    }

    #[allow(clippy::too_many_arguments)]
    fn terrain_field(
        &self,
        x: i32,
        y: i32,
        z: i32,
        count_x: usize,
        count_y: usize,
        count_z: usize,
        temperature: &[f64],
        rain: &[f64],
    ) -> Vec<f64> {
        let horizontal = 684.412;
        let selector_2d_x = 1.121;
        let selector_2d_z = 1.121;

        let g = self.selector_512.noise_array(
            x as f64,
            10.0,
            z as f64,
            count_x,
            1,
            count_z,
            selector_2d_x,
            1.0,
            selector_2d_z,
        );
        let h = self.selector_8000.noise_array(
            x as f64, 10.0, z as f64, count_x, 1, count_z, 200.0, 1.0, 200.0,
        );
        let d = self.noise_m.noise_array(
            x as f64,
            y as f64,
            z as f64,
            count_x,
            count_y,
            count_z,
            horizontal / 80.0,
            horizontal / 160.0,
            horizontal / 80.0,
        );
        let e = self.noise_k.noise_array(
            x as f64, y as f64, z as f64, count_x, count_y, count_z, horizontal, horizontal,
            horizontal,
        );
        let f = self.noise_l.noise_array(
            x as f64, y as f64, z as f64, count_x, count_y, count_z, horizontal, horizontal,
            horizontal,
        );

        let mut out = vec![0.0f64; count_x * count_y * count_z];
        let mut selector_index = 0usize;
        let mut field_index = 0usize;
        let step = 16 / count_x;

        for xi in 0..count_x {
            let temperature_x = xi * step + step / 2;

            for zi in 0..count_z {
                let temperature_z = zi * step + step / 2;
                let temp = temperature[temperature_x * 16 + temperature_z];
                let humidity = rain[temperature_x * 16 + temperature_z] * temp;
                let mut dampening = 1.0 - humidity;
                dampening *= dampening;
                dampening *= dampening;
                dampening = 1.0 - dampening;

                let mut height = (g[selector_index] + 256.0) / 512.0;
                height *= dampening;
                if height > 1.0 {
                    height = 1.0;
                }

                let mut selector = h[selector_index] / 8000.0;
                if selector < 0.0 {
                    selector = -selector * 0.3;
                }
                selector = selector * 3.0 - 2.0;
                if selector < 0.0 {
                    selector /= 2.0;
                    if selector < -1.0 {
                        selector = -1.0;
                    }
                    selector /= 1.4;
                    selector /= 2.0;
                    height = 0.0;
                } else {
                    if selector > 1.0 {
                        selector = 1.0;
                    }
                    selector /= 8.0;
                }

                if height < 0.0 {
                    height = 0.0;
                }
                height += 0.5;
                selector = selector * count_y as f64 / 16.0;
                let anchor = count_y as f64 / 2.0 + selector * 4.0;

                selector_index += 1;

                for yi in 0..count_y {
                    let gradient_base = (yi as f64 - anchor) * 12.0;
                    let mut gradient = gradient_base / height;
                    if gradient < 0.0 {
                        gradient *= 4.0;
                    }

                    let low = e[field_index] / 512.0;
                    let high = f[field_index] / 512.0;
                    let blend = (d[field_index] / 10.0 + 1.0) / 2.0;

                    let mut noise = if blend < 0.0 {
                        low
                    } else if blend > 1.0 {
                        high
                    } else {
                        low + (high - low) * blend
                    };

                    noise -= gradient;
                    if yi > count_y - 4 {
                        let squash = ((yi - (count_y - 4)) as f32 / 3.0) as f64;
                        noise = noise * (1.0 - squash) + -10.0 * squash;
                    }

                    out[field_index] = noise;
                    field_index += 1;
                }
            }
        }

        out
    }

    fn fill_terrain(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        blocks: &mut [u8],
        temperature: &[f64],
        rain: &[f64],
    ) {
        let horizontal = 4usize;
        let water_level = 64i32;
        let count_x = horizontal + 1;
        let count_y = 17usize;
        let count_z = horizontal + 1;

        let field = self.terrain_field(
            chunk_x * horizontal as i32,
            0,
            chunk_z * horizontal as i32,
            count_x,
            count_y,
            count_z,
            temperature,
            rain,
        );

        for i1 in 0..horizontal {
            for j1 in 0..horizontal {
                for k1 in 0..16usize {
                    let d0 = 0.125f64;
                    let d1 = field[((i1) * count_z + j1) * count_y + k1];
                    let d2 = field[((i1) * count_z + j1 + 1) * count_y + k1];
                    let d3 = field[((i1 + 1) * count_z + j1) * count_y + k1];
                    let d4 = field[((i1 + 1) * count_z + j1 + 1) * count_y + k1];
                    let d5 = (field[((i1) * count_z + j1) * count_y + k1 + 1] - d1) * d0;
                    let d6 = (field[((i1) * count_z + j1 + 1) * count_y + k1 + 1] - d2) * d0;
                    let d7 = (field[((i1 + 1) * count_z + j1) * count_y + k1 + 1] - d3) * d0;
                    let d8 = (field[((i1 + 1) * count_z + j1 + 1) * count_y + k1 + 1] - d4) * d0;

                    let mut d1 = d1;
                    let mut d2 = d2;
                    let mut d3 = d3;
                    let mut d4 = d4;

                    for l1 in 0..8usize {
                        let d9 = 0.25f64;
                        let mut d10 = d1;
                        let mut d11 = d2;
                        let d12 = (d3 - d1) * d9;
                        let d13 = (d4 - d2) * d9;

                        for i2 in 0..4usize {
                            let mut index = block_index(i1 * 4 + i2, k1 * 8 + l1, j1 * 4);
                            let d14 = 0.25f64;
                            let mut d15 = d10;
                            let d16 = (d11 - d10) * d14;

                            for k2 in 0..4usize {
                                let temperature_value =
                                    temperature[(i1 * 4 + i2) * 16 + j1 * 4 + k2];
                                let mut id = 0u8;

                                if (k1 * 8 + l1) < water_level as usize {
                                    if temperature_value < 0.5 && (k1 * 8 + l1) >= 63 {
                                        id = blocks::ICE;
                                    } else {
                                        id = blocks::STATIONARY_WATER;
                                    }
                                }

                                if d15 > 0.0 {
                                    id = blocks::STONE;
                                }

                                blocks[index] = id;
                                index += 128;
                                d15 += d16;
                            }

                            d10 += d12;
                            d11 += d13;
                        }

                        d1 += d5;
                        d2 += d6;
                        d3 += d7;
                        d4 += d8;
                    }
                }
            }
        }
    }

    fn replace_surface(&mut self, chunk_x: i32, chunk_z: i32, blocks: &mut [u8], biomes: &[Biome]) {
        let water_level = 64i32;
        let d0 = 0.03125f64;

        let r = self.noise_n.noise_array(
            (chunk_x * 16) as f64,
            (chunk_z * 16) as f64,
            0.0,
            16,
            16,
            1,
            d0,
            d0,
            1.0,
        );
        let s = self.noise_n.noise_array(
            (chunk_x * 16) as f64,
            109.0134,
            (chunk_z * 16) as f64,
            16,
            1,
            16,
            d0,
            1.0,
            d0,
        );
        let t = self.noise_o.noise_array(
            (chunk_x * 16) as f64,
            (chunk_z * 16) as f64,
            0.0,
            16,
            16,
            1,
            d0 * 2.0,
            d0 * 2.0,
            d0 * 2.0,
        );

        for z in 0..16usize {
            for x in 0..16usize {
                let biome = biomes[x * 16 + z];
                let has_sand = r[x * 16 + z] + self.random.next_f64() * 0.2 > 0.0;
                let has_gravel = s[x * 16 + z] + self.random.next_f64() * 0.2 > 3.0;
                let soil_depth = (t[x * 16 + z] / 3.0 + 3.0 + self.random.next_f64() * 0.25) as i32;
                let mut remaining = -1i32;
                let mut top = biome.top_block();
                let mut filler = biome.filler_block();

                let mut y = 127i32;
                while y >= 0 {
                    let index = (x * 16 + z) * 128 + y as usize;

                    if y <= self.random.next_i32_bound(5) {
                        blocks[index] = blocks::BEDROCK;
                    } else {
                        let current = blocks[index];

                        if current == blocks::AIR {
                            remaining = -1;
                        } else if current == blocks::STONE {
                            if remaining == -1 {
                                if soil_depth <= 0 {
                                    top = blocks::AIR;
                                    filler = blocks::STONE;
                                } else if y >= water_level - 4 && y <= water_level + 1 {
                                    top = biome.top_block();
                                    filler = biome.filler_block();
                                    if has_gravel {
                                        top = blocks::AIR;
                                    }
                                    if has_gravel {
                                        filler = blocks::GRAVEL;
                                    }
                                    if has_sand {
                                        top = blocks::SAND;
                                    }
                                    if has_sand {
                                        filler = blocks::SAND;
                                    }
                                }

                                if y < water_level && top == blocks::AIR {
                                    top = blocks::STATIONARY_WATER;
                                }

                                remaining = soil_depth;
                                if y >= water_level - 1 {
                                    blocks[index] = top;
                                } else {
                                    blocks[index] = filler;
                                }
                            } else if remaining > 0 {
                                remaining -= 1;
                                blocks[index] = filler;
                                if remaining == 0
                                    && filler == blocks::SAND
                                    && GENERATE_SANDSTONE.load(Ordering::Relaxed)
                                {
                                    remaining = self.random.next_i32_bound(4);
                                    filler = blocks::SANDSTONE;
                                }
                            }
                        }
                    }

                    y -= 1;
                }
            }
        }
    }

    pub fn generate_base(&mut self, chunk_x: i32, chunk_z: i32) -> Chunk {
        self.random.set_seed(
            (chunk_x as i64)
                .wrapping_mul(CHUNK_SEED_X)
                .wrapping_add((chunk_z as i64).wrapping_mul(CHUNK_SEED_Z)),
        );

        let biomes = self.manager.biome_data(chunk_x * 16, chunk_z * 16, 16, 16);
        let temperature = self.manager.temperature().to_vec();
        let rain = self.manager.rain().to_vec();

        let mut blocks = vec![blocks::AIR; BLOCKS];
        self.fill_terrain(chunk_x, chunk_z, &mut blocks, &temperature, &rain);
        self.replace_surface(chunk_x, chunk_z, &mut blocks, &biomes);
        self.caves
            .generate(self.seed, chunk_x, chunk_z, &mut blocks);

        let mut chunk = Chunk::new(ChunkPos::new(chunk_x, chunk_z));
        for x in 0..CHUNK_WIDTH {
            for z in 0..CHUNK_WIDTH {
                chunk.biomes[z * 16 + x] = biomes[x * 16 + z].id();
                for y in 0..CHUNK_HEIGHT {
                    let id = blocks[block_index(x, y, z)];
                    if id != blocks::AIR {
                        chunk.set_block(x, y, z, id);
                    }
                }
            }
        }
        chunk.init_lighting();
        chunk.state = crate::world::chunk::ChunkState::Generated;
        chunk
    }

    pub fn populate(&mut self, world: &mut dyn GenWorld, chunk_x: i32, chunk_z: i32) {
        let k = chunk_x * 16;
        let l = chunk_z * 16;
        let biome = self.manager.biome_at(k + 16, l + 16);

        self.random.set_seed(self.seed);
        let i1 = self.random.next_i64() / 2 * 2 + 1;
        let j1 = self.random.next_i64() / 2 * 2 + 1;
        self.random.set_seed(
            ((chunk_x as i64)
                .wrapping_mul(i1)
                .wrapping_add((chunk_z as i64).wrapping_mul(j1)))
                ^ self.seed,
        );

        if self.random.next_i32_bound(4) == 0 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_lakes(world, &mut self.random, x, y, z, blocks::STATIONARY_WATER);
        }

        if self.random.next_i32_bound(8) == 0 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let bound = self.random.next_i32_bound(120) + 8;
            let y = self.random.next_i32_bound(bound);
            let z = l + self.random.next_i32_bound(16) + 8;
            if y < 64 || self.random.next_i32_bound(10) == 0 {
                decorate::generate_lakes(world, &mut self.random, x, y, z, blocks::STATIONARY_LAVA);
            }
        }

        for _ in 0..8 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_dungeons(world, &mut self.random, x, y, z);
        }

        for _ in 0..10 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_clay(world, &mut self.random, x, y, z, 32);
        }

        for _ in 0..20 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::DIRT, 32);
        }

        for _ in 0..10 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::GRAVEL, 32);
        }

        for _ in 0..20 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::COAL_ORE, 16);
        }

        for _ in 0..20 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(64);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::IRON_ORE, 8);
        }

        for _ in 0..2 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(32);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::GOLD_ORE, 8);
        }

        for _ in 0..8 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(16);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::REDSTONE_ORE, 7);
        }

        for _ in 0..1 {
            let x = k + self.random.next_i32_bound(16);
            let y = self.random.next_i32_bound(16);
            let z = l + self.random.next_i32_bound(16);
            decorate::generate_minable(world, &mut self.random, x, y, z, blocks::DIAMOND_ORE, 7);
        }

        let wg = world.worldgen_config();
        if wg.generate_lapis_ores {
            for _ in 0..1 {
                let x = k + self.random.next_i32_bound(16);
                let a = self.random.next_i32_bound(16);
                let y = a + self.random.next_i32_bound(16);
                let z = l + self.random.next_i32_bound(16);
                decorate::generate_minable(world, &mut self.random, x, y, z, blocks::LAPIS_ORE, 6);
            }
        }

        let d0 = 0.5;
        let density = ((self.tree_density.noise_2d(k as f64 * d0, l as f64 * d0) / 8.0
            + self.random.next_f64() * 4.0
            + 4.0)
            / 3.0) as i32;

        let mut tree_count = 0;
        if self.random.next_i32_bound(10) == 0 {
            tree_count += 1;
        }
        match biome {
            Biome::Forest | Biome::Rainforest | Biome::Taiga => tree_count += density + 5,
            Biome::SeasonalForest => tree_count += density + 2,
            Biome::Desert | Biome::Tundra | Biome::Plains => tree_count -= 20,
            _ => {}
        }

        for _ in 0..tree_count.max(0) {
            let x = k + self.random.next_i32_bound(16) + 8;
            let z = l + self.random.next_i32_bound(16) + 8;
            let mut kind = if wg.pre_b1_2_tree_generation {
                let mut knd = if self.random.next_i32_bound(10) == 0 {
                    TreeKind::BigTree
                } else {
                    TreeKind::Trees
                };
                if biome == Biome::Rainforest && self.random.next_i32_bound(3) == 0 {
                    knd = TreeKind::BigTree;
                }
                knd
            } else {
                biome.tree_kind(&mut self.random)
            };
            if ((kind == TreeKind::Taiga1 || kind == TreeKind::Taiga2) && !wg.generate_spruces)
                || (kind == TreeKind::Forest && !wg.generate_birches)
            {
                kind = TreeKind::Trees;
            }
            let y = world.highest_block_y(x, z);
            decorate::generate_tree(world, &mut self.random, kind, x, y, z);
        }

        let flowers = match biome {
            Biome::Forest => 2,
            Biome::SeasonalForest => 4,
            Biome::Taiga => 2,
            Biome::Plains => 3,
            _ => 0,
        };
        for _ in 0..flowers.max(0) {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_flowers(world, &mut self.random, x, y, z, blocks::YELLOW_FLOWER);
        }

        let grass = match biome {
            Biome::Forest => 2,
            Biome::Rainforest => 10,
            Biome::SeasonalForest => 2,
            Biome::Taiga => 1,
            Biome::Plains => 10,
            _ => 0,
        };
        if wg.generate_tallgrass {
            for _ in 0..grass.max(0) {
                let mut data = 1u8;
                if biome == Biome::Rainforest && self.random.next_i32_bound(3) != 0 {
                    data = 2;
                }
                let x = k + self.random.next_i32_bound(16) + 8;
                let y = self.random.next_i32_bound(128);
                let z = l + self.random.next_i32_bound(16) + 8;
                decorate::generate_grass(
                    world,
                    &mut self.random,
                    x,
                    y,
                    z,
                    blocks::TALL_GRASS,
                    data,
                );
            }
        }

        let dead_bush = if biome == Biome::Desert { 2 } else { 0 };
        if wg.generate_tallgrass {
            for _ in 0..dead_bush {
                let x = k + self.random.next_i32_bound(16) + 8;
                let y = self.random.next_i32_bound(128);
                let z = l + self.random.next_i32_bound(16) + 8;
                decorate::generate_dead_bush(world, &mut self.random, x, y, z, blocks::DEAD_BUSH);
            }
        }

        if self.random.next_i32_bound(2) == 0 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_flowers(world, &mut self.random, x, y, z, blocks::RED_ROSE);
        }

        if self.random.next_i32_bound(4) == 0 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_flowers(world, &mut self.random, x, y, z, blocks::BROWN_MUSHROOM);
        }

        if self.random.next_i32_bound(8) == 0 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_flowers(world, &mut self.random, x, y, z, blocks::RED_MUSHROOM);
        }

        for _ in 0..10 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_reed(world, &mut self.random, x, y, z);
        }

        if self.random.next_i32_bound(32) == 0 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_pumpkin(world, &mut self.random, x, y, z);
        }

        let cacti = if biome == Biome::Desert { 10 } else { 0 };
        for _ in 0..cacti {
            let x = k + self.random.next_i32_bound(16) + 8;
            let y = self.random.next_i32_bound(128);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_cactus(world, &mut self.random, x, y, z);
        }

        for _ in 0..50 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let bound = self.random.next_i32_bound(120) + 8;
            let y = self.random.next_i32_bound(bound);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_liquids(world, &mut self.random, x, y, z, blocks::WATER);
        }

        for _ in 0..20 {
            let x = k + self.random.next_i32_bound(16) + 8;
            let b1 = self.random.next_i32_bound(112) + 8;
            let b2 = self.random.next_i32_bound(b1) + 8;
            let y = self.random.next_i32_bound(b2);
            let z = l + self.random.next_i32_bound(16) + 8;
            decorate::generate_liquids(world, &mut self.random, x, y, z, blocks::LAVA);
        }

        self.manager.biome_data(k + 8, l + 8, 16, 16);
        let temperature = self.manager.temperature().to_vec();
        for i3 in k + 8..k + 8 + 16 {
            for l2 in l + 8..l + 8 + 16 {
                let k3 = i3 - (k + 8);
                let j3 = l2 - (l + 8);
                let l3 = world.highest_solid_y(i3, l2);
                let d1 = temperature[(k3 * 16 + j3) as usize] - (l3 - 64) as f64 / 64.0 * 0.3;
                if d1 < 0.5
                    && l3 > 0
                    && l3 < 128
                    && world.is_empty(i3, l3, l2)
                    && blocks::is_solid(world.get_block(i3, l3 - 1, l2))
                    && !blocks::is_ice(world.get_block(i3, l3 - 1, l2))
                {
                    world.set_block_physics(i3, l3, l2, blocks::SNOW);
                }
            }
        }

        if wg.generate_steveco_chests {
            let k2 = k + self.random.next_i32_bound(16) + 8;
            let l2 = self.random.next_i32_bound(128);
            let i3 = l + self.random.next_i32_bound(16) + 8;
            if l2 > 0
                && world.get_block(k2, l2, i3) == blocks::AIR
                && blocks::material(world.get_block(k2, l2 - 1, i3)).is_solid()
            {
                world.set_block_physics(k2, l2, i3, blocks::LOCKED_CHEST);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &[(i64, i32, i32, i64, u32)] = &[
        (12345, 0, 0, -5416338515155351236, 21269),
        (12345, 1, 2, 2269812607796579776, 21534),
        (12345, -1, -1, -6473394275854444781, 21410),
        (12345, 10, -7, -5055962380375063859, 17710),
        (0, 0, 0, 6922452243744838306, 22419),
        (0, 1, 2, 4322084082189298131, 27146),
        (0, -1, -1, -3341028623372444707, 21782),
        (0, 10, -7, -1459127559241918174, 16961),
        (-1, 0, 0, -2577433475730723513, 17521),
        (-1, 1, 2, 3710244815211913936, 19655),
    ];

    fn fnv(blocks: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &value in blocks {
            hash ^= value as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn base_generation_matches_vanilla() {
        for &(seed, chunk_x, chunk_z, expected_hash, expected_non_air) in GOLDEN {
            let mut generator = Generator::new(seed);
            let chunk = generator.generate_base(chunk_x, chunk_z);

            let mut buffer = vec![0u8; BLOCKS];
            let mut non_air = 0u32;
            for index in 0..BLOCKS {
                let x = index >> 11;
                let z = (index >> 7) & 15;
                let y = index & 127;
                let id = chunk.get_block(x, y, z);
                buffer[index] = id;
                if id != blocks::AIR {
                    non_air += 1;
                }
            }

            assert_eq!(
                fnv(&buffer) as i64,
                expected_hash,
                "hash mismatch for seed {seed} chunk ({chunk_x},{chunk_z})"
            );
            assert_eq!(
                non_air, expected_non_air,
                "non-air mismatch for seed {seed} chunk ({chunk_x},{chunk_z})"
            );
        }
    }

    use std::cell::RefCell;
    use std::collections::HashMap;

    struct TestWorld {
        chunks: RefCell<HashMap<(i32, i32), Chunk>>,
        base: RefCell<Generator>,
    }

    impl TestWorld {
        fn new(seed: i64) -> Self {
            Self {
                chunks: RefCell::new(HashMap::new()),
                base: RefCell::new(Generator::new(seed)),
            }
        }

        fn ensure(&self, cx: i32, cz: i32) {
            if self.chunks.borrow().contains_key(&(cx, cz)) {
                return;
            }
            let chunk = self.base.borrow_mut().generate_base(cx, cz);
            self.chunks.borrow_mut().insert((cx, cz), chunk);
        }

        fn recalc_height(&self, cx: i32, cz: i32, lx: usize, lz: usize) {
            let mut chunks = self.chunks.borrow_mut();
            let chunk = chunks.get_mut(&(cx, cz)).unwrap();
            let mut h = CHUNK_HEIGHT as i32 - 1;
            while h > 0 && blocks::light_opacity(chunk.get_block(lx, h as usize - 1, lz)) == 0 {
                h -= 1;
            }
            chunk.height_map[(lz << 4) | lx] = h as u8;
        }
    }

    impl GenWorld for TestWorld {
        fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
            if !(0..CHUNK_HEIGHT as i32).contains(&y) {
                return blocks::AIR;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks.borrow()[&(cx, cz)].get_block(
                (x & 15) as usize,
                y as usize,
                (z & 15) as usize,
            )
        }

        fn get_data(&self, x: i32, y: i32, z: i32) -> u8 {
            if !(0..CHUNK_HEIGHT as i32).contains(&y) {
                return 0;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks.borrow()[&(cx, cz)].get_metadata(
                (x & 15) as usize,
                y as usize,
                (z & 15) as usize,
            )
        }

        fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
            if !(0..CHUNK_HEIGHT as i32).contains(&y) {
                return;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks
                .borrow_mut()
                .get_mut(&(cx, cz))
                .unwrap()
                .set_block((x & 15) as usize, y as usize, (z & 15) as usize, id);
            self.recalc_height(cx, cz, (x & 15) as usize, (z & 15) as usize);
        }

        fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
            if !(0..CHUNK_HEIGHT as i32).contains(&y) {
                return;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            let mut chunks = self.chunks.borrow_mut();
            let chunk = chunks.get_mut(&(cx, cz)).unwrap();
            chunk.set_block((x & 15) as usize, y as usize, (z & 15) as usize, id);
            chunk.set_metadata((x & 15) as usize, y as usize, (z & 15) as usize, data);
            drop(chunks);
            self.recalc_height(cx, cz, (x & 15) as usize, (z & 15) as usize);
        }

        fn highest_block_y(&self, x: i32, z: i32) -> i32 {
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks.borrow()[&(cx, cz)].get_height((x & 15) as usize, (z & 15) as usize) as i32
        }

        fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
            if y < 0 {
                return 0;
            }
            if y >= CHUNK_HEIGHT as i32 {
                return 15;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks.borrow()[&(cx, cz)].get_sky_light(
                (x & 15) as usize,
                y as usize,
                (z & 15) as usize,
            ) as i32
        }

        fn light_value(&self, x: i32, y: i32, z: i32) -> i32 {
            if y < 0 {
                return 0;
            }
            if y >= CHUNK_HEIGHT as i32 {
                return 15;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks.borrow()[&(cx, cz)].get_block_light_value(
                (x & 15) as usize,
                y as usize,
                (z & 15) as usize,
                0,
            )
        }

        fn see_sky(&self, x: i32, y: i32, z: i32) -> bool {
            if y < 0 {
                return false;
            }
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            self.chunks.borrow()[&(cx, cz)].can_block_see_sky(
                (x & 15) as usize,
                y as usize,
                (z & 15) as usize,
            )
        }

        fn highest_solid_y(&self, x: i32, z: i32) -> i32 {
            let (cx, cz) = (x >> 4, z >> 4);
            self.ensure(cx, cz);
            let chunk = &self.chunks.borrow()[&(cx, cz)];
            for y in (1..CHUNK_HEIGHT as i32).rev() {
                let id = chunk.get_block((x & 15) as usize, y as usize, (z & 15) as usize);
                if blocks::is_solid(id) || blocks::is_liquid(id) {
                    return y + 1;
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

    const GOLDEN_POP: &[(i64, i32, i32, i64, u32)] = &[
        (12345, 0, 0, -8563560949885519596, 21270),
        (12345, 1, 0, -3665013329563741994, 21417),
        (12345, 0, 1, -5948858733794687256, 21731),
        (12345, -1, 2, 3895952963359369696, 21903),
        (12345, 5, -3, 8542155410691632459, 20677),
        (12345, 10, 10, -639065288296654524, 23019),
        (12345, -7, -7, -5260984423991573043, 16876),
        (12345, 3, 9, 242164828331727552, 20148),
        (12345, 16, 16, 6512137662491223957, 16195),
        (12345, -20, 4, -6445381530264815792, 16181),
        (0, 0, 0, 3601655530815687693, 23355),
        (0, 1, 0, -5638991513396661127, 24689),
        (0, 0, 1, 8974827294333346569, 23256),
        (0, -1, 2, 1830492721445748991, 20073),
        (0, 5, -3, -4289693764916265238, 19837),
        (0, 10, 10, 4440931059072104900, 16280),
        (0, -7, -7, 4267913927296213586, 16404),
        (0, 3, 9, -595566689927176311, 23198),
        (0, 16, 16, 2100115367200694326, 16206),
        (0, -20, 4, -5120192868539967383, 15908),
        (-1, 0, 0, -5754128858978052367, 17521),
        (-1, 1, 0, -6689661608678302302, 17864),
        (-1, 0, 1, 364035544191218120, 16380),
        (-1, -1, 2, -1579796610009547624, 17623),
        (-1, 5, -3, 1190008376297691081, 16006),
        (-1, 10, 10, -3478631914714908422, 16358),
        (-1, -7, -7, 262506598090135388, 18431),
        (-1, 3, 9, 2120324043367761061, 15961),
        (-1, 16, 16, -5332388115073882507, 16396),
        (-1, -20, 4, -892961142311109326, 15663),
    ];

    #[test]
    fn populated_generation_matches_vanilla() {
        for &(seed, chunk_x, chunk_z, expected_hash, expected_non_air) in GOLDEN_POP {
            let mut world = TestWorld::new(seed);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    world.get_block((chunk_x + dx) * 16 + 8, 64, (chunk_z + dz) * 16 + 8);
                }
            }
            let mut populate_gen = Generator::new(seed);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    populate_gen.populate(&mut world, chunk_x + dx, chunk_z + dz);
                }
            }

            let chunks = world.chunks.borrow();
            let chunk = &chunks[&(chunk_x, chunk_z)];
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            let mut non_air = 0u32;
            for x in 0..CHUNK_WIDTH {
                for z in 0..CHUNK_WIDTH {
                    for y in 0..CHUNK_HEIGHT {
                        let id = chunk.get_block(x, y, z);
                        let meta = chunk.get_metadata(x, y, z);
                        hash ^= id as u64;
                        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                        hash ^= (meta & 15) as u64;
                        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                        if id != blocks::AIR {
                            non_air += 1;
                        }
                    }
                }
            }

            assert_eq!(
                hash as i64, expected_hash,
                "populate hash mismatch for seed {seed} chunk ({chunk_x},{chunk_z})"
            );
            assert_eq!(
                non_air, expected_non_air,
                "populate non-air mismatch for seed {seed} chunk ({chunk_x},{chunk_z})"
            );
        }
    }
}
