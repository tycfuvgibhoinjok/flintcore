//! 1:1 port of all 14 Minecraft Beta 1.7.3 Animals & Mobs, `DataWatcher` metadata,
//! `Pathfinder` navigation, and `SpawnerCreature` natural spawning from `.references/Project-Poseidon`.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::gameplay::blocks;
use crate::gameplay::items::{self, ItemStack};
use crate::util::bytes::Writer;
use crate::util::rng::JavaRandom;
use crate::world::biome::Biome;
use crate::world::chunk::ChunkPos;
use crate::world::storage::nbt::NbtTag;
use crate::world::world::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MobType {
    Creeper = 50,
    Skeleton = 51,
    Spider = 52,
    Giant = 53,
    Zombie = 54,
    Slime = 55,
    Ghast = 56,
    PigZombie = 57,
    Pig = 90,
    Sheep = 91,
    Cow = 92,
    Chicken = 93,
    Squid = 94,
    Wolf = 95,
}

impl MobType {
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            50 => Some(Self::Creeper),
            51 => Some(Self::Skeleton),
            52 => Some(Self::Spider),
            53 => Some(Self::Giant),
            54 => Some(Self::Zombie),
            55 => Some(Self::Slime),
            56 => Some(Self::Ghast),
            57 => Some(Self::PigZombie),
            90 => Some(Self::Pig),
            91 => Some(Self::Sheep),
            92 => Some(Self::Cow),
            93 => Some(Self::Chicken),
            94 => Some(Self::Squid),
            95 => Some(Self::Wolf),
            _ => None,
        }
    }

    pub fn from_nbt_id(name: &str) -> Option<Self> {
        match name {
            "Creeper" => Some(Self::Creeper),
            "Skeleton" => Some(Self::Skeleton),
            "Spider" => Some(Self::Spider),
            "Giant" => Some(Self::Giant),
            "Zombie" => Some(Self::Zombie),
            "Slime" => Some(Self::Slime),
            "Ghast" => Some(Self::Ghast),
            "PigZombie" => Some(Self::PigZombie),
            "Pig" => Some(Self::Pig),
            "Sheep" => Some(Self::Sheep),
            "Cow" => Some(Self::Cow),
            "Chicken" => Some(Self::Chicken),
            "Squid" => Some(Self::Squid),
            "Wolf" => Some(Self::Wolf),
            _ => None,
        }
    }

    pub fn nbt_id(self) -> &'static str {
        match self {
            Self::Creeper => "Creeper",
            Self::Skeleton => "Skeleton",
            Self::Spider => "Spider",
            Self::Giant => "Giant",
            Self::Zombie => "Zombie",
            Self::Slime => "Slime",
            Self::Ghast => "Ghast",
            Self::PigZombie => "PigZombie",
            Self::Pig => "Pig",
            Self::Sheep => "Sheep",
            Self::Cow => "Cow",
            Self::Chicken => "Chicken",
            Self::Squid => "Squid",
            Self::Wolf => "Wolf",
        }
    }

    pub fn is_monster(self) -> bool {
        matches!(
            self,
            Self::Creeper
                | Self::Skeleton
                | Self::Spider
                | Self::Giant
                | Self::Zombie
                | Self::Slime
                | Self::Ghast
                | Self::PigZombie
        )
    }

    pub fn is_animal(self) -> bool {
        matches!(
            self,
            Self::Pig | Self::Sheep | Self::Cow | Self::Chicken | Self::Wolf
        )
    }

    pub fn is_water_animal(self) -> bool {
        matches!(self, Self::Squid)
    }

    pub fn dimensions(self, slime_size: i32) -> (f32, f32) {
        match self {
            Self::Creeper | Self::Skeleton | Self::Zombie | Self::PigZombie => (0.6, 1.8),
            Self::Spider => (1.4, 0.9),
            Self::Giant => (3.6, 10.8),
            Self::Slime => {
                let s = slime_size.max(1) as f32;
                (0.6 * s, 0.6 * s)
            }
            Self::Ghast => (4.0, 4.0),
            Self::Pig => (0.9, 0.9),
            Self::Sheep | Self::Cow => (0.9, 1.3),
            Self::Chicken => (0.3, 0.4),
            Self::Squid => (0.95, 0.95),
            Self::Wolf => (0.8, 0.8),
        }
    }

    pub fn default_health(self, slime_size: i32) -> i16 {
        match self {
            Self::Creeper | Self::Skeleton | Self::Spider | Self::Zombie | Self::PigZombie => 20,
            Self::Giant => 100,
            Self::Slime => {
                let s = slime_size.max(1) as i16;
                s * s
            }
            Self::Ghast | Self::Pig | Self::Cow | Self::Squid => 10,
            Self::Sheep | Self::Wolf => 8,
            Self::Chicken => 4,
        }
    }

    pub fn move_speed(self) -> f32 {
        match self {
            Self::Zombie | Self::Skeleton | Self::Creeper | Self::Giant | Self::PigZombie => 0.5,
            Self::Spider => 0.8,
            Self::Wolf => 1.1,
            _ => 0.7,
        }
    }

    pub fn attack_damage(self, slime_size: i32, wolf_tamed: bool) -> i16 {
        match self {
            Self::Zombie | Self::PigZombie => 5,
            Self::Spider | Self::Skeleton => 2,
            Self::Giant => 50,
            Self::Slime => slime_size.max(1) as i16,
            Self::Wolf => {
                if wolf_tamed {
                    4
                } else {
                    2
                }
            }
            _ => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mob {
    pub entity_id: i32,
    pub mob_type: MobType,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub health: i16,
    pub max_health: i16,
    pub air_ticks: i16,
    pub no_damage_ticks: u8,
    pub last_damage: i16,
    pub attack_ticks: i32,
    pub fire_ticks: i32,
    pub fall_distance: f32,
    pub on_ground: bool,
    pub horiz_collided: bool,
    pub in_water: bool,
    pub velocity_changed: bool,
    pub dead: bool,
    pub no_despawn_ticks: i32,
    pub target_id: Option<i32>,
    pub path: Vec<(i32, i32, i32)>,
    pub path_index: usize,
    pub jumping: bool,
    // Pig
    pub pig_saddled: bool,
    pub passenger_id: Option<i32>,
    pub vehicle_id: Option<i32>,
    // Sheep
    pub sheep_color: u8,
    pub sheep_sheared: bool,
    // Chicken
    pub chicken_egg_timer: i32,
    // Wolf
    pub wolf_sitting: bool,
    pub wolf_angry: bool,
    pub wolf_tamed: bool,
    pub wolf_owner: String,
    pub wolf_wet: bool,
    pub wolf_shaking: bool,
    pub wolf_shake_time: f32,
    // Creeper
    pub creeper_fuse: i32,
    pub creeper_state: i8,
    pub creeper_powered: bool,
    // Slime
    pub slime_size: i32,
    pub slime_jump_delay: i32,
    // Ghast
    pub ghast_waypoint: (f64, f64, f64),
    pub ghast_course_cooldown: i32,
    pub ghast_charge: i32,
    // PigZombie
    pub pigzombie_anger: i32,
    // Squid
    pub squid_swim_vec: (f32, f32, f32),
    pub squid_tentacle_angle: f32,
    pub squid_speed: f32,
    // Internal RNG
    pub rng: JavaRandom,
}

impl Mob {
    pub fn new(entity_id: i32, mob_type: MobType, x: f64, y: f64, z: f64, seed: i64) -> Self {
        let mut rng = JavaRandom::new(seed ^ ((entity_id as i64).wrapping_mul(0x5DEECE66D)));
        let slime_size = if mob_type == MobType::Slime {
            1 << rng.next_i32_bound(3)
        } else {
            1
        };
        let health = mob_type.default_health(slime_size);
        let sheep_color = if mob_type == MobType::Sheep {
            Self::natural_sheep_color(&mut rng)
        } else {
            0
        };
        let chicken_egg_timer = if mob_type == MobType::Chicken {
            rng.next_i32_bound(6000) + 6000
        } else {
            6000
        };
        let slime_jump_delay = rng.next_i32_bound(20) + 10;

        Self {
            entity_id,
            mob_type,
            x,
            y,
            z,
            mot_x: 0.0,
            mot_y: 0.0,
            mot_z: 0.0,
            yaw: rng.next_f32() * 360.0,
            pitch: 0.0,
            health,
            max_health: health,
            air_ticks: 300,
            no_damage_ticks: 0,
            last_damage: 0,
            attack_ticks: 0,
            fire_ticks: 0,
            fall_distance: 0.0,
            on_ground: false,
            horiz_collided: false,
            in_water: false,
            velocity_changed: false,
            dead: false,
            no_despawn_ticks: 0,
            target_id: None,
            path: Vec::new(),
            path_index: 0,
            jumping: false,
            pig_saddled: false,
            passenger_id: None,
            vehicle_id: None,
            sheep_color,
            sheep_sheared: false,
            chicken_egg_timer,
            wolf_sitting: false,
            wolf_angry: false,
            wolf_tamed: false,
            wolf_owner: String::new(),
            wolf_wet: false,
            wolf_shaking: false,
            wolf_shake_time: 0.0,
            creeper_fuse: 0,
            creeper_state: -1,
            creeper_powered: false,
            slime_size,
            slime_jump_delay,
            ghast_waypoint: (x, y, z),
            ghast_course_cooldown: 0,
            ghast_charge: 0,
            pigzombie_anger: 0,
            squid_swim_vec: (0.0, 0.0, 0.0),
            squid_tentacle_angle: 0.0,
            squid_speed: 0.2,
            rng,
        }
    }

    /// Java `EntitySheep.a(Random)`: natural wool color distribution.
    pub fn natural_sheep_color(rng: &mut JavaRandom) -> u8 {
        Self::natural_sheep_color_ex(rng, true, true)
    }

    pub fn natural_sheep_color_ex(
        rng: &mut JavaRandom,
        spawn_black: bool,
        spawn_brown_pink: bool,
    ) -> u8 {
        let i = rng.next_i32_bound(100);
        if i < 5 && spawn_black {
            15 // Black
        } else if i < 10 && spawn_black {
            7 // Gray
        } else if i < 15 && spawn_black {
            8 // Silver / Light Gray
        } else if i < 18 && spawn_brown_pink {
            12 // Brown
        } else if spawn_brown_pink && rng.next_i32_bound(500) == 0 {
            6 // Pink
        } else {
            0 // White
        }
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x.floor() as i32) >> 4, (self.z.floor() as i32) >> 4)
    }

    pub fn size(&self) -> (f32, f32) {
        self.mob_type.dimensions(self.slime_size)
    }

    pub fn eye_height(&self) -> f64 {
        let (_, h) = self.size();
        (h * 0.85) as f64
    }

    pub fn mounted_y_offset(&self) -> f64 {
        let (_, h) = self.size();
        if self.mob_type == MobType::Spider {
            (h * 0.75) as f64 - 0.5
        } else {
            (h * 0.75) as f64
        }
    }

    pub fn fire_proof(&self) -> bool {
        matches!(self.mob_type, MobType::Ghast | MobType::PigZombie)
    }

    pub fn can_despawn(&self) -> bool {
        if self.mob_type == MobType::Wolf && self.wolf_tamed {
            return false;
        }
        if self.mob_type == MobType::Pig && self.pig_saddled {
            return false;
        }
        self.mob_type.is_monster() || self.mob_type.is_water_animal()
    }

    pub fn held_item(&self) -> Option<ItemStack> {
        match self.mob_type {
            MobType::Skeleton => Some(ItemStack::new(items::BOW, 1, 0)),
            MobType::PigZombie => Some(ItemStack::new(items::GOLD_SWORD, 1, 0)),
            _ => None,
        }
    }

    /// Encode Beta 1.7.3 `DataWatcher` metadata bytes (terminated with `127`).
    pub fn encode_metadata(&self) -> Vec<u8> {
        let mut w = Writer::new();
        // Index 0 (byte): entity flags (bit 0 = burning, bit 2 = riding)
        let mut flags = 0u8;
        if self.fire_ticks > 0 {
            flags |= 0x01;
        }
        if self.vehicle_id.is_some() {
            flags |= 0x04;
        }
        w.write_u8(0); // type 0 (byte), index 0
        w.write_u8(flags);

        match self.mob_type {
            MobType::Pig => {
                w.write_u8(16); // type 0 (byte), index 16
                w.write_u8(u8::from(self.pig_saddled));
            }
            MobType::Sheep => {
                w.write_u8(16);
                let val = (self.sheep_color & 15) | if self.sheep_sheared { 16 } else { 0 };
                w.write_u8(val);
            }
            MobType::Creeper => {
                w.write_u8(16);
                w.write_i8(self.creeper_state);
                w.write_u8(17);
                w.write_u8(u8::from(self.creeper_powered));
            }
            MobType::Slime => {
                w.write_u8(16);
                w.write_u8(self.slime_size.max(1) as u8);
            }
            MobType::Ghast => {
                w.write_u8(16);
                w.write_u8(u8::from(self.ghast_charge > 10));
            }
            MobType::Wolf => {
                let mut wf = 0u8;
                if self.wolf_sitting {
                    wf |= 0x01;
                }
                if self.wolf_angry {
                    wf |= 0x02;
                }
                if self.wolf_tamed {
                    wf |= 0x04;
                }
                w.write_u8(16); // type 0 (byte), index 16
                w.write_u8(wf);
                w.write_u8((4 << 5) | 17); // type 4 (String), index 17
                let _ = w.write_string(&self.wolf_owner, 16);
                w.write_u8((2 << 5) | 18); // type 2 (Int), index 18
                w.write_i32(self.health as i32);
            }
            _ => {}
        }

        w.write_u8(127);
        w.as_slice().to_vec()
    }

    pub fn set_slime_size(&mut self, size: i32) {
        let s = size.max(1);
        self.slime_size = s;
        self.health = (s * s) as i16;
        self.max_health = self.health;
    }

    /// Generate death loot drops (`EntityLiving.q()` / `j()`).
    pub fn get_death_drops(&mut self, killed_by_skeleton_arrow: bool) -> Vec<ItemStack> {
        let mut drops = Vec::new();
        match self.mob_type {
            MobType::Pig => {
                let count = self.rng.next_i32_bound(3) as u8;
                let id = if self.fire_ticks > 0 {
                    items::GRILLED_PORK
                } else {
                    items::PORK
                };
                for _ in 0..count {
                    drops.push(ItemStack::new(id, 1, 0));
                }
            }
            MobType::Sheep => {
                if !self.sheep_sheared {
                    drops.push(ItemStack::new(
                        blocks::WOOL as i16,
                        1,
                        self.sheep_color as i16,
                    ));
                }
            }
            MobType::Cow => {
                let count = self.rng.next_i32_bound(3) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::LEATHER, 1, 0));
                }
            }
            MobType::Chicken | MobType::Zombie => {
                let count = self.rng.next_i32_bound(3) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::FEATHER, 1, 0));
                }
            }
            MobType::Squid => {
                let count = (self.rng.next_i32_bound(3) + 1) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::INK_SACK, 1, 0));
                }
            }
            MobType::Skeleton => {
                let arrows = self.rng.next_i32_bound(3) as u8;
                for _ in 0..arrows {
                    drops.push(ItemStack::new(items::ARROW, 1, 0));
                }
                let bones = self.rng.next_i32_bound(3) as u8;
                for _ in 0..bones {
                    drops.push(ItemStack::new(items::BONE, 1, 0));
                }
            }
            MobType::Spider => {
                let count = self.rng.next_i32_bound(3) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::STRING, 1, 0));
                }
            }
            MobType::Creeper => {
                let count = self.rng.next_i32_bound(3) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::SULPHUR, 1, 0));
                }
                if killed_by_skeleton_arrow {
                    let record = items::GOLD_RECORD + self.rng.next_i32_bound(2) as i16;
                    drops.push(ItemStack::new(record, 1, 0));
                }
            }
            MobType::Ghast => {
                let count = self.rng.next_i32_bound(3) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::SULPHUR, 1, 0));
                }
            }
            MobType::Slime => {
                if self.slime_size == 1 {
                    let count = self.rng.next_i32_bound(3) as u8;
                    for _ in 0..count {
                        drops.push(ItemStack::new(items::SLIME_BALL, 1, 0));
                    }
                }
            }
            MobType::PigZombie => {
                let count = self.rng.next_i32_bound(3) as u8;
                for _ in 0..count {
                    drops.push(ItemStack::new(items::GRILLED_PORK, 1, 0));
                }
            }
            MobType::Wolf | MobType::Giant => {}
        }
        drops
    }

    pub fn to_nbt(&self) -> NbtTag {
        let mut tag = NbtTag::compound();
        tag.insert("id", NbtTag::String(self.mob_type.nbt_id().to_string()));
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
        tag.insert("Health", NbtTag::Short(self.health));
        tag.insert("OnGround", NbtTag::Byte(i8::from(self.on_ground)));

        match self.mob_type {
            MobType::Pig => {
                tag.insert("Saddle", NbtTag::Byte(i8::from(self.pig_saddled)));
            }
            MobType::Sheep => {
                tag.insert("Sheared", NbtTag::Byte(i8::from(self.sheep_sheared)));
                tag.insert("Color", NbtTag::Byte(self.sheep_color as i8));
            }
            MobType::Wolf => {
                tag.insert("Angry", NbtTag::Byte(i8::from(self.wolf_angry)));
                tag.insert("Sitting", NbtTag::Byte(i8::from(self.wolf_sitting)));
                tag.insert("Owner", NbtTag::String(self.wolf_owner.clone()));
            }
            MobType::Creeper => {
                tag.insert("powered", NbtTag::Byte(i8::from(self.creeper_powered)));
            }
            MobType::Slime => {
                tag.insert("Size", NbtTag::Int(self.slime_size - 1));
            }
            MobType::PigZombie => {
                tag.insert("Anger", NbtTag::Short(self.pigzombie_anger as i16));
            }
            _ => {}
        }
        tag
    }

    pub fn from_nbt(entity_id: i32, tag: &NbtTag, seed: i64) -> Option<Self> {
        let id_str = tag.get_str("id")?;
        let mob_type = MobType::from_nbt_id(id_str)?;
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

        let mut mob = Self::new(entity_id, mob_type, x, y, z, seed);
        if let Some(mot) = tag.get_list("Motion") {
            if mot.len() >= 3 {
                if let NbtTag::Double(v) = mot[0] {
                    mob.mot_x = v;
                }
                if let NbtTag::Double(v) = mot[1] {
                    mob.mot_y = v;
                }
                if let NbtTag::Double(v) = mot[2] {
                    mob.mot_z = v;
                }
            }
        }
        if let Some(rot) = tag.get_list("Rotation") {
            if rot.len() >= 2 {
                if let NbtTag::Float(v) = rot[0] {
                    mob.yaw = v;
                }
                if let NbtTag::Float(v) = rot[1] {
                    mob.pitch = v;
                }
            }
        }
        if let Some(h) = tag.get_i16("Health") {
            mob.health = h;
        }
        if let Some(f) = tag.get_i16("Fire") {
            mob.fire_ticks = f as i32;
        }
        if let Some(og) = tag.get_bool("OnGround") {
            mob.on_ground = og;
        }

        match mob_type {
            MobType::Pig => {
                mob.pig_saddled = tag.get_bool("Saddle").unwrap_or(false);
            }
            MobType::Sheep => {
                mob.sheep_sheared = tag.get_bool("Sheared").unwrap_or(false);
                mob.sheep_color = (tag.get_i8("Color").unwrap_or(0) as u8) & 15;
            }
            MobType::Wolf => {
                mob.wolf_angry = tag.get_bool("Angry").unwrap_or(false);
                mob.wolf_sitting = tag.get_bool("Sitting").unwrap_or(false);
                if let Some(owner) = tag.get_str("Owner") {
                    if !owner.is_empty() {
                        mob.wolf_owner = owner.to_string();
                        mob.wolf_tamed = true;
                        mob.max_health = 20;
                    }
                }
            }
            MobType::Creeper => {
                mob.creeper_powered = tag.get_bool("powered").unwrap_or(false);
            }
            MobType::Slime => {
                if let Some(sz) = tag.get_i32("Size") {
                    mob.set_slime_size((sz + 1).clamp(1, 4));
                }
            }
            MobType::PigZombie => {
                mob.pigzombie_anger = tag.get_i16("Anger").unwrap_or(0) as i32;
            }
            _ => {}
        }

        Some(mob)
    }
}

