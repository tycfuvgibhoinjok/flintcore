//! Projectiles (`EntityArrow`, `EntitySnowball`, `EntityEgg`, `EntityFireball`, `EntityFish`),
//! `EntityTNTPrimed`, `EntityMinecart`, and `EntityPainting` ported from `.references/Project-Poseidon`.

use crate::gameplay::items::ItemStack;
use crate::util::rng::JavaRandom;
use crate::world::chunk::ChunkPos;
use crate::world::storage::nbt::NbtTag;

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
    /// Java `EntityArrow.shake`: the arrow that hit a block shakes for seven
    /// ticks, it can only be picked up when the counter is zero.
    pub shake: i32,
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

pub const MINECART_LENGTH: f32 = 0.98;
pub const MINECART_WIDTH: f32 = 0.70;
pub const MINECART_HEIGHT_OFFSET: f64 = (MINECART_WIDTH / 2.0) as f64;

/// Java `EntityMinecart.matrix` (`EntityMinecart.java:23`).
pub const RAIL_MATRIX: [[[i32; 3]; 2]; 10] = [
    [[0, 0, -1], [0, 0, 1]],
    [[-1, 0, 0], [1, 0, 0]],
    [[-1, -1, 0], [1, 0, 0]],
    [[-1, 0, 0], [1, -1, 0]],
    [[0, 0, -1], [0, -1, 1]],
    [[0, -1, -1], [0, 0, 1]],
    [[0, 0, 1], [1, 0, 0]],
    [[0, 0, 1], [-1, 0, 0]],
    [[0, 0, -1], [-1, 0, 0]],
    [[0, 0, -1], [1, 0, 0]],
];

#[derive(Debug, Clone)]
pub struct Minecart {
    pub entity_id: i32,
    pub cart_type: u8, // 0 = rideable (type 10), 1 = chest (type 11), 2 = furnace (type 12)
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub last_x: f64,
    pub last_y: f64,
    pub last_z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub last_yaw: f32,
    pub damage: i32,
    pub time_since_hit: i32,
    pub rock_direction: i32,
    pub yaw_flipped: bool,
    pub fuel: i32,
    pub push_x: f64,
    pub push_z: f64,
    pub slow_when_empty: bool,
    pub derailed_x: f64,
    pub derailed_y: f64,
    pub derailed_z: f64,
    pub flying_x: f64,
    pub flying_y: f64,
    pub flying_z: f64,
    pub max_speed: f64,
    pub passenger_id: Option<i32>,
    pub cargo: [Option<ItemStack>; 27],
    pub on_ground: bool,
    pub fall_distance: f32,
    pub fire_ticks: i32,
    pub rng: JavaRandom,
    pub dead: bool,
}

