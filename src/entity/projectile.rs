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