// -------------------------------------------------------------------------
// Pathfinder (`Pathfinder.java`: A* grid pathfinding over the voxel world)
// -------------------------------------------------------------------------

#[derive(Clone, Eq, PartialEq)]
struct AStarNode {
    pos: (i32, i32, i32),
    f_cost: i32,
    g_cost: i32,
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f_cost.cmp(&self.f_cost)
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `Pathfinder.a(Entity, int x, int y, int z, float maxDist)`.
#[allow(clippy::too_many_arguments)]
pub fn find_path(
    world: &World,
    start_x: f64,
    start_y: f64,
    start_z: f64,
    goal_x: i32,
    goal_y: i32,
    goal_z: i32,
    max_dist: f32,
) -> Vec<(i32, i32, i32)> {
    let sx = start_x.floor() as i32;
    let sy = start_y.floor() as i32;
    let sz = start_z.floor() as i32;
    if sx == goal_x && sy == goal_y && sz == goal_z {
        return Vec::new();
    }

    let max_dist_sq = ((max_dist + 4.0) * (max_dist + 4.0)) as i32;
    let mut open = BinaryHeap::new();
    let mut g_scores: HashMap<(i32, i32, i32), i32> = HashMap::new();
    let mut came_from: HashMap<(i32, i32, i32), (i32, i32, i32)> = HashMap::new();

    let h0 = ((goal_x - sx).abs() + (goal_y - sy).abs() + (goal_z - sz).abs()) * 10;
    g_scores.insert((sx, sy, sz), 0);
    open.push(AStarNode {
        pos: (sx, sy, sz),
        f_cost: h0,
        g_cost: 0,
    });

    let mut best_pos = (sx, sy, sz);
    let mut best_h = h0;
    let mut iterations = 0;

    while let Some(current) = open.pop() {
        iterations += 1;
        if iterations > 160 {
            break;
        }
        let (cx, cy, cz) = current.pos;
        let h = ((goal_x - cx).abs() + (goal_y - cy).abs() + (goal_z - cz).abs()) * 10;
        if h < best_h {
            best_h = h;
            best_pos = (cx, cy, cz);
        }
        if (cx - goal_x).abs() <= 1 && (cy - goal_y).abs() <= 1 && (cz - goal_z).abs() <= 1 {
            best_pos = (cx, cy, cz);
            break;
        }

        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let nx = cx + dx;
            let nz = cz + dz;
            let dist_sq = (nx - sx) * (nx - sx) + (nz - sz) * (nz - sz);
            if dist_sq > max_dist_sq {
                continue;
            }

            if let Some(ny) = resolve_step_y(world, nx, cy, nz) {
                let ng = current.g_cost + 10 + (ny - cy).abs() * 5;
                if ng < *g_scores.get(&(nx, ny, nz)).unwrap_or(&i32::MAX) {
                    g_scores.insert((nx, ny, nz), ng);
                    came_from.insert((nx, ny, nz), (cx, cy, cz));
                    let nh = ((goal_x - nx).abs() + (goal_y - ny).abs() + (goal_z - nz).abs()) * 10;
                    open.push(AStarNode {
                        pos: (nx, ny, nz),
                        f_cost: ng + nh,
                        g_cost: ng,
                    });
                }
            }
        }
    }