impl Minecart {
    /// Mirror `EntityMinecart(World, double d0, double d1, double d2, int type)` (`EntityMinecart.java:76-88`).
    pub fn new(entity_id: i32, cart_type: u8, d0: f64, d1: f64, d2: f64) -> Self {
        Self {
            entity_id,
            cart_type,
            x: d0,
            y: d1 + MINECART_HEIGHT_OFFSET,
            z: d2,
            last_x: d0,
            last_y: d1,
            last_z: d2,
            mot_x: 0.0,
            mot_y: 0.0,
            mot_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            last_yaw: 0.0,
            damage: 0,
            time_since_hit: 0,
            rock_direction: 1,
            yaw_flipped: false,
            fuel: 0,
            push_x: 0.0,
            push_z: 0.0,
            slow_when_empty: true,
            derailed_x: 0.5,
            derailed_y: 0.5,
            derailed_z: 0.5,
            flying_x: 0.95,
            flying_y: 0.95,
            flying_z: 0.95,
            max_speed: 0.4,
            passenger_id: None,
            cargo: [None; 27],
            on_ground: false,
            fall_distance: 0.0,
            fire_ticks: 0,
            rng: JavaRandom::new((entity_id as i64).wrapping_mul(341_873_128_712)),
            dead: false,
        }
    }

    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.x = x;
        self.y = y;
        self.z = z;
    }

    /// Java `Entity.setPosition` AABB `(min_x, min_y, min_z, max_x, max_y, max_z)` (`Entity.java:199-207`).
    pub fn bounding_box(&self) -> (f64, f64, f64, f64, f64, f64) {
        let half_len = (MINECART_LENGTH / 2.0) as f64;
        let width_h = MINECART_WIDTH as f64;
        let min_y = self.y - MINECART_HEIGHT_OFFSET;
        (
            self.x - half_len,
            min_y,
            self.z - half_len,
            self.x + half_len,
            min_y + width_h,
            self.z + half_len,
        )
    }

    /// Java `EntityMinecart.m()` (`EntityMinecart.java:90-92`).
    pub fn mounted_y_offset(&self) -> f64 {
        (MINECART_WIDTH as f64) * 0.0 - 0.30000001192092896
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x.floor() as i32) >> 4, (self.z.floor() as i32) >> 4)
    }

    pub fn vehicle_type(&self) -> u8 {
        10 + self.cart_type.min(2)
    }

    /// Java `EntityMinecart.b(NBTTagCompound)` (`EntityMinecart.java:658-679`).
    pub fn to_nbt(&self) -> NbtTag {
        let mut tag = NbtTag::compound();
        tag.insert("id", NbtTag::String("Minecart".to_string()));
        tag.insert(
            "Pos",
            NbtTag::List(vec![
                NbtTag::Double(self.x),
                NbtTag::Double(self.y),
                NbtTag::Double(self.z),
            ]),
        );
        tag.insert(
            "Motion",
            NbtTag::List(vec![
                NbtTag::Double(self.mot_x),
                NbtTag::Double(self.mot_y),
                NbtTag::Double(self.mot_z),
            ]),
        );
        tag.insert(
            "Rotation",
            NbtTag::List(vec![NbtTag::Float(self.yaw), NbtTag::Float(self.pitch)]),
        );
        tag.insert("FallDistance", NbtTag::Float(self.fall_distance));
        tag.insert("Fire", NbtTag::Short(self.fire_ticks as i16));
        tag.insert("Air", NbtTag::Short(300));
        tag.insert("OnGround", NbtTag::Byte(i8::from(self.on_ground)));
        tag.insert("Type", NbtTag::Int(self.cart_type as i32));
        if self.cart_type == 2 {
            tag.insert("PushX", NbtTag::Double(self.push_x));
            tag.insert("PushZ", NbtTag::Double(self.push_z));
            tag.insert("Fuel", NbtTag::Short(self.fuel as i16));
        } else if self.cart_type == 1 {
            let mut list = Vec::new();
            for (slot, item) in self.cargo.iter().enumerate() {
                if let Some(stack) = item {
                    let mut st = NbtTag::compound();
                    st.insert("Slot", NbtTag::Byte(slot as i8));
                    st.insert("id", NbtTag::Short(stack.id));
                    st.insert("Count", NbtTag::Byte(stack.count as i8));
                    st.insert("Damage", NbtTag::Short(stack.damage));
                    list.push(st);
                }
            }
            tag.insert("Items", NbtTag::List(list));
        }
        tag
    }

    /// Java `EntityMinecart.a(NBTTagCompound)` (`EntityMinecart.java:681-701`).
    pub fn from_nbt(entity_id: i32, tag: &NbtTag) -> Option<Self> {
        if tag.get_str("id")? != "Minecart" {
            return None;
        }
        let pos = tag.get_list("Pos")?;
        if pos.len() < 3 {
            return None;
        }
        let x = match pos[0] {
            NbtTag::Double(v) => v,
            _ => return None,
        };
        let y = match pos[1] {
            NbtTag::Double(v) => v,
            _ => return None,
        };
        let z = match pos[2] {
            NbtTag::Double(v) => v,
            _ => return None,
        };
        let cart_type = (tag.get_i32("Type").unwrap_or(0) as u8).min(2);
        let mut cart = Self::new(entity_id, cart_type, x, y - MINECART_HEIGHT_OFFSET, z);
        cart.y = y;
        cart.last_y = y;
        if let Some(mot) = tag.get_list("Motion") {
            if mot.len() >= 3 {
                if let NbtTag::Double(vx) = mot[0] {
                    cart.mot_x = vx;
                }
                if let NbtTag::Double(vy) = mot[1] {
                    cart.mot_y = vy;
                }
                if let NbtTag::Double(vz) = mot[2] {
                    cart.mot_z = vz;
                }
            }
        }
        if let Some(rot) = tag.get_list("Rotation") {
            if rot.len() >= 2 {
                if let NbtTag::Float(yw) = rot[0] {
                    cart.yaw = yw;
                    cart.last_yaw = yw;
                }
                if let NbtTag::Float(pt) = rot[1] {
                    cart.pitch = pt;
                }
            }
        }
        cart.fall_distance = tag.get_f32("FallDistance").unwrap_or(0.0);
        cart.fire_ticks = tag.get_i16("Fire").unwrap_or(0) as i32;
        cart.on_ground = tag.get_i8("OnGround").unwrap_or(0) != 0;
        if cart_type == 2 {
            cart.push_x = tag.get_f64("PushX").unwrap_or(0.0);
            cart.push_z = tag.get_f64("PushZ").unwrap_or(0.0);
            cart.fuel = tag.get_i16("Fuel").unwrap_or(0) as i32;
        } else if cart_type == 1 {
            if let Some(items) = tag.get_list("Items") {
                for entry in items {
                    let slot = (entry.get_i8("Slot").unwrap_or(-1) as u8) as usize;
                    let id = entry.get_i16("id").unwrap_or(0);
                    let count = entry.get_i8("Count").unwrap_or(0) as u8;
                    let damage = entry.get_i16("Damage").unwrap_or(0);
                    if slot < 27 && id > 0 && count > 0 {
                        cart.cargo[slot] = Some(ItemStack::new(id, count, damage));
                    }
                }
            }
        }
        Some(cart)
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
    /// Java `EntityPainting.f`: the counter of the ticks between the checks
    /// of the painting.
    pub tick_count: i32,
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
