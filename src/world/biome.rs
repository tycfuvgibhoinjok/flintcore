use crate::gameplay::blocks;
use crate::util::noise::SimplexOctavesNoise;
use crate::util::rng::JavaRandom;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Rainforest,
    Swampland,
    SeasonalForest,
    Forest,
    Savanna,
    Shrubland,
    Taiga,
    Desert,
    Plains,
    IceDesert,
    Tundra,
    Hell,
    Sky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    Trees,
    BigTree,
    Forest,
    Taiga1,
    Taiga2,
}

impl Biome {
    pub fn id(self) -> u8 {
        match self {
            Biome::Rainforest => 0,
            Biome::Swampland => 1,
            Biome::SeasonalForest => 2,
            Biome::Forest => 3,
            Biome::Savanna => 4,
            Biome::Shrubland => 5,
            Biome::Taiga => 6,
            Biome::Desert => 7,
            Biome::Plains => 8,
            Biome::IceDesert => 9,
            Biome::Tundra => 10,
            Biome::Hell => 11,
            Biome::Sky => 12,
        }
    }

    pub fn from_id(id: u8) -> Self {
        match id {
            0 => Biome::Rainforest,
            1 => Biome::Swampland,
            2 => Biome::SeasonalForest,
            3 => Biome::Forest,
            4 => Biome::Savanna,
            5 => Biome::Shrubland,
            6 => Biome::Taiga,
            7 => Biome::Desert,
            8 => Biome::Plains,
            9 => Biome::IceDesert,
            10 => Biome::Tundra,
            11 => Biome::Hell,
            12 => Biome::Sky,
            _ => Biome::Plains,
        }
    }

    pub fn top_block(self) -> u8 {
        match self {
            Biome::Desert | Biome::IceDesert => blocks::SAND,
            _ => blocks::GRASS,
        }
    }

    pub fn filler_block(self) -> u8 {
        match self {
            Biome::Desert | Biome::IceDesert => blocks::SAND,
            _ => blocks::DIRT,
        }
    }

    pub fn tree_kind(self, random: &mut JavaRandom) -> TreeKind {
        match self {
            Biome::Rainforest => {
                if random.next_i32_bound(3) == 0 {
                    TreeKind::BigTree
                } else {
                    TreeKind::Trees
                }
            }
            Biome::Forest => {
                if random.next_i32_bound(5) == 0 {
                    TreeKind::Forest
                } else if random.next_i32_bound(3) == 0 {
                    TreeKind::BigTree
                } else {
                    TreeKind::Trees
                }
            }
            Biome::Taiga => {
                if random.next_i32_bound(3) == 0 {
                    TreeKind::Taiga1
                } else {
                    TreeKind::Taiga2
                }
            }
            _ => {
                if random.next_i32_bound(10) == 0 {
                    TreeKind::BigTree
                } else {
                    TreeKind::Trees
                }
            }
        }
    }
}

fn classify(temperature: f32, humidity: f32) -> Biome {
    let humidity = humidity * temperature;
    if temperature < 0.1 {
        Biome::Tundra
    } else if humidity < 0.2 {
        if temperature < 0.5 {
            Biome::Tundra
        } else if temperature < 0.95 {
            Biome::Savanna
        } else {
            Biome::Desert
        }
    } else if humidity > 0.5 && temperature < 0.7 {
        Biome::Swampland
    } else if temperature < 0.5 {
        Biome::Taiga
    } else if temperature < 0.97 {
        if humidity < 0.35 {
            Biome::Shrubland
        } else {
            Biome::Forest
        }
    } else if humidity < 0.45 {
        Biome::Plains
    } else if humidity < 0.9 {
        Biome::SeasonalForest
    } else {
        Biome::Rainforest
    }
}

static BIOME_TABLE: std::sync::OnceLock<[Biome; 4096]> = std::sync::OnceLock::new();

fn biome_table() -> &'static [Biome; 4096] {
    BIOME_TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index % 64;
            let z = index / 64;
            classify(x as f32 / 63.0, z as f32 / 63.0)
        })
    })
}

pub fn biome_from_temperature(temperature: f64, humidity: f64) -> Biome {
    let x = (temperature * 63.0) as i64;
    let z = (humidity * 63.0) as i64;
    biome_table()[z as usize * 64 + x as usize]
}

pub struct WorldChunkManager {
    temperature_noise: SimplexOctavesNoise,
    rain_noise: SimplexOctavesNoise,
    detail_noise: SimplexOctavesNoise,
    temperature: Vec<f64>,
    rain: Vec<f64>,
    detail: Vec<f64>,
}

impl WorldChunkManager {
    pub fn new(seed: i64) -> Self {
        Self {
            temperature_noise: SimplexOctavesNoise::new(
                &mut JavaRandom::new(seed.wrapping_mul(9871)),
                4,
            ),
            rain_noise: SimplexOctavesNoise::new(&mut JavaRandom::new(seed.wrapping_mul(39811)), 4),
            detail_noise: SimplexOctavesNoise::new(
                &mut JavaRandom::new(seed.wrapping_mul(543321)),
                2,
            ),
            temperature: Vec::new(),
            rain: Vec::new(),
            detail: Vec::new(),
        }
    }

    pub fn temperature(&self) -> &[f64] {
        &self.temperature
    }

    pub fn rain(&self) -> &[f64] {
        &self.rain
    }

    pub fn biome_at(&mut self, x: i32, z: i32) -> Biome {
        self.biome_data(x, z, 1, 1)[0]
    }

    pub fn biome_data(&mut self, x: i32, z: i32, count_x: usize, count_z: usize) -> Vec<Biome> {
        self.temperature = self.temperature_noise.noise_array(
            x as f64,
            z as f64,
            count_x,
            count_x,
            0.02500000037252903,
            0.02500000037252903,
            0.25,
        );
        self.rain = self.rain_noise.noise_array(
            x as f64,
            z as f64,
            count_x,
            count_x,
            0.05000000074505806,
            0.05000000074505806,
            1.0 / 3.0,
        );
        self.detail = self.detail_noise.noise_array(
            x as f64,
            z as f64,
            count_x,
            count_x,
            0.25,
            0.25,
            0.5882352941176471,
        );

        let mut biomes = vec![Biome::Tundra; count_x * count_z];
        let mut index = 0;
        for _ in 0..count_x {
            for _ in 0..count_z {
                let base = self.detail[index] * 1.1 + 0.5;
                let weight = 0.01;
                let inverse = 1.0 - weight;
                let mut temp = (self.temperature[index] * 0.15 + 0.7) * inverse + base * weight;

                let rain_weight = 0.0020;
                let rain_inverse = 1.0 - rain_weight;
                let mut rain = (self.rain[index] * 0.15 + 0.5) * rain_inverse + base * rain_weight;

                temp = 1.0 - (1.0 - temp) * (1.0 - temp);
                if temp < 0.0 {
                    temp = 0.0;
                }
                if rain < 0.0 {
                    rain = 0.0;
                }
                if temp > 1.0 {
                    temp = 1.0;
                }
                if rain > 1.0 {
                    rain = 1.0;
                }

                self.temperature[index] = temp;
                self.rain[index] = rain;
                biomes[index] = biome_from_temperature(temp, rain);
                index += 1;
            }
        }

        biomes
    }
}