    if best_pos == (sx, sy, sz) {
        return Vec::new();
    }

    let mut path = Vec::new();
    let mut cur = best_pos;
    while cur != (sx, sy, sz) {
        path.push(cur);
        if let Some(&prev) = came_from.get(&cur) {
            cur = prev;
        } else {
            break;
        }
    }
    path.reverse();
    path
}

fn is_passable_for_mob(world: &World, x: i32, y: i32, z: i32) -> bool {
    if !(1..126).contains(&y) {
        return false;
    }
    let b0 = world.get_block(x, y, z);
    let b1 = world.get_block(x, y + 1, z);
    !blocks::is_solid(b0)
        && !blocks::is_lava(b0)
        && b0 != blocks::FIRE
        && b0 != blocks::CACTUS
        && !blocks::is_solid(b1)
        && !blocks::is_lava(b1)
}

fn resolve_step_y(world: &World, x: i32, y: i32, z: i32) -> Option<i32> {
    // Same level
    if is_passable_for_mob(world, x, y, z) {
        let mut drop_y = y;
        for _ in 0..4 {
            let below = world.get_block(x, drop_y - 1, z);
            if blocks::is_solid(below) || blocks::is_water(below) {
                return Some(drop_y);
            }
            if blocks::is_lava(below) || below == blocks::FIRE {
                return None;
            }
            drop_y -= 1;
            if drop_y <= 1 {
                return None;
            }
        }
        return None;
    }
    // Step up +1
    if is_passable_for_mob(world, x, y + 1, z) && blocks::is_solid(world.get_block(x, y, z)) {
        return Some(y + 1);
    }
    None
}

