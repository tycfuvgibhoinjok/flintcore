//! Projectiles (`EntityArrow`, `EntitySnowball`, `EntityEgg`, `EntityFireball`, `EntityFish`),
//! `EntityTNTPrimed`, `EntityMinecart`, and `EntityPainting` ported from `.references/Project-Poseidon`.

use crate::gameplay::items::ItemStack;
use crate::world::chunk::ChunkPos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectileKind {
    Arrow,       // VehicleSpawn type 60
    Snowball,    // VehicleSpawn type 61
    Egg,         // VehicleSpawn type 62
    Fireball,    // VehicleSpawn type 63
    FishingHook, // VehicleSpawn type 90
}

impl ProjectileKind {
    pub fn vehicle_type(self) -> u8 {
        match self {
            Self::Arrow => 60,
            Self::Snowball => 61,
            Self::Egg => 62,
            Self::Fireball => 63,
            Self::FishingHook => 90,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Projectile {
    pub entity_id: i32,
    pub kind: ProjectileKind,
    pub shooter_id: Option<i32>,
    pub shooter_is_skeleton: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub accel_x: f64,
    pub accel_y: f64,
    pub accel_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub in_ground: bool,
    pub ticks_in_ground: i32,
    pub ticks_in_air: i32,
    pub catch_ticks: i32,
    pub hooked_entity: Option<i32>,
    pub from_player: bool,
    pub dead: bool,
}

impl Projectile {
    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x.floor() as i32) >> 4, (self.z.floor() as i32) >> 4)
    }

    pub fn in_water_or_catching(&self) -> bool {
        self.catch_ticks > 0
    }
}

#[derive(Debug, Clone)]
pub struct PrimedTnt {
    pub entity_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub fuse: i32,
    pub on_ground: bool,
    pub dead: bool,
}

impl PrimedTnt {
    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x.floor() as i32) >> 4, (self.z.floor() as i32) >> 4)
    }
}

#[derive(Debug, Clone)]
pub struct Minecart {
    pub entity_id: i32,
    pub cart_type: u8, // 0 = rideable (type 10), 1 = chest (type 11), 2 = furnace (type 12)
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub damage: i32,
    pub time_since_hit: i32,
    pub fuel: i32,
    pub push_x: f64,
    pub push_z: f64,
    pub passenger_id: Option<i32>,
    pub cargo: [Option<ItemStack>; 27],
    pub on_ground: bool,
    pub dead: bool,
}

impl Minecart {
    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x.floor() as i32) >> 4, (self.z.floor() as i32) >> 4)
    }

    pub fn vehicle_type(&self) -> u8 {
        10 + self.cart_type.min(2)
    }
}

#[derive(Debug, Clone)]
pub struct Painting {
    pub entity_id: i32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub direction: i32,
    pub dead: bool,
}

impl Painting {
    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new(self.x >> 4, self.z >> 4)
    }

    pub fn art_pixel_size(title: &str) -> (i32, i32) {
        match title {
            "Kebab" | "Aztec" | "Alban" | "Aztec2" | "Bomb" | "Plant" | "Wasteland" => (16, 16),
            "Pool" | "Courbet" | "Sea" | "Sunset" | "Creebet" => (32, 16),
            "Wanderer" | "Graham" => (16, 32),
            "Match" | "Bust" | "Stage" | "Void" | "SkullAndRoses" => (32, 32),
            "Fighters" => (64, 32),
            "Pointer" | "Pigscene" | "BurningSkull" => (64, 64),
            "Skeleton" | "DonkeyKong" => (64, 48),
            _ => (16, 16),
        }
    }

    fn center_offset(px: i32) -> f32 {
        if px == 32 || px == 64 {
            0.5
        } else {
            0.0
        }
    }

    /// Compute `(min_x, min_y, min_z, max_x, max_y, max_z)` AABB and list of backing wall block coordinates
    /// (`EntityPainting.b` and `EntityPainting.h` in `EntityPainting.java:57-196`).
    #[allow(clippy::type_complexity)]
    pub fn bounds_and_wall_blocks(
        x: i32,
        y: i32,
        z: i32,
        direction: i32,
        title: &str,
    ) -> ((f64, f64, f64, f64, f64, f64), Vec<(i32, i32, i32)>) {
        let (b_px, c_px) = Self::art_pixel_size(title);
        let mut f = b_px as f32;
        let f1 = (c_px as f32) / 32.0;
        let mut f2 = b_px as f32;
        if direction != 0 && direction != 2 {
            f = 0.5;
        } else {
            f2 = 0.5;
        }
        f /= 32.0;
        f2 /= 32.0;

        let mut f3 = x as f32 + 0.5;
        let mut f4 = y as f32 + 0.5;
        let mut f5 = z as f32 + 0.5;
        let f6 = 0.5625f32;
        match direction {
            0 => {
                f5 -= f6;
                f3 -= Self::center_offset(b_px);
            }
            1 => {
                f3 -= f6;
                f5 += Self::center_offset(b_px);
            }
            2 => {
                f5 += f6;
                f3 += Self::center_offset(b_px);
            }
            _ => {
                f3 += f6;
                f5 -= Self::center_offset(b_px);
            }
        }
        f4 += Self::center_offset(c_px);
        let f7 = -0.00625f32;
        let aabb = (
            (f3 - f - f7) as f64,
            (f4 - f1 - f7) as f64,
            (f5 - f2 - f7) as f64,
            (f3 + f + f7) as f64,
            (f4 + f1 + f7) as f64,
            (f5 + f2 + f7) as f64,
        );

        let w_blocks = b_px / 16;
        let h_blocks = c_px / 16;
        let mut k = x;
        let mut i1 = z;
        if direction == 0 || direction == 2 {
            k = ((f3 as f64) - (b_px as f64 / 32.0)).floor() as i32;
        } else {
            i1 = ((f5 as f64) - (b_px as f64 / 32.0)).floor() as i32;
        }
        let l = ((f4 as f64) - (c_px as f64 / 32.0)).floor() as i32;

        let mut wall = Vec::with_capacity((w_blocks * h_blocks) as usize);
        for k1 in 0..w_blocks {
            for j1 in 0..h_blocks {
                if direction != 0 && direction != 2 {
                    wall.push((x, l + j1, i1 + k1));
                } else {
                    wall.push((k + k1, l + j1, z));
                }
            }
        }
        (aabb, wall)
    }
}

pub const PAINTING_TITLES: &[&str] = &[
    "Kebab",
    "Aztec",
    "Alban",
    "Aztec2",
    "Bomb",
    "Plant",
    "Wasteland",
    "Pool",
    "Courbet",
    "Sea",
    "Sunset",
    "Creebet",
    "Wanderer",
    "Graham",
    "Match",
    "Bust",
    "Stage",
    "Void",
    "SkullAndRoses",
    "Fighters",
    "Pointer",
    "Pigscene",
    "BurningSkull",
    "Skeleton",
    "DonkeyKong",
];