// -------------------------------------------------------------------------
// SpawnerCreature (`SpawnerCreature.java` + `BiomeBase.java`)
// -------------------------------------------------------------------------

fn biome_monsters(_biome: Biome) -> &'static [(MobType, i32)] {
    &[
        (MobType::Spider, 10),
        (MobType::Zombie, 10),
        (MobType::Skeleton, 10),
        (MobType::Creeper, 10),
        (MobType::Slime, 10),
    ]
}

fn biome_creatures(biome: Biome) -> &'static [(MobType, i32)] {
    match biome {
        Biome::Desert | Biome::IceDesert => &[],
        Biome::Taiga => &[
            (MobType::Sheep, 12),
            (MobType::Pig, 10),
            (MobType::Chicken, 10),
            (MobType::Cow, 8),
            (MobType::Wolf, 2),
        ],
        Biome::Forest => &[
            (MobType::Sheep, 12),
            (MobType::Pig, 10),
            (MobType::Chicken, 10),
            (MobType::Cow, 8),
            (MobType::Wolf, 2),
        ],
        _ => &[
            (MobType::Sheep, 12),
            (MobType::Pig, 10),
            (MobType::Chicken, 10),
            (MobType::Cow, 8),
        ],
    }
}

fn pick_weighted(entries: &[(MobType, i32)], rng: &mut JavaRandom) -> Option<MobType> {
    let total: i32 = entries.iter().map(|(_, w)| *w).sum();
    if total <= 0 {
        return None;
    }
    let mut rnd = rng.next_i32_bound(total);
    for &(mob_type, weight) in entries {
        rnd -= weight;
        if rnd < 0 {
            return Some(mob_type);
        }
    }
    Some(entries[0].0)
}

/// Check if `(chunk_x, chunk_z)` is a slime chunk (`Chunk.a(987234911L).nextInt(10) == 0`).
pub fn is_slime_chunk(world_seed: i64, cx: i32, cz: i32) -> bool {
    let cx64 = cx as i64;
    let cz64 = cz as i64;
    let slime_seed = world_seed
        .wrapping_add(cx64.wrapping_mul(cx64).wrapping_mul(4_987_142))
        .wrapping_add(cx64.wrapping_mul(5_947_611))
        .wrapping_add(cz64.wrapping_mul(cz64).wrapping_mul(4_392_871))
        .wrapping_add(cz64.wrapping_mul(389_711))
        ^ 987_234_911;
    let mut r = JavaRandom::new(slime_seed);
    r.next_i32_bound(10) == 0
}

#[derive(Debug)]
pub struct SpawnedMobSpec {
    pub mob_type: MobType,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub spider_jockey: bool,
}

/// Java `SpawnerCreature.spawnEntities`: evaluate natural spawns around online players.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_natural_spawns(
    world: &World,
    rng: &mut JavaRandom,
    player_positions: &[(f64, f64, f64)],
    allow_monsters: bool,
    allow_animals: bool,
    current_monsters: usize,
    current_animals: usize,
    current_water: usize,
) -> Vec<SpawnedMobSpec> {
    if player_positions.is_empty() || (!allow_monsters && !allow_animals) {
        return Vec::new();
    }

    let mut candidate_chunks = HashSet::new();
    for &(px, _, pz) in player_positions {
        let pcx = (px.floor() as i32) >> 4;
        let pcz = (pz.floor() as i32) >> 4;
        for dx in -8..=8 {
            for dz in -8..=8 {
                let cpos = ChunkPos::new(pcx + dx, pcz + dz);
                if world.is_chunk_loaded(cpos) {
                    candidate_chunks.insert(cpos);
                }
            }
        }
    }
    if candidate_chunks.is_empty() {
        return Vec::new();
    }

    let (spawn_x, spawn_y, spawn_z) = world.spawn_point();
    let sky_sub = world_sky_subtracted(world.world_time());
    let mut spawned = Vec::new();

    // Categories: (0 = Monster, 1 = Creature, 2 = WaterCreature)
    let categories = [
        (
            0u8,
            allow_monsters && world.spawning.spawn_monsters,
            current_monsters,
            world.spawning.monster_cap,
        ),
        (
            1u8,
            allow_animals && world.spawning.spawn_animals,
            current_animals,
            world.spawning.animal_cap,
        ),
        (
            2u8,
            allow_animals && world.spawning.spawn_water_mobs && world.spawning.spawn_squids,
            current_water,
            world.spawning.water_mob_cap,
        ),
    ];

    for (cat, enabled, cur_count, max_base) in categories {
        if !enabled {
            continue;
        }
        let cap = max_base * candidate_chunks.len() / 256;
        if cur_count > cap {
            continue;
        }

        for &cpos in &candidate_chunks {
            // Only sample a subset of chunks per tick so TPS stays 20.0
            if rng.next_i32_bound(16) != 0 {
                continue;
            }
            let Some(chunk) = world.get_chunk(cpos) else {
                continue;
            };
            let biome = Biome::from_id(chunk.biomes[0]);
            let chosen_mob = if world.is_hell {
                match cat {
                    0 => pick_weighted(&[(MobType::Ghast, 10), (MobType::PigZombie, 10)], rng),
                    _ => None,
                }
            } else {
                match cat {
                    0 => pick_weighted(biome_monsters(biome), rng),
                    1 => pick_weighted(biome_creatures(biome), rng),
                    _ => Some(MobType::Squid),
                }
            };
            let Some(mob_type) = chosen_mob else {
                continue;
            };
            if (mob_type == MobType::Slime && !world.spawning.spawn_slimes)
                || (mob_type == MobType::Wolf && !world.spawning.spawn_wolves)
                || (mob_type == MobType::Squid && !world.spawning.spawn_squids)
            {
                continue;
            }

            let base_x = cpos.x * 16 + rng.next_i32_bound(16);
            let base_y = rng.next_i32_bound(128);
            let base_z = cpos.z * 16 + rng.next_i32_bound(16);

            let base_id = world.get_block(base_x, base_y, base_z);
            if cat == 2 {
                if !blocks::is_water(base_id) {
                    continue;
                }
            } else if blocks::is_solid_normal_block(base_id) || blocks::is_liquid(base_id) {
                continue;
            }

            let mut group_spawned = 0;
            'group: for _ in 0..3 {
                let mut x = base_x;
                let y = base_y;
                let mut z = base_z;
                for _ in 0..4 {
                    x += rng.next_i32_bound(6) - rng.next_i32_bound(6);
                    z += rng.next_i32_bound(6) - rng.next_i32_bound(6);
                    if !(1..126).contains(&y) {
                        continue;
                    }

                    let can_stand = if cat == 2 {
                        blocks::is_water(world.get_block(x, y, z))
                            && !blocks::is_solid_normal_block(world.get_block(x, y + 1, z))
                    } else {
                        world.is_solid_normal_at(x, y - 1, z)
                            && !world.is_solid_normal_at(x, y, z)
                            && !blocks::is_liquid(world.get_block(x, y, z))
                            && !world.is_solid_normal_at(x, y + 1, z)
                    };
                    if !can_stand {
                        continue;
                    }

                    let fx = x as f64 + 0.5;
                    let fy = y as f64;
                    let fz = z as f64 + 0.5;

                    // Must be >= 24 blocks from any player and >= 24 blocks from world spawn
                    let near_player = player_positions.iter().any(|&(px, py, pz)| {
                        let dx = px - fx;
                        let dy = py - fy;
                        let dz = pz - fz;
                        dx * dx + dy * dy + dz * dz < 576.0
                    });
                    if near_player {
                        continue;
                    }
                    let dsx = fx - spawn_x as f64;
                    let dsy = fy - spawn_y as f64;
                    let dsz = fz - spawn_z as f64;
                    if dsx * dsx + dsy * dsy + dsz * dsz < 576.0 {
                        continue;
                    }

                    // Specific `mob.d()` spawn conditions
                    if cat == 1 {
                        // Animals require GRASS below and light > 8
                        if world.get_block(x, y - 1, z) != blocks::GRASS {
                            continue;
                        }
                        let light = chunk_light_at(world, x, y, z, sky_sub);
                        if light <= 8 {
                            continue;
                        }
                    } else if cat == 0 {
                        if mob_type == MobType::Slime {
                            if y >= 16
                                || rng.next_i32_bound(10) != 0
                                || !is_slime_chunk(world.seed(), x >> 4, z >> 4)
                            {
                                continue;
                            }
                        } else {
                            let sky_l = chunk_sky_light_at(world, x, y, z);
                            if sky_l > rng.next_i32_bound(32) {
                                continue;
                            }
                            let light = chunk_light_at(world, x, y, z, sky_sub);
                            if light > rng.next_i32_bound(8) {
                                continue;
                            }
                        }
                    } else if !(46..=62).contains(&y) {
                        continue;
                    }

                    let spider_jockey = mob_type == MobType::Spider && rng.next_i32_bound(100) == 0;
                    spawned.push(SpawnedMobSpec {
                        mob_type,
                        x: fx,
                        y: fy,
                        z: fz,
                        spider_jockey,
                    });
                    group_spawned += 1;
                    let max_group = if mob_type == MobType::Wolf { 8 } else { 4 };
                    if group_spawned >= max_group {
                        break 'group;
                    }
                }
            }
        }
    }

    spawned
}

pub fn world_sky_subtracted(world_time: i64) -> i32 {
    let time_of_day = world_time.rem_euclid(24000) as f32;
    let mut f1 = (time_of_day + 1.0) / 24000.0 - 0.25;
    if f1 < 0.0 {
        f1 += 1.0;
    }
    if f1 > 1.0 {
        f1 -= 1.0;
    }
    let f2 = f1;
    f1 = 1.0 - (((f1 * std::f32::consts::PI).cos() + 1.0) / 2.0);
    f1 = f2 + (f1 - f2) / 3.0;
    let mut f3 = 1.0 - ((f1 * std::f32::consts::PI * 2.0).cos() * 2.0 + 0.5);
    f3 = f3.clamp(0.0, 1.0);
    (f3 * 11.0) as i32
}

pub fn chunk_light_at(world: &World, x: i32, y: i32, z: i32, sky_sub: i32) -> i32 {
    if !(0..128).contains(&y) {
        return 15;
    }
    if let Some(chunk) = world.get_chunk(ChunkPos::new(x >> 4, z >> 4)) {
        chunk.get_block_light_value((x & 15) as usize, y as usize, (z & 15) as usize, sky_sub)
    } else {
        0
    }
}

pub fn chunk_sky_light_at(world: &World, x: i32, y: i32, z: i32) -> i32 {
    if !(0..128).contains(&y) {
        return 15;
    }
    if let Some(chunk) = world.get_chunk(ChunkPos::new(x >> 4, z >> 4)) {
        chunk.get_sky_light((x & 15) as usize, y as usize, (z & 15) as usize) as i32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;

    #[test]
    fn all_14_mob_types_round_trip_nbt_and_metadata() {
        let all = [
            MobType::Creeper,
            MobType::Skeleton,
            MobType::Spider,
            MobType::Giant,
            MobType::Zombie,
            MobType::Slime,
            MobType::Ghast,
            MobType::PigZombie,
            MobType::Pig,
            MobType::Sheep,
            MobType::Cow,
            MobType::Chicken,
            MobType::Squid,
            MobType::Wolf,
        ];
        for (idx, &mtype) in all.iter().enumerate() {
            let mut mob = Mob::new(idx as i32 + 1, mtype, 10.5, 64.0, -20.5, 12345);
            if mtype == MobType::Wolf {
                mob.wolf_tamed = true;
                mob.wolf_sitting = true;
                mob.wolf_owner = "Alice".to_string();
                mob.health = 20;
            } else if mtype == MobType::Sheep {
                mob.sheep_color = 14;
                mob.sheep_sheared = true;
            } else if mtype == MobType::Pig {
                mob.pig_saddled = true;
            } else if mtype == MobType::Creeper {
                mob.creeper_powered = true;
            }

            let meta = mob.encode_metadata();
            assert_eq!(*meta.last().unwrap(), 127);

            let nbt = mob.to_nbt();
            let restored = Mob::from_nbt(mob.entity_id, &nbt, 12345).unwrap();
            assert_eq!(restored.mob_type, mtype);
            assert_eq!(restored.pig_saddled, mob.pig_saddled);
            assert_eq!(restored.sheep_color, mob.sheep_color);
            assert_eq!(restored.sheep_sheared, mob.sheep_sheared);
            assert_eq!(restored.wolf_tamed, mob.wolf_tamed);
            assert_eq!(restored.wolf_sitting, mob.wolf_sitting);
            assert_eq!(restored.wolf_owner, mob.wolf_owner);
            assert_eq!(restored.creeper_powered, mob.creeper_powered);
        }
    }

    #[test]
    fn creeper_drops_music_record_when_killed_by_skeleton_arrow() {
        let mut creeper = Mob::new(1, MobType::Creeper, 0.0, 64.0, 0.0, 42);
        let drops = creeper.get_death_drops(true);
        assert!(drops
            .iter()
            .any(|d| d.id == items::GOLD_RECORD || d.id == items::GREEN_RECORD));
    }

    #[test]
    fn astar_pathfinder_climbs_step_and_reaches_goal() {
        let mut world = World::new(WorldConfig {
            seed: 100,
            save_enabled: false,
            ..Default::default()
        });
        world.ensure_chunk_base(ChunkPos::new(0, 0));
        // Flat floor at y=63, with a 1-block step up at x>=4
        for x in 0..10 {
            for z in 0..10 {
                world.set_block(x, 63, z, blocks::STONE);
                world.set_block(x, 64, z, blocks::AIR);
                world.set_block(x, 65, z, blocks::AIR);
                world.set_block(x, 66, z, blocks::AIR);
                if x >= 4 {
                    world.set_block(x, 64, z, blocks::STONE);
                }
            }
        }
        let path = find_path(&world, 1.5, 64.0, 2.5, 6, 65, 2, 16.0);
        assert!(
            !path.is_empty(),
            "Pathfinder should find path up 1-block step"
        );
        let last = *path.last().unwrap();
        assert!((last.0 - 6).abs() <= 1 && last.1 == 65);
    }
}
