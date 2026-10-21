//! 1:1 port of all 14 Minecraft Beta 1.7.3 Animals & Mobs, `DataWatcher` metadata,
//! `Pathfinder` navigation, and `SpawnerCreature` natural spawning from `.references/Project-Poseidon`.

use std::collections::{BTreeSet, HashMap};

use crate::config::{MobSpawnSettings, SpawningConfig};
use crate::entity::physics::{move_entity, Aabb};
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

    /// The spawn category of the mob (`EnumCreatureType`).
    pub fn category(self) -> MobCategory {
        if self.is_monster() {
            MobCategory::Monster
        } else if self.is_animal() {
            MobCategory::Creature
        } else {
            MobCategory::WaterCreature
        }
    }

    /// The name of the mob in the configuration file (`[spawning.mobs.<name>]`).
    pub fn config_name(self) -> &'static str {
        match self {
            Self::Creeper => "creeper",
            Self::Skeleton => "skeleton",
            Self::Spider => "spider",
            Self::Giant => "giant",
            Self::Zombie => "zombie",
            Self::Slime => "slime",
            Self::Ghast => "ghast",
            Self::PigZombie => "pig_zombie",
            Self::Pig => "pig",
            Self::Sheep => "sheep",
            Self::Cow => "cow",
            Self::Chicken => "chicken",
            Self::Squid => "squid",
            Self::Wolf => "wolf",
        }
    }

    /// Parses the name of the mob in the configuration file.
    pub fn from_config_name(name: &str) -> Option<Self> {
        Some(match name {
            "creeper" => Self::Creeper,
            "skeleton" => Self::Skeleton,
            "spider" => Self::Spider,
            "giant" => Self::Giant,
            "zombie" => Self::Zombie,
            "slime" => Self::Slime,
            "ghast" => Self::Ghast,
            "pig_zombie" => Self::PigZombie,
            "pig" => Self::Pig,
            "sheep" => Self::Sheep,
            "cow" => Self::Cow,
            "chicken" => Self::Chicken,
            "squid" => Self::Squid,
            "wolf" => Self::Wolf,
            _ => return None,
        })
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
            // `EntityGiantZombie`: the health of the monster is ten times
            // higher.
            Self::Giant => 200,
            Self::Slime => {
                let s = slime_size.max(1) as i16;
                s * s
            }
            Self::Ghast | Self::Pig | Self::Cow | Self::Squid => 10,
            // `EntitySheep` does not set the health, the default of
            // `EntityLiving` is ten. The wolf has eight (`EntityWolf`).
            Self::Sheep => 10,
            Self::Wolf => 8,
            Self::Chicken => 4,
        }
    }

    /// Java `EntityLiving.aE`: the speed of the movement of the mob.
    ///
    /// The default of the reference is 0.7, the zombie, the pig zombie and the
    /// giant have 0.5, the spider 0.8 and the wolf 1.1. The pig zombie is
    /// faster when it has a target (`EntityPigZombie.m_()`).
    pub fn move_speed(self, has_target: bool) -> f32 {
        match self {
            Self::Zombie | Self::Giant => 0.5,
            Self::PigZombie => {
                if has_target {
                    0.95
                } else {
                    0.5
                }
            }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobCategory {
    /// `EnumCreatureType.MONSTER`
    Monster,
    /// `EnumCreatureType.CREATURE`
    Creature,
    /// `EnumCreatureType.WATER_CREATURE`
    WaterCreature,
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
    /// Java `EntityLiving.ae`: the time of the red flash of the hurt. The
    /// mob has ten ticks of it after a hit (`EntityLiving.damageEntity`).
    pub hurt_ticks: i32,
    /// Java `EntityLiving.ac`: the health of the mob before the last hit.
    pub health_before_damage: i16,
    /// Java `EntityLiving.ao`: the time of the animation of the hit.
    pub attack_anim: f32,
    /// Java `EntityLiving.an`: the value of `ao` of the previous tick.
    pub attack_anim_prev: f32,
    /// Java `EntityLiving.ap`: the distance the mob walked, the client
    /// uses it for the legs of the mob.
    pub walk_dist: f32,
    pub attack_ticks: i32,
    pub fire_ticks: i32,
    pub fall_distance: f32,
    pub on_ground: bool,
    pub horiz_collided: bool,
    pub in_water: bool,
    /// Java `Entity.bA`: the mob is in the water.
    pub in_lava: bool,
    /// Java `Entity.bf`: the mob is in a web, the movement of the next
    /// tick is damped (`Entity.move()`).
    pub web: bool,
    /// Java `Entity.airBorne`: the mob was pushed, jumped or hurt, the
    /// tracker sends the packet of the position at once.
    pub air_borne: bool,
    /// Java `Entity.bm`: the distance the mob walked, the sound of the step
    /// is played when the distance passes the counter of the steps.
    pub walk_accum: f32,
    /// Java `Entity.b`: the counter of the steps of the mob.
    pub step_counter: i32,
    /// Java `Entity.positionChanged`: the last movement changed the position.
    pub position_changed: bool,
    /// Java `Entity.br`: the offset between the box of the mob and its
    /// position (the step up of the reference).
    pub step_offset: f32,
    /// Java `EntityLiving.az`: the sideways movement input.
    pub move_strafe: f32,
    /// Java `EntityLiving.aA`: the forward movement input.
    pub move_forward: f32,
    /// Java `EntityLiving.e`: the mob sees its target. The flag is reset at
    /// the beginning of the artificial intelligence (`EntityCreature.c_()`)
    /// and the attack of the creeper and of the skeleton sets it.
    pub sees_target: bool,
    /// Java `EntityLiving.aC`: the mob wants to jump.
    pub jump_flag: bool,
    /// Java `EntityLiving.aB`: the drift of the yaw while the mob looks
    /// around.
    pub yaw_drift: f32,
    /// Java `EntityLiving.a`: the counter of the idle sound of the mob.
    pub idle_sound_ticks: i32,
    /// Java `EntityLiving.b`: the player the mob looks at.
    pub look_target: Option<i32>,
    /// Java `EntityLiving.aF`: the ticks the mob looks at the player.
    pub look_ticks: i32,
    pub velocity_changed: bool,
    pub dead: bool,
    /// Java `Entity.dead`: the world removes the entity at the beginning of
    /// its next tick (`World.tickEntities`). The creeper that explodes dies
    /// this way, its health does not change.
    pub removed: bool,
    /// Java `EntityLiving.deathTicks`: the number of the ticks the corpse of
    /// the mob lives. The mob leaves the world when the counter passes
    /// twenty.
    pub death_ticks: i32,
    pub no_despawn_ticks: i32,
    pub target_id: Option<i32>,
    pub path: Vec<(i32, i32, i32)>,
    pub path_index: usize,
    // Pig
    pub pig_saddled: bool,
    pub passenger_id: Option<i32>,
    pub vehicle_id: Option<i32>,
    // Sheep
    pub sheep_color: u8,
    pub sheep_sheared: bool,
    // Chicken
    pub chicken_egg_timer: i32,
    /// Java `EntityChicken.b`: the angle of the wing.
    pub chicken_wing: f32,
    /// Java `EntityChicken.c`: the angle of the tip of the wing.
    pub chicken_wing_tip: f32,
    /// Java `EntityChicken.h`: the factor of the flap of the wings.
    pub chicken_flap: f32,
    // Wolf
    pub wolf_sitting: bool,
    pub wolf_angry: bool,
    pub wolf_tamed: bool,
    pub wolf_owner: String,
    /// Java `EntityWolf.f`: the wolf is wet ✓.
    pub wolf_wet: bool,
    /// Java `EntityWolf.g`: the wolf is shaking ✓.
    pub wolf_shaking: bool,
    /// Java `EntityWolf.h`: the time of the shake ✓.
    pub wolf_shake_time: f32,
    /// Java `EntityWolf.i`: the **previous** time of the shake ✓ — the shake
    /// ends when it reaches two seconds ✓ (`EntityWolf.java:170-176` ✓).
    pub wolf_prev_shake_time: f32,
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
    /// The counter of the search of the target of the ghast
    /// (`EntityGhast.h`): the search of the nearby player is repeated every
    /// twenty ticks.
    pub ghast_target_timer: i32,
    // PigZombie
    pub pigzombie_anger: i32,
    /// Java `EntityPigZombie.soundDelay`: the counter of the sound of the
    /// angry pig zombie.
    pub pigzombie_sound_delay: i32,
    // Squid
    pub squid_swim_vec: (f32, f32, f32),
    pub squid_tentacle_angle: f32,
    /// Java `EntitySquid.k`: the factor of the swim.
    pub squid_speed: f32,
    /// Java `EntitySquid.g`: the phase of the swim.
    pub squid_phase: f32,
    /// Java `EntitySquid.l`: the speed of the phase.
    pub squid_phase_speed: f32,
    /// Java `EntitySquid.m`: the squash of the tentacles.
    pub squid_squash: f32,
    /// Java `EntitySquid.K`: the yaw of the swim.
    pub squid_yaw: f32,
    /// Java `Entity.bA` of the previous tick: the mob was in the water.
    pub was_in_water: bool,
    /// Java `Entity.justCreated`: the mob was just created.
    pub just_created: bool,
    /// Java `EntityLiving.K`: the yaw the mob looks at.
    pub look_yaw: f32,
    /// Java `Entity.lastX`: the position of the mob in the previous tick.
    pub last_x: f64,
    /// Java `Entity.lastZ`: the position of the mob in the previous tick.
    pub last_z: f64,
}

impl Mob {
    /// The constructor of a mob.
    ///
    /// The random numbers come from the shared stream of the entities
    /// (`Entity.SHARED_RANDOM`), the reference uses one `Random` for all the
    /// entities.
    pub fn new(
        entity_id: i32,
        mob_type: MobType,
        x: f64,
        y: f64,
        z: f64,
        rng: &mut JavaRandom,
    ) -> Self {
        let slime_size = if mob_type == MobType::Slime {
            1 << rng.next_i32_bound(3)
        } else {
            1
        };
        let health = mob_type.default_health(slime_size);
        // The colour of the sheep is drawn by the spawner of the reference
        // (`SpawnerCreature`), with the random of the world and not with the
        // one of the entities, so the constructor does not draw it.
        let sheep_color = 0u8;
        // The egg timer of the chicken and the split timer of the slime are
        // drawn in the constructor of the reference.
        let chicken_egg_timer = if mob_type == MobType::Chicken {
            rng.next_i32_bound(6000) + 6000
        } else {
            6000
        };
        let slime_jump_delay = if mob_type == MobType::Slime {
            rng.next_i32_bound(20) + 10
        } else {
            10
        };

        Self {
            entity_id,
            mob_type,
            x,
            y,
            z,
            mot_x: 0.0,
            mot_y: 0.0,
            mot_z: 0.0,
            // The reference starts with the yaw of the entity (zero), the
            // mobs look around in their AI.
            yaw: 0.0,
            pitch: 0.0,
            health,
            max_health: health,
            air_ticks: 300,
            no_damage_ticks: 0,
            last_damage: 0,
            hurt_ticks: 0,
            health_before_damage: 0,
            attack_anim: 0.0,
            attack_anim_prev: 0.0,
            walk_dist: 0.0,
            attack_ticks: 0,
            fire_ticks: 0,
            fall_distance: 0.0,
            on_ground: false,
            horiz_collided: false,
            in_water: false,
            in_lava: false,
            web: false,
            air_borne: false,
            walk_accum: 0.0,
            step_counter: 0,
            position_changed: false,
            step_offset: 0.0,
            move_strafe: 0.0,
            move_forward: 0.0,
            sees_target: false,
            jump_flag: false,
            yaw_drift: 0.0,
            idle_sound_ticks: 0,
            look_target: None,
            look_ticks: 0,
            velocity_changed: false,
            dead: false,
            removed: false,
            death_ticks: 0,
            no_despawn_ticks: 0,
            target_id: None,
            path: Vec::new(),
            path_index: 0,
            pig_saddled: false,
            passenger_id: None,
            vehicle_id: None,
            sheep_color,
            sheep_sheared: false,
            chicken_egg_timer,
            chicken_wing: 0.0,
            chicken_wing_tip: 0.0,
            chicken_flap: 1.0,
            wolf_sitting: false,
            wolf_angry: false,
            wolf_tamed: false,
            wolf_owner: String::new(),
            wolf_wet: false,
            wolf_shaking: false,
            wolf_shake_time: 0.0,
            wolf_prev_shake_time: 0.0,
            creeper_fuse: 0,
            creeper_state: -1,
            creeper_powered: false,
            slime_size,
            slime_jump_delay,
            // The waypoint of the ghast is only set in its AI.
            ghast_waypoint: (0.0, 0.0, 0.0),
            ghast_course_cooldown: 0,
            ghast_charge: 0,
            ghast_target_timer: 0,
            pigzombie_anger: 0,
            pigzombie_sound_delay: 0,
            squid_swim_vec: (0.0, 0.0, 0.0),
            squid_tentacle_angle: 0.0,
            squid_speed: 0.0,
            squid_phase: 0.0,
            // Only the squid draws the speed of its phase (`EntitySquid`).
            squid_phase_speed: if mob_type == MobType::Squid {
                1.0 / (rng.next_f32() + 1.0) * 0.2
            } else {
                0.2
            },
            squid_squash: 0.0,
            squid_yaw: 0.0,
            look_yaw: 0.0,
            last_x: x,
            last_z: z,
            was_in_water: false,
            just_created: true,
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

    /// The bounding box of the mob (`Entity.setPosition`).
    pub fn aabb(&self) -> Aabb {
        let (length, width) = self.size();
        Aabb::for_entity(self.x, self.y, self.z, length, width, 0.0, self.step_offset)
    }

    /// Java `Entity.d(double, double, double)`: the mob swims up when the
    /// box it would move to is free.
    ///
    /// The method returns false when there is an entity in the box or when
    /// the box holds a liquid (`World.c(AxisAlignedBB)`).
    fn can_swim_up(&self, world: &World, box_: &Aabb, entity_boxes: &[Aabb]) -> bool {
        // The entities of the box: the reference asks the world for them.
        if entity_boxes.iter().any(|other| other.intersects(box_)) {
            return false;
        }
        // The liquid of the box (`World.c(AxisAlignedBB)`).
        !crate::entity::physics::box_has_fluid(world, box_)
    }

    /// Java `EntityLiving.a(float, float)`: the movement of the mob.
    ///
    /// The acceleration and the drag depend on the ground, the water and the
    /// lava, the collision is the axis aligned box of the reference
    /// (`Entity.move`). The returned value is the fall damage of the tick.
    pub fn apply_movement(
        &mut self,
        world: &World,
        entity_boxes: &[Aabb],
        scratch: &mut Vec<Aabb>,
        spider_climb: bool,
        flying: bool,
    ) -> f32 {
        let mut aabb = self.aabb();
        // The position of the mob before the movement (`EntityLiving.a`).
        let y_before = self.y;
        let x_before = self.x;
        let z_before = self.z;

        // The squid has its own movement (`EntitySquid.a(float, float)`): it
        // only moves, without the acceleration, the gravity and the drag of
        // the water.
        let squid = self.mob_type == MobType::Squid;

        let mut drag = 0.0f32;
        if !squid {
            // The acceleration (`Entity.a(float, float, float)`).
            // The reference computes the acceleration with single precision.
            let input = crate::util::math::math_helper_a(
                (self.move_strafe * self.move_strafe + self.move_forward * self.move_forward)
                    as f64,
            );
            let in_water = self.in_water;
            let in_lava = self.in_lava;
            let (acceleration, friction): (f32, f32) = if in_water || in_lava {
                (0.02, 0.5)
            } else {
                // The friction of the ground (`EntityLiving.a(float, float)`): the
                // block below the mob has its own friction factor.
                let friction = if self.on_ground {
                    // The block of the friction is under the bottom of the
                    // box of the mob (`EntityLiving.a(float, float)`), the
                    // bottom of the box is the position of the mob plus the
                    // offset of the step.
                    let below = world.get_block(
                        self.x.floor() as i32,
                        ((self.y + self.step_offset as f64).floor() as i32) - 1,
                        self.z.floor() as i32,
                    );
                    if below > 0 {
                        blocks::friction_factor(below) * 0.91
                    } else {
                        0.54600006
                    }
                } else {
                    0.91
                };
                let f3 = 0.16277136 / (friction * friction * friction);
                (if self.on_ground { 0.1 * f3 } else { 0.02 }, friction)
            };
            drag = friction;
            if input >= 0.01 {
                let scale = acceleration / input.max(1.0);
                let strafe = self.move_strafe * scale;
                let forward = self.move_forward * scale;
                // The sine and the cosine come from the table of the reference
                // (`MathHelper.sin` and `MathHelper.cos`), the plain functions
                // would drift a few units in the last place.
                let radians = self.yaw * JAVA_PI / 180.0f32;
                let sin = crate::util::math::sin(radians);
                let cos = crate::util::math::cos(radians);
                self.mot_x += (strafe * cos - forward * sin) as f64;
                self.mot_z += (strafe * sin + forward * cos) as f64;
            }
        }

        // The spider clings to the wall it touches (`EntitySpider.p()`).
        if self.mob_type == MobType::Spider && self.position_changed {
            self.mot_x = self.mot_x.clamp(-0.15, 0.15);
            self.mot_z = self.mot_z.clamp(-0.15, 0.15);
            self.fall_distance = 0.0;
            if self.mot_y < -0.15 {
                self.mot_y = -0.15;
            }
        }

        // The web (`Entity.move()`): the movement is damped and the
        // velocity of the mob is set to zero when the flag is set. The flag
        // is set by `BlockWeb.a` at the end of the previous movement.
        let mut req_x = self.mot_x;
        let mut req_y = self.mot_y;
        let mut req_z = self.mot_z;
        if self.web {
            self.web = false;
            req_x *= 0.25;
            req_y *= 0.05;
            req_z *= 0.25;
            self.mot_x = 0.0;
            self.mot_y = 0.0;
            self.mot_z = 0.0;
        }

        // The movement (`Entity.move`).
        let result = move_entity(
            world,
            &mut aabb,
            req_x,
            req_y,
            req_z,
            0.5,
            self.step_offset,
            self.on_ground,
            0.0,
            entity_boxes,
            scratch,
        );
        self.x = result.x;
        self.y = result.y;
        self.z = result.z;
        self.step_offset = result.step_offset;
        self.on_ground = result.on_ground;
        self.position_changed = result.position_changed;
        self.horiz_collided = result.moved_x != req_x || result.moved_z != req_z;

        // The animation of the walk (`EntityLiving.v()`): the time of the
        // animation of the hit follows the speed of the mob, and the
        // distance of the walk grows with it.
        self.attack_anim_prev = self.attack_anim;
        let dx = self.x - self.last_x;
        let dz = self.z - self.last_z;
        let mut walk = crate::util::math::math_helper_a(dx * dx + dz * dz) * 4.0;
        if walk > 1.0 {
            walk = 1.0;
        }
        self.attack_anim += (walk - self.attack_anim) * 0.4;
        self.walk_dist += self.attack_anim;

        // The web (`BlockWeb.a`): the flag is set when the box of the mob
        // holds a web, the movement of the next tick is damped then.
        if crate::entity::physics::count_blocks(world, &aabb, blocks::WEB) > 0 {
            self.web = true;
        }

        // The sound of the step (`Entity.move()`): the distance of the
        // walk is accumulated and the sound is played when the distance
        // passes the counter of the steps.
        self.walk_accum += crate::util::math::math_helper_a(
            (self.x - x_before) * (self.x - x_before) + (self.z - z_before) * (self.z - z_before),
        ) * 0.6;
        let below_x = self.x.floor() as i32;
        // The height of the entity (`Entity.height`) is zero for the mobs.
        let below_y = (self.y - 0.2).floor() as i32;
        let below_z = self.z.floor() as i32;
        let mut below = world.get_block(below_x, below_y, below_z);
        if world.get_block(below_x, below_y - 1, below_z) == blocks::FENCE {
            below = blocks::FENCE;
        }
        if self.walk_accum > self.step_counter as f32 && below > 0 {
            // The reference does not send the sound of the step to the
            // client (`WorldManager.a(String, double, double, double, float,
            // float)` has an empty body), the counter of the steps is
            // raised all the same.
            self.step_counter += 1;
        }

        // The fall distance (`Entity.a(double, boolean)`).
        let mut fall_damage = 0.0;
        if result.on_ground {
            if self.fall_distance > 0.0 {
                fall_damage = self.fall_distance;
                self.fall_distance = 0.0;
            }
        } else if result.moved_y < 0.0 {
            // `Entity.a(double, boolean)`: the distance is a single precision
            // value.
            self.fall_distance = (self.fall_distance as f64 - result.moved_y) as f32;
        }

        // The velocity of the blocked axes is dropped.
        if result.moved_x != req_x {
            self.mot_x = 0.0;
        }
        if result.moved_y != req_y {
            self.mot_y = 0.0;
        }
        if result.moved_z != req_z {
            self.mot_z = 0.0;
        }

        if squid {
            // `EntitySquid.a(float, float)`: the squid only moves, the
            // velocity of the axes is not changed by the movement.
        } else if self.in_water || self.in_lava {
            if flying {
                // `EntityFlying.a(float, float)`: the drag of the flying mobs.
                self.mot_x *= if self.in_water {
                    0.800_000_011_920_929
                } else {
                    0.5
                };
                self.mot_y *= if self.in_water {
                    0.800_000_011_920_929
                } else {
                    0.5
                };
                self.mot_z *= if self.in_water {
                    0.800_000_011_920_929
                } else {
                    0.5
                };
            } else {
                // The water and the lava slow the mob down
                // (`EntityLiving.a(float, float)`).
                let fluid_drag: f64 = if self.in_water {
                    0.800_000_011_920_929
                } else {
                    0.5
                };
                self.mot_x *= fluid_drag;
                self.mot_y *= fluid_drag;
                self.mot_z *= fluid_drag;
                self.mot_y -= 0.02;
                // The mob swims up when it is stuck
                // (`Entity.d(double, double, double)`): the method returns
                // false when there is an entity in the moved box or when the
                // box holds a liquid.
                if result.position_changed {
                    // The reference uses the position of the mob after the
                    // movement (`this.locY`) and the position before it
                    // (`d0`).
                    let box_ = aabb.moved(
                        self.mot_x,
                        self.mot_y + 0.600_000_023_841_857_9 - result.y + y_before,
                        self.mot_z,
                    );
                    if self.can_swim_up(world, &box_, entity_boxes) {
                        self.mot_y = 0.300_000_011_920_928_96;
                    }
                }
            }
        } else if flying {
            // `EntityFlying.a(float, float)`: no gravity, the drag on all the
            // axes.
            self.mot_x *= drag as f64;
            self.mot_y *= drag as f64;
            self.mot_z *= drag as f64;
        } else {
            // The spider climbs the wall it hits (`EntityLiving.a`).
            if result.position_changed && self.mob_type == MobType::Spider && spider_climb {
                self.mot_y = 0.2;
            }
            // The constants of the reference are the single precision
            // values (`0.9800000190734863D`).
            self.mot_y -= 0.08;
            self.mot_y *= 0.980_000_019_073_486_3;
            self.mot_x *= drag as f64;
            self.mot_z *= drag as f64;
        }
        fall_damage
    }

    /// Java `EntityLiving.t()`: the height of the eyes of the mob.
    ///
    /// The reference uses the width of the mob, not its height, and the
    /// multiplication is a single precision one.
    /// Java `EntityLiving.t()`: the height of the eyes of the mob.
    ///
    /// The base is `width * 0.85F` ✓, the wolf overrides it with
    /// `width * 0.8F` ✓ (`EntityWolf.java:207-209` ✓).
    pub fn eye_height(&self) -> f64 {
        let (_, width) = self.size();
        let factor = match self.mob_type {
            MobType::Wolf => 0.8f32,
            _ => 0.85f32,
        };
        (width * factor) as f64
    }

    /// Java `EntityLiving.k()`: the volume of the sound of the mob.
    pub fn sound_volume(&self) -> f32 {
        match self.mob_type {
            MobType::Cow | MobType::Squid | MobType::Wolf => 0.4,
            MobType::Slime => 0.6,
            MobType::Ghast => 10.0,
            _ => 1.0,
        }
    }

    /// Java `EntityLiving.h()`: the sound of the hit of the mob.
    pub fn hurt_sound(&self) -> &'static str {
        match self.mob_type {
            MobType::Pig => "mob.pig",
            MobType::Cow => "mob.cowhurt",
            MobType::Sheep => "mob.sheep",
            MobType::Chicken => "mob.chickenhurt",
            MobType::Wolf => "mob.wolf.hurt",
            MobType::Spider => "mob.spider",
            MobType::Creeper => "mob.creeper",
            MobType::Slime => "mob.slime",
            MobType::Zombie => "mob.zombiehurt",
            MobType::Skeleton => "mob.skeletonhurt",
            MobType::PigZombie => "mob.zombiepig.zpighurt",
            MobType::Ghast => "mob.ghast.scream",
            MobType::Squid => "",
            MobType::Giant => "mob.zombiehurt",
        }
    }

    /// Java `EntityLiving.i()`: the sound of the death of the mob.
    pub fn death_sound(&self) -> &'static str {
        match self.mob_type {
            MobType::Pig => "mob.pigdeath",
            MobType::Cow => "mob.cowhurt",
            MobType::Sheep => "mob.sheep",
            MobType::Chicken => "mob.chickenhurt",
            MobType::Wolf => "mob.wolf.death",
            MobType::Spider => "mob.spiderdeath",
            MobType::Creeper => "mob.creeperdeath",
            MobType::Slime => "mob.slime",
            MobType::Zombie => "mob.zombiedeath",
            MobType::Skeleton => "mob.skeletonhurt",
            MobType::PigZombie => "mob.zombiepig.zpigdeath",
            MobType::Ghast => "mob.ghast.death",
            MobType::Squid => "",
            MobType::Giant => "mob.zombiedeath",
        }
    }

    pub fn mounted_y_offset(&self) -> f64 {
        let (_, h) = self.size();
        if self.mob_type == MobType::Spider {
            (h * 0.75) as f64 - 0.5
        } else {
            (h * 0.75) as f64
        }
    }

    /// Java `EntityLiving.g()`: the name of the idle sound of the mob.
    ///
    /// The wolf draws a number for the sound (`EntityWolf.g()`), but only
    /// when it is not angry: the number is the condition of the branch.
    pub fn idle_sound_name(&self, rng: &mut JavaRandom) -> Option<&'static str> {
        match self.mob_type {
            MobType::Pig => Some("mob.pig"),
            MobType::Cow => Some("mob.cow"),
            MobType::Sheep => Some("mob.sheep"),
            MobType::Chicken => Some("mob.chicken"),
            MobType::Zombie => Some("mob.zombie"),
            MobType::PigZombie => Some("mob.zombiepig.zpig"),
            MobType::Skeleton => Some("mob.skeleton"),
            MobType::Spider => Some("mob.spider"),
            MobType::Ghast => Some("mob.ghast.moan"),
            MobType::Wolf => {
                if self.wolf_angry {
                    Some("mob.wolf.growl")
                } else {
                    let roll = rng.next_i32_bound(3);
                    Some(if roll == 0 {
                        // Java `EntityWolf.g()`: the whine needs the tamed
                        // wolf with the health below ten (the datawatcher 18
                        // is the health ✓).
                        if self.wolf_tamed && self.health < 10 {
                            "mob.wolf.whine"
                        } else {
                            "mob.wolf.panting"
                        }
                    } else {
                        "mob.wolf.bark"
                    })
                }
            }
            // The giant, the slime and the creeper have no sound of their own
            // (`EntityLiving.g()` is `null`).
            MobType::Squid | MobType::Slime | MobType::Creeper | MobType::Giant => None,
        }
    }

    /// Java `EntityLiving.e()`: the interval of the idle sound of the mob.
    ///
    /// The animals (`EntityAnimal.e()`) and the water animals
    /// (`EntityWaterAnimal.e()`) wait 120 ticks, the other mobs 80.
    pub fn idle_sound_interval(&self) -> i32 {
        match self.mob_type {
            MobType::Pig
            | MobType::Cow
            | MobType::Sheep
            | MobType::Chicken
            | MobType::Wolf
            | MobType::Squid => 120,
            _ => 80,
        }
    }

    pub fn fire_proof(&self) -> bool {
        matches!(self.mob_type, MobType::Ghast | MobType::PigZombie)
    }

    /// Java `EntityLiving.h_()`: can the mob leave the world when it is far
    /// from the players? Every mob can, except the tamed wolf.
    pub fn can_despawn(&self) -> bool {
        !(self.mob_type == MobType::Wolf && self.wolf_tamed)
    }

    /// Java `EntityCreature.w()`: the value of the flag `EntityLiving.e` at
    /// the beginning of the artificial intelligence.
    ///
    /// Only the wolf overrides the method of the reference: the flag is true
    /// when the wolf sits or begs for the food. The begging is not
    /// implemented in the port, so only the sitting wolf has the flag.
    pub fn reference_sees_target(&self) -> bool {
        match self.mob_type {
            MobType::Wolf => self.wolf_sitting,
            _ => false,
        }
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
    /// The drops of a dead mob (`EntityLiving.q()`).
    ///
    /// The random numbers come from the shared stream of the entities.
    pub fn get_death_drops(
        &mut self,
        rng: &mut JavaRandom,
        killed_by_skeleton_arrow: bool,
    ) -> Vec<ItemStack> {
        let mut drops = Vec::new();
        match self.mob_type {
            MobType::Pig => {
                // Java `EntityLiving.q()`: one draw of the number of the items
                // and **one** stack of them.
                let count = rng.next_i32_bound(3) as u8;
                let id = if self.fire_ticks > 0 {
                    items::GRILLED_PORK
                } else {
                    items::PORK
                };
                if count > 0 {
                    drops.push(ItemStack::new(id, count, 0));
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
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::LEATHER, count, 0));
                }
            }
            MobType::Chicken | MobType::Zombie => {
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::FEATHER, count, 0));
                }
            }
            MobType::Squid => {
                // Java `EntitySquid.q()`: one draw and one stack.
                let count = (rng.next_i32_bound(3) + 1) as u8;
                drops.push(ItemStack::new(items::INK_SACK, count, 0));
            }
            MobType::Skeleton => {
                // Java `EntitySkeleton.q()`: two draws and two stacks — the
                // arrows and the bones.
                let arrows = rng.next_i32_bound(3) as u8;
                if arrows > 0 {
                    drops.push(ItemStack::new(items::ARROW, arrows, 0));
                }
                let bones = rng.next_i32_bound(3) as u8;
                if bones > 0 {
                    drops.push(ItemStack::new(items::BONE, bones, 0));
                }
            }
            MobType::Spider => {
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::STRING, count, 0));
                }
            }
            MobType::Creeper => {
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::SULPHUR, count, 0));
                }
                if killed_by_skeleton_arrow {
                    let record = items::GOLD_RECORD + rng.next_i32_bound(2) as i16;
                    drops.push(ItemStack::new(record, 1, 0));
                }
            }
            MobType::Ghast => {
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::SULPHUR, count, 0));
                }
            }
            MobType::Slime => {
                // The base `q()` of the reference always draws the number of
                // the items, the slime of a size bigger than one drops
                // nothing (`EntitySlime.j()`).
                let count = rng.next_i32_bound(3) as u8;
                if self.slime_size == 1 && count > 0 {
                    drops.push(ItemStack::new(items::SLIME_BALL, count, 0));
                }
            }
            MobType::PigZombie => {
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::GRILLED_PORK, count, 0));
                }
            }
            MobType::Wolf => {
                // The wolf drops nothing (`EntityWolf.j()` returns -1), but
                // the base `q()` of the reference always draws the number of
                // the items.
                let _ = rng.next_i32_bound(3);
            }
            MobType::Giant => {
                // The giant is a zombie (`EntityZombie.j()` returns the
                // feathers).
                let count = rng.next_i32_bound(3) as u8;
                if count > 0 {
                    drops.push(ItemStack::new(items::FEATHER, count, 0));
                }
            }
        }
        drops
    }

    /// Java `Entity.c(NBTTagCompound)`: the mob in the NBT.
    ///
    /// The tags of `Entity.d(NBTTagCompound)` and of
    /// `EntityLiving.b(NBTTagCompound)`.
    pub fn to_nbt(&self, world_uuid: (i64, i64), entity_uuid: (i64, i64)) -> NbtTag {
        let base = crate::entity::save::BaseEntity {
            id: self.mob_type.nbt_id(),
            pos: (self.x, self.y + self.step_offset as f64, self.z),
            motion: (self.mot_x, self.mot_y, self.mot_z),
            rotation: (self.yaw, self.pitch),
            fall_distance: self.fall_distance,
            fire_ticks: self.fire_ticks,
            air_ticks: self.air_ticks as i32,
            on_ground: self.on_ground,
        };
        let mut tag = NbtTag::compound();
        base.write(&mut tag, world_uuid, entity_uuid);
        // Java `EntityLiving.b(NBTTagCompound)`.
        tag.insert("Health", NbtTag::Short(self.health));
        tag.insert("HurtTime", NbtTag::Short(self.hurt_ticks as i16));
        tag.insert("DeathTime", NbtTag::Short(self.death_ticks as i16));
        tag.insert("AttackTime", NbtTag::Short(self.attack_ticks as i16));

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

    /// Java `Entity.a(NBTTagCompound)`: the load of a mob.
    ///
    /// The random numbers come from the shared stream of the entities.
    pub fn from_nbt(entity_id: i32, tag: &NbtTag, rng: &mut JavaRandom) -> Option<Self> {
        let id_str = tag.get_str("id")?;
        let mob_type = MobType::from_nbt_id(id_str)?;
        // The position is read by the base tags (`Entity.e(NBTTagCompound)`).
        if tag.get_list("Pos").map(|pos| pos.len()) != Some(3) {
            return None;
        }

        let base = crate::entity::save::BaseEntity::read(tag, mob_type.nbt_id())?;
        let mut mob = Self::new(entity_id, mob_type, base.pos.0, base.pos.1, base.pos.2, rng);
        mob.mot_x = base.motion.0;
        mob.mot_y = base.motion.1;
        mob.mot_z = base.motion.2;
        mob.yaw = base.rotation.0;
        mob.pitch = base.rotation.1;
        mob.fall_distance = base.fall_distance;
        mob.fire_ticks = base.fire_ticks;
        mob.air_ticks = base.air_ticks as i16;
        mob.on_ground = base.on_ground;
        // Java `EntityLiving.a(NBTTagCompound)`.
        if let Some(h) = tag.get_i16("Health") {
            mob.health = h;
        }
        if let Some(h) = tag.get_i16("HurtTime") {
            mob.hurt_ticks = h as i32;
        }
        if let Some(d) = tag.get_i16("DeathTime") {
            mob.death_ticks = d as i32;
        }
        if let Some(a) = tag.get_i16("AttackTime") {
            mob.attack_ticks = a as i32;
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
// Pathfinder (`Pathfinder.java`, `PathPoint.java`, `Path.java`)
// -------------------------------------------------------------------------

/// Java `PathPoint`: a node of the path.
#[derive(Debug, Clone)]
struct PathPoint {
    x: i32,
    y: i32,
    z: i32,
    /// Java `PathPoint.e`: the cost from the start.
    g: f32,
    /// Java `PathPoint.f`: the heuristic to the goal.
    h: f32,
    /// Java `PathPoint.g`: the score in the open list.
    score: f32,
    /// Java `PathPoint.h`: the predecessor.
    parent: Option<usize>,
    /// Java `PathPoint.i`: the node was expanded already.
    visited: bool,
    /// Java `PathPoint.d`: the index in the open list (-1 when it is not in it).
    open_index: i32,
}

/// The A* of the reference implementation (`Pathfinder`).
///
/// The open list is the binary heap of `Path.java`; its tie breaking (the left
/// child wins, the parent stays above an equal child) decides which of the
/// equal paths is taken, so it is reproduced exactly.
struct Pathfinder<'a> {
    world: &'a World,
    points: Vec<PathPoint>,
    /// The index of the point of a position.
    lookup: HashMap<(i32, i32, i32), usize>,
    /// The open list: the indices of the points.
    heap: Vec<usize>,
    /// The goal of the path.
    goal: (i32, i32, i32),
    /// The size of the entity (`Pathfinder` builds a `PathPoint` of the size).
    size: (i32, i32, i32),
    /// The maximum distance to the goal.
    max_dist: f32,
}

impl<'a> Pathfinder<'a> {
    fn new(world: &'a World, size: (i32, i32, i32), goal: (i32, i32, i32), max_dist: f32) -> Self {
        Self {
            world,
            points: Vec::new(),
            lookup: HashMap::new(),
            heap: Vec::new(),
            goal,
            size,
            max_dist,
        }
    }

    /// Java `PathPoint.a(PathPoint)`: the Euclidean distance.
    fn distance(from: (i32, i32, i32), to: (i32, i32, i32)) -> f32 {
        let dx = (to.0 - from.0) as f32;
        let dy = (to.1 - from.1) as f32;
        let dz = (to.2 - from.2) as f32;
        crate::util::math::math_helper_a((dx * dx + dy * dy + dz * dz) as f64)
    }

    /// Java `Pathfinder.a(int, int, int)`: the point of a position.
    fn point(&mut self, x: i32, y: i32, z: i32) -> usize {
        if let Some(index) = self.lookup.get(&(x, y, z)) {
            return *index;
        }
        self.points.push(PathPoint {
            x,
            y,
            z,
            g: 0.0,
            h: 0.0,
            score: 0.0,
            parent: None,
            visited: false,
            open_index: -1,
        });
        let index = self.points.len() - 1;
        self.lookup.insert((x, y, z), index);
        index
    }

    /// Java `Pathfinder.a(Entity, int, int, int, PathPoint)`: the check of the
    /// blocks of a node.
    ///
    /// 1 = the node is free, 0 = a solid block, -1 = water, -2 = lava.
    fn check_block(&self, x: i32, y: i32, z: i32) -> i32 {
        for lx in x..x + self.size.0 {
            for ly in y..y + self.size.1 {
                for lz in z..z + self.size.2 {
                    let id = self.world.get_block(lx, ly, lz);
                    if id == 0 {
                        continue;
                    }
                    if id == blocks::IRON_DOOR || id == blocks::WOODEN_DOOR {
                        // An open door can be walked through.
                        let data = self.world.get_data(lx, ly, lz);
                        if data & 4 == 0 {
                            return 0;
                        }
                        continue;
                    }
                    if blocks::material_is_solid(id) {
                        return 0;
                    }
                    if blocks::is_water(id) {
                        return -1;
                    }
                    if blocks::is_lava(id) {
                        return -2;
                    }
                }
            }
        }
        1
    }

    /// Java `Pathfinder.a(Entity, int, int, int, PathPoint, int)`: the
    /// neighbour of a node.
    fn neighbour(&mut self, x: i32, y: i32, z: i32, step_up: bool) -> Option<usize> {
        let mut result = None;
        let mut y = y;
        if self.check_block(x, y, z) == 1 {
            result = Some(self.point(x, y, z));
        }
        if result.is_none() && step_up && self.check_block(x, y + 1, z) == 1 {
            result = Some(self.point(x, y + 1, z));
            y += 1;
        }
        if result.is_some() {
            let mut drops = 0;
            let mut below = 0;
            while y > 0 {
                below = self.check_block(x, y - 1, z);
                if below != 1 {
                    break;
                }
                drops += 1;
                if drops >= 4 {
                    return None;
                }
                y -= 1;
                if y > 0 {
                    result = Some(self.point(x, y, z));
                }
            }
            if below == -2 {
                return None;
            }
        }
        result
    }

    /// Java `Pathfinder.b(Entity, PathPoint, PathPoint, PathPoint, float)`: the
    /// four neighbours of a node.
    fn expand(&mut self, index: usize) -> [Option<usize>; 4] {
        let (x, y, z) = {
            let point = &self.points[index];
            (point.x, point.y, point.z)
        };
        let step_up = self.check_block(x, y + 1, z) == 1;
        let mut neighbours: [Option<usize>; 4] = [None, None, None, None];
        // The order of the reference: +Z, -X, +X, -Z.
        let candidates = [(x, y, z + 1), (x - 1, y, z), (x + 1, y, z), (x, y, z - 1)];
        for (slot, (nx, ny, nz)) in candidates.into_iter().enumerate() {
            if let Some(neighbour) = self.neighbour(nx, ny, nz, step_up) {
                let point = &self.points[neighbour];
                let position = (point.x, point.y, point.z);
                // The distance to the goal has to be below the maximum.
                let distance = Self::distance(position, self.goal);
                if distance < self.max_dist && !point.visited {
                    neighbours[slot] = Some(neighbour);
                }
            }
        }
        neighbours
    }

    /// Java `Path.a(PathPoint)`: the push onto the open list.
    fn push(&mut self, index: usize) {
        self.points[index].open_index = self.heap.len() as i32;
        self.heap.push(index);
        self.sift_up(self.heap.len() - 1);
    }

    /// Java `Path.b()`: the pop of the open list.
    fn pop(&mut self) -> Option<usize> {
        if self.heap.is_empty() {
            return None;
        }
        let top = self.heap[0];
        let last = self.heap.pop().unwrap();
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.points[last].open_index = 0;
            self.sift_down(0);
        }
        self.points[top].open_index = -1;
        Some(top)
    }

    /// Java `Path.a(int)`: the sift up of the heap.
    fn sift_up(&mut self, mut i: usize) {
        let index = self.heap[i];
        let score = self.points[index].score;
        while i > 0 {
            let parent = (i - 1) >> 1;
            let parent_index = self.heap[parent];
            if score >= self.points[parent_index].score {
                break;
            }
            self.heap[i] = parent_index;
            self.points[parent_index].open_index = i as i32;
            i = parent;
        }
        self.heap[i] = index;
        self.points[index].open_index = i as i32;
    }

    /// Java `Path.b(int)`: the sift down of the heap.
    fn sift_down(&mut self, mut i: usize) {
        let index = self.heap[i];
        let score = self.points[index].score;
        loop {
            let left = 1 + (i << 1);
            let right = left + 1;
            if left >= self.heap.len() {
                break;
            }
            let left_index = self.heap[left];
            let left_score = self.points[left_index].score;
            let (right_index, right_score) = if right >= self.heap.len() {
                (usize::MAX, f32::INFINITY)
            } else {
                let right_index = self.heap[right];
                (right_index, self.points[right_index].score)
            };
            if left_score < right_score {
                if left_score >= score {
                    break;
                }
                self.heap[i] = left_index;
                self.points[left_index].open_index = i as i32;
                i = left;
            } else {
                if right_score >= score {
                    break;
                }
                self.heap[i] = right_index;
                self.points[right_index].open_index = i as i32;
                i = right;
            }
        }
        self.heap[i] = index;
        self.points[index].open_index = i as i32;
    }

    /// Java `Pathfinder.a(Entity, PathPoint, PathPoint, PathPoint, float)`: the
    /// search.
    fn search(&mut self, start: (i32, i32, i32)) -> Option<Vec<(i32, i32, i32)>> {
        let start_index = self.point(start.0, start.1, start.2);
        self.points[start_index].g = 0.0;
        self.points[start_index].h = Self::distance(start, self.goal);
        self.points[start_index].score = self.points[start_index].h;
        self.heap.clear();
        self.push(start_index);

        let mut closest = start_index;
        while let Some(current) = self.pop() {
            let position = {
                let point = &self.points[current];
                (point.x, point.y, point.z)
            };
            if position == self.goal {
                return Some(self.build_path(current));
            }
            let closest_position = {
                let point = &self.points[closest];
                (point.x, point.y, point.z)
            };
            if Self::distance(position, self.goal) < Self::distance(closest_position, self.goal) {
                closest = current;
            }
            self.points[current].visited = true;
            for neighbour in self.expand(current).into_iter().flatten() {
                let (current_position, neighbour_position) = {
                    let current_point = &self.points[current];
                    let neighbour_point = &self.points[neighbour];
                    (
                        (current_point.x, current_point.y, current_point.z),
                        (neighbour_point.x, neighbour_point.y, neighbour_point.z),
                    )
                };
                let g =
                    self.points[current].g + Self::distance(current_position, neighbour_position);
                if self.points[neighbour].open_index < 0 || g < self.points[neighbour].g {
                    self.points[neighbour].parent = Some(current);
                    self.points[neighbour].g = g;
                    self.points[neighbour].h = Self::distance(neighbour_position, self.goal);
                    let score = g + self.points[neighbour].h;
                    if self.points[neighbour].open_index >= 0 {
                        let old_score = self.points[neighbour].score;
                        self.points[neighbour].score = score;
                        let position = self.points[neighbour].open_index as usize;
                        if score < old_score {
                            self.sift_up(position);
                        } else {
                            self.sift_down(position);
                        }
                    } else {
                        self.points[neighbour].score = score;
                        self.push(neighbour);
                    }
                }
            }
        }
        if closest == start_index {
            None
        } else {
            Some(self.build_path(closest))
        }
    }

    /// Java `Pathfinder.a(PathPoint, PathPoint)`: the path from the start to
    /// the goal.
    fn build_path(&self, end: usize) -> Vec<(i32, i32, i32)> {
        let mut path = Vec::new();
        let mut current = Some(end);
        while let Some(index) = current {
            let point = &self.points[index];
            path.push((point.x, point.y, point.z));
            current = point.parent;
        }
        path.reverse();
        path
    }
}

/// The constant of the reference (`3.1415927F`).
#[allow(clippy::approx_constant)]
pub const JAVA_PI: f32 = 3.1415927;

/// The double constant of the reference (`3.1415927410125732`).
#[allow(clippy::approx_constant)]
pub const JAVA_PI_DOUBLE: f64 = 3.1415927410125732;

/// The curve of the light of the reference (`WorldProvider.c()`).
///
/// The mobs score a block by the light of the block, the curve is not linear.
pub fn light_curve(level: i32) -> f32 {
    let f = 0.05f32;
    let i = level.clamp(0, 15) as f32;
    let f1 = 1.0 - i / 15.0;
    (1.0 - f1) / (f1 * 3.0 + 1.0) * (1.0 - f) + f
}

/// Java `Pathfinder.a(Entity, int x, int y, int z, float maxDist)`.
///
/// The `length` and the `width` are the size of the mob (`Entity.length` and
/// `Entity.width`), the goal is the position of the target.
#[allow(clippy::too_many_arguments)]
pub fn find_path(
    world: &World,
    start_x: f64,
    start_y: f64,
    start_z: f64,
    length: f32,
    width: f32,
    goal_x: f64,
    goal_y: f64,
    goal_z: f64,
    max_dist: f32,
) -> Vec<(i32, i32, i32)> {
    // Java `Pathfinder.java:26`: `entity.length / 2.0F` — the division is in
    // a single precision ✓ — and the goal is `MathHelper.floor(d0 - (double)
    // (entity.length / 2.0F))` ✓.
    let half = (length / 2.0f32) as f64;
    let start = (
        crate::util::math::floor_double(start_x - half),
        crate::util::math::floor_double(start_y),
        crate::util::math::floor_double(start_z - half),
    );
    let goal = (
        crate::util::math::floor_double(goal_x - half),
        crate::util::math::floor_double(goal_y),
        crate::util::math::floor_double(goal_z - half),
    );
    // `Pathfinder.java:27`: the size box of the entity —
    // `MathHelper.d(entity.length + 1.0F)` ✓ — the addition is in a single
    // precision ✓.
    let size = (
        crate::util::math::floor_float(length + 1.0f32),
        crate::util::math::floor_float(width + 1.0f32),
        crate::util::math::floor_float(length + 1.0f32),
    );
    let mut pathfinder = Pathfinder::new(world, size, goal, max_dist);
    pathfinder.search(start).unwrap_or_default()
}

// -------------------------------------------------------------------------
// SpawnerCreature (`SpawnerCreature.java` + `BiomeBase.java`)
// -------------------------------------------------------------------------

/// The order in which the mobs are listed in the biome spawn tables.
///
/// Java builds the tables in the `BiomeBase` constructor (spider, zombie,
/// skeleton, creeper, slime, sheep, pig, chicken, cow, squid) and the
/// biome subclasses append their own mobs (the wolf in the forest and the
/// taiga, the ghast and the pig zombie in the hell). The weighted pick walks
/// the list in order, so the order has to stay the same to keep the random
/// sequence of the world identical.
const SPAWN_TABLE_ORDER: [MobType; 14] = [
    MobType::Spider,
    MobType::Zombie,
    MobType::Skeleton,
    MobType::Creeper,
    MobType::Slime,
    MobType::Giant,
    MobType::Ghast,
    MobType::PigZombie,
    MobType::Sheep,
    MobType::Pig,
    MobType::Chicken,
    MobType::Cow,
    MobType::Wolf,
    MobType::Squid,
];

/// The mobs of a category that may spawn at all, in the order of the Java
/// biome spawn tables.
///
/// The global switches of the `Uberbukkit` settings act as master switches on
/// top of the per mob configuration.
fn category_candidates(
    spawning: &SpawningConfig,
    category: MobCategory,
) -> Vec<(MobType, &MobSpawnSettings)> {
    let mut candidates = Vec::new();
    for &mob_type in &SPAWN_TABLE_ORDER {
        if mob_type.category() != category {
            continue;
        }
        let master_enabled = match mob_type {
            MobType::Slime => spawning.spawn_slimes,
            MobType::Wolf => spawning.spawn_wolves,
            MobType::Squid => spawning.spawn_squids,
            _ => true,
        };
        if !master_enabled {
            continue;
        }
        let Some(settings) = spawning.mobs.get(mob_type.config_name()) else {
            continue;
        };
        if !settings.enabled || settings.weight <= 0 {
            continue;
        }
        candidates.push((mob_type, settings));
    }
    candidates
}

/// Picks a mob out of a weighted list (`BiomeMeta`).
///
/// A list with a single entry does not roll the dice, which keeps the random
/// sequence of the world identical to the one of the hard coded tables.
fn pick_weighted(entries: &[(MobType, i32)], rng: &mut JavaRandom) -> Option<MobType> {
    if entries.is_empty() {
        return None;
    }
    if entries.len() == 1 {
        return Some(entries[0].0);
    }
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
    /// The chance of the skeleton rider of a spider (`SpawnerCreature`
    /// uses `world.random.nextInt(100) == 0`).
    pub jockey_chance: i32,
}

/// Java `EntityLiving.d()` and `EntityCreature.d()`: can the mob live at the
/// position?
///
/// * the chunk of the box has to be loaded ✓ (`World.containsEntity` ✓),
/// * the box must not hold a block nor an entity ✓
///   (`World.getEntities(this, boundingBox).size() == 0` ✓ — the list has the
///   collision boxes of the blocks ✓ and the boxes of the entities ✓),
/// * the box must not hold a liquid ✓ (`World.c(AxisAlignedBB)` ✓),
/// * the light check of the class of the mob ✓ (`EntityCreature.d()` ✓ —
///   `a(i, j, k) >= 0.0F` ✓).
fn mob_can_spawn_here(
    world: &World,
    mob_type: MobType,
    x: f64,
    y: f64,
    z: f64,
    entity_boxes: &[crate::entity::physics::Aabb],
    sky_sub: i32,
) -> bool {
    // The box of the mob (`Entity.setPosition` ✓): the height of the mob is
    // the width of the entity ✓ and the length is the half of the box ✓.
    let (length, width) = mob_type.dimensions(1);
    let box_ = crate::entity::physics::Aabb::for_entity(x, y, z, length, width, 0.0, 0.0);

    // `World.containsEntity(boundingBox)`: the chunk of the box is loaded ✓.
    let chunk = ChunkPos::new(
        (box_.min_x.floor() as i32) >> 4,
        (box_.min_z.floor() as i32) >> 4,
    );
    if !world.is_chunk_loaded(chunk) {
        return false;
    }

    // The blocks and the entities of the box ✓ (`World.getEntities` ✓).
    let mut scratch = Vec::new();
    crate::entity::physics::collect_collision_boxes(world, &box_, entity_boxes, &mut scratch);
    if !scratch.is_empty() {
        return false;
    }

    // The liquid of the box ✓ (`World.c(AxisAlignedBB)` ✓).
    if crate::entity::physics::box_has_fluid(world, &box_) {
        return false;
    }

    // The light check of the class of the mob ✓ (`EntityCreature.d()` ✓).
    let i = x.floor() as i32;
    let j = box_.min_y.floor() as i32;
    let k = z.floor() as i32;
    let light = chunk_light_at(world, i, j, k, sky_sub);
    match mob_type {
        // `EntityMonster.a(i, j, k)`: `0.5F - world.n(i, j, k)` ✓ — the
        // monster needs a dark spot ✓.
        MobType::Zombie
        | MobType::Skeleton
        | MobType::Creeper
        | MobType::Spider
        | MobType::Giant
        | MobType::PigZombie => 0.5 - light_curve(light) >= 0.0,
        // `EntityAnimal.a(i, j, k)`: on the grass the check always passes ✓,
        // otherwise the curve of the light minus half a ✓.
        MobType::Pig | MobType::Sheep | MobType::Cow | MobType::Chicken => {
            if world.get_block(i, j - 1, k) == blocks::GRASS {
                true
            } else {
                light_curve(light) - 0.5 >= 0.0
            }
        }
        // `EntityCreature.a(i, j, k)` is zero ✓ — the check always passes ✓
        // (the water animal has no light check at all ✓ — `EntityWaterAnimal.d()`
        // ✓).
        _ => true,
    }
}

/// Java `SpawnerCreature.spawnEntities`: evaluate natural spawns around online players.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_natural_spawns(
    world: &World,
    rng: &mut JavaRandom,
    player_positions: &[(f64, f64, f64)],
    entity_boxes: &[crate::entity::physics::Aabb],
    allow_monsters: bool,
    allow_animals: bool,
    current_monsters: usize,
    current_animals: usize,
    current_water: usize,
) -> Vec<SpawnedMobSpec> {
    let spawning = &world.spawning;
    if player_positions.is_empty() || (!allow_monsters && !allow_animals) {
        return Vec::new();
    }

    // The set is sorted: the order of the chunks decides the draws of the
    // random of the world, and the order of a hash table of Rust is random
    // for every process.
    let mut candidate_chunks = BTreeSet::new();
    let radius = spawning.spawn_radius_chunks.clamp(0, 32);
    for &(px, _, pz) in player_positions {
        let pcx = (px.floor() as i32) >> 4;
        let pcz = (pz.floor() as i32) >> 4;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
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
    let sky_sub = world.sky_factor();
    let min_player_distance = spawning.min_player_distance;
    let min_spawn_distance = spawning.min_spawn_distance;
    let sample_divisor = spawning.chunk_sample_divisor.max(1);
    let mut spawned = Vec::new();

    // The categories of `EnumCreatureType`.
    let categories = [
        (
            MobCategory::Monster,
            allow_monsters && spawning.spawn_monsters,
            current_monsters,
            spawning.monster_cap,
        ),
        (
            MobCategory::Creature,
            allow_animals && spawning.spawn_animals,
            current_animals,
            spawning.animal_cap,
        ),
        (
            MobCategory::WaterCreature,
            allow_animals && spawning.spawn_water_mobs,
            current_water,
            spawning.water_mob_cap,
        ),
    ];

    for (category, enabled, cur_count, max_base) in categories {
        if !enabled {
            continue;
        }
        let cap = max_base * candidate_chunks.len() / 256;
        if cur_count > cap {
            continue;
        }
        let candidates = category_candidates(spawning, category);
        if candidates.is_empty() {
            continue;
        }
        // The buffers are reused for every chunk; the spawn pass runs every
        // tick over every loaded chunk, so allocations would show up in the
        // tick time.
        let mut biome_candidates: Vec<(MobType, &MobSpawnSettings)> = Vec::with_capacity(16);
        let mut table: Vec<(MobType, i32)> = Vec::with_capacity(16);

        for &cpos in &candidate_chunks {
            // Only sample a subset of chunks per tick so TPS stays 20.0
            if rng.next_i32_bound(sample_divisor) != 0 {
                continue;
            }
            let Some(chunk) = world.get_chunk(cpos) else {
                continue;
            };
            let biome = if world.is_hell {
                Biome::Hell
            } else {
                Biome::from_id(chunk.biomes[0])
            };
            // The spawn table of the biome (`BiomeBase.a(type)`).
            biome_candidates.clear();
            table.clear();
            for &(mob_type, settings) in &candidates {
                if settings.spawns_in(biome) {
                    biome_candidates.push((mob_type, settings));
                    table.push((mob_type, settings.weight));
                }
            }
            if biome_candidates.is_empty() {
                continue;
            }
            let Some(mob_type) = pick_weighted(&table, rng) else {
                continue;
            };
            let Some(settings) = biome_candidates
                .iter()
                .find(|(candidate, _)| *candidate == mob_type)
                .map(|(_, settings)| *settings)
            else {
                continue;
            };

            let base_x = cpos.x * 16 + rng.next_i32_bound(16);
            let base_y = rng.next_i32_bound(128);
            let base_z = cpos.z * 16 + rng.next_i32_bound(16);

            let base_id = world.get_block(base_x, base_y, base_z);
            if settings.require_water {
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
                    if y < settings.min_y || y > settings.max_y {
                        continue;
                    }

                    let can_stand = if settings.require_water {
                        blocks::is_water(world.get_block(x, y, z))
                            && !world.is_solid_normal_at(x, y + 1, z)
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
                        dx * dx + dy * dy + dz * dz < min_player_distance * min_player_distance
                    });
                    if near_player {
                        continue;
                    }
                    let dsx = fx - spawn_x as f64;
                    let dsy = fy - spawn_y as f64;
                    let dsz = fz - spawn_z as f64;
                    if dsx * dsx + dsy * dsy + dsz * dsz < min_spawn_distance * min_spawn_distance {
                        continue;
                    }

                    // The spawn conditions of the mob (`Entity.d()`).
                    // The sky light is rolled before the light of the block
                    // (`EntityMonster.d()`), the sky light is the raw one and
                    // the light of the block has the shift of the sky.
                    if settings.sky_light_roll > 0 {
                        let sky_l = chunk_sky_light_at(world, x, y, z);
                        if sky_l > rng.next_i32_bound(settings.sky_light_roll) {
                            continue;
                        }
                    }
                    if settings.light_roll > 0 {
                        let light = chunk_light_at(world, x, y, z, sky_sub);
                        if light > rng.next_i32_bound(settings.light_roll) {
                            continue;
                        }
                    }
                    if settings.min_light > 0 {
                        // The animals are checked against the light of the
                        // day (`EntityAnimal.d()`: `world.k(i, j, k) > 8`,
                        // the light without the shift of the sky).
                        let light = chunk_light_at(world, x, y, z, 0);
                        if light < settings.min_light {
                            continue;
                        }
                    }
                    if settings.require_grass_below && world.get_block(x, y - 1, z) != blocks::GRASS
                    {
                        continue;
                    }
                    if settings.spawn_chance < 100
                        && rng.next_i32_bound(100) >= settings.spawn_chance
                    {
                        continue;
                    }
                    if settings.slime_chunk_only && !is_slime_chunk(world.seed(), x >> 4, z >> 4) {
                        continue;
                    }
                    // The check of the reference (`mob.d()` ✓ —
                    // `EntityLiving.d()` ✓ and `EntityCreature.d()` ✓).
                    if !mob_can_spawn_here(world, mob_type, fx, fy, fz, entity_boxes, sky_sub) {
                        continue;
                    }

                    spawned.push(SpawnedMobSpec {
                        mob_type,
                        x: fx,
                        y: fy,
                        z: fz,
                        jockey_chance: settings.jockey_chance,
                    });
                    group_spawned += 1;
                    if group_spawned >= settings.max_group {
                        break 'group;
                    }
                }
            }
        }
    }

    spawned
}

pub fn world_sky_subtracted(world_time: i64) -> i32 {
    world_sky_subtracted_with_weather(world_time, 0.0, 0.0)
}

/// Java `World.a(float)`: the factor of the sky of the world.
///
/// The light of the sky is dimmed by the rain and by the thunder: the
/// factor is multiplied by `1 - intensity * 5 / 16` for each of them.
pub fn world_sky_subtracted_with_weather(world_time: i64, rain: f32, thunder: f32) -> i32 {
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
    // The rain and the thunder dim the light of the sky.
    let mut f = 1.0 - f3;
    f *= 1.0 - (rain * 5.0) / 16.0;
    f *= 1.0 - (thunder * 5.0) / 16.0;
    ((1.0 - f) * 11.0) as i32
}

/// Java `EntityLiving.b(float)`: the vector of the look of the entity.
///
/// For the factor of one ✓ the vector is built from the yaw and the pitch of
/// the entity ✓ (`EntityLiving.java:884-889` ✓): the trigonometry of the
/// reference is the table of `MathHelper` ✓.
pub fn look_vector(yaw: f32, pitch: f32) -> (f64, f64, f64) {
    let f1 = crate::util::math::cos(-yaw * 0.017453292f32 - JAVA_PI);
    let f2 = crate::util::math::sin(-yaw * 0.017453292f32 - JAVA_PI);
    let f3 = -crate::util::math::cos(-pitch * 0.017453292f32);
    let f4 = crate::util::math::sin(-pitch * 0.017453292f32);
    ((f2 * f3) as f64, f4 as f64, (f1 * f3) as f64)
}

/// Java `Entity.c(float)`: the light of the block the mob is in.
///
/// The reference uses the light of the block under the middle of the box of
/// the mob, the value is the curve of the light of the reference.
pub fn mob_light_at(world: &World, mob: &Mob, sky_sub: i32) -> f32 {
    let box_ = mob.aabb();
    let offset = (box_.max_y - box_.min_y) * 0.66;
    let y = (mob.y + offset).floor() as i32;
    light_curve(chunk_light_at(
        world,
        mob.x.floor() as i32,
        y,
        mob.z.floor() as i32,
        sky_sub,
    ))
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

    /// The vector of the look of the entity.
    ///
    /// Java `EntityLiving.b(float)`: for the factor of one the vector is
    /// built from the yaw and the pitch of the entity - the trigonometry of
    /// the reference is the table of `MathHelper`.
    #[test]
    fn the_vector_of_the_look_is_the_direction_of_the_angles() {
        // The yaw of zero: the vector points to the positive Z.
        let (x, y, z) = look_vector(0.0, 0.0);
        assert!(x.abs() < 1e-6, "the X of the vector");
        assert!(y.abs() < 1e-6, "the Y of the vector");
        assert!((z - 1.0).abs() < 1e-6, "the Z of the vector");
        // The length of the vector is one.
        let (x, y, z) = look_vector(35.0, -20.0);
        let len = (x * x + y * y + z * z).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "the length of the vector");
    }

    /// The wolf whines only when it is tamed and its health is below ten.
    ///
    /// Java `EntityWolf.g()`: `isTamed() && datawatcher.b(18) < 10` — the
    /// datawatcher 18 is the health of the wolf ✓.
    #[test]
    fn the_wolf_whines_only_when_it_is_tamed_and_its_health_is_low() {
        // The seed whose first draw of `nextInt(3)` is zero — the branch of
        // the whine.
        let mut seed = 0i64;
        while JavaRandom::new(seed).next_i32_bound(3) != 0 {
            seed += 1;
        }
        let mut mob = Mob::new(1, MobType::Wolf, 0.0, 64.0, 0.0, &mut JavaRandom::new(0));
        // The wild wolf pants.
        assert_eq!(
            mob.idle_sound_name(&mut JavaRandom::new(seed)),
            Some("mob.wolf.panting")
        );
        // The tamed wolf with the health of ten pants too.
        mob.wolf_tamed = true;
        mob.health = 10;
        assert_eq!(
            mob.idle_sound_name(&mut JavaRandom::new(seed)),
            Some("mob.wolf.panting")
        );
        // The tamed wolf with the health of nine whines.
        mob.health = 9;
        assert_eq!(
            mob.idle_sound_name(&mut JavaRandom::new(seed)),
            Some("mob.wolf.whine")
        );
    }

    use crate::config::{WorldConfig, MOB_SPAWN_NAMES};
    use crate::world::chunk::CHUNK_AREA;

    /// A world with a flat grass surface in the chunk `(5, 5)`; the chunk of
    /// the world spawn is empty.
    fn spawn_world(spawning: SpawningConfig) -> World {
        let mut world = World::new(WorldConfig {
            seed: 100,
            save_enabled: false,
            ..Default::default()
        });
        world.spawning = spawning;
        world.ensure_chunk_base(ChunkPos::new(0, 0));
        world.ensure_chunk_base(ChunkPos::new(5, 5));
        for x in 80..96 {
            for z in 80..96 {
                world.set_block(x, 63, z, blocks::GRASS);
                world.set_block(x, 64, z, blocks::AIR);
                world.set_block(x, 65, z, blocks::AIR);
            }
        }
        if let Some(chunk) = world.get_chunk_mut(ChunkPos::new(5, 5)) {
            chunk.biomes = [Biome::Plains.id(); CHUNK_AREA];
            chunk.init_lighting(false);
        }
        while world.java_do_lighting() {}
        world
    }

    /// The spawn table of a category in a biome (`BiomeBase.a(type)`).
    fn table_for_biome(
        spawning: &SpawningConfig,
        category: MobCategory,
        biome: Biome,
    ) -> Vec<(&'static str, i32)> {
        category_candidates(spawning, category)
            .iter()
            .filter(|(_, settings)| settings.spawns_in(biome))
            .map(|(mob_type, settings)| (mob_type.config_name(), settings.weight))
            .collect()
    }

    /// Runs the natural spawn evaluation a few hundred times; the position of
    /// a spawn attempt is rolled randomly, so a single run is not enough.
    fn collect_spawns(world: &World, seed: i64, rounds: i64) -> Vec<SpawnedMobSpec> {
        let mut spawned = Vec::new();
        for round in 0..rounds {
            let mut rng = JavaRandom::new(seed + round);
            let players = [(0.0, 64.0, 0.0)];
            spawned.extend(evaluate_natural_spawns(
                world,
                &mut rng,
                &players,
                &[],
                true,
                true,
                0,
                0,
                0,
            ));
        }
        spawned
    }

    /// The spawn of the mob needs a free box.
    ///
    /// Java `EntityLiving.d()`: the box of the mob must not hold a block nor
    /// an entity ✓ (`World.getEntities(this, boundingBox).size() == 0` ✓) and
    /// must not hold a liquid ✓ (`World.c(AxisAlignedBB)` ✓).
    #[test]
    fn the_spawn_of_the_mob_needs_a_free_box() {
        let mut world = spawn_world(SpawningConfig::default());
        // The box of the pig at (80.5, 64, 80.5) is free ✓ (the flat area of
        // the world ✓).
        assert!(
            mob_can_spawn_here(&world, MobType::Pig, 80.5, 64.0, 80.5, &[], 0),
            "the pig spawns in a free box"
        );
        // A block in the box ✓ — the pig does not spawn ✓.
        world.set_block(80, 64, 80, blocks::STONE);
        assert!(
            !mob_can_spawn_here(&world, MobType::Pig, 80.5, 64.0, 80.5, &[], 0),
            "the pig does not spawn in a blocked box"
        );
    }

    /// The monster needs a dark spot.
    ///
    /// Java `EntityMonster.a(i, j, k)`: `0.5F - world.n(i, j, k)` — the
    /// monster spawns only where the curve of the light is not above half ✓.
    #[test]
    fn the_monster_needs_a_dark_spot() {
        let mut world = spawn_world(SpawningConfig::default());
        // The sky light of the day is fifteen ✓ — the curve of the light is
        // one ✓ — the monster does not spawn ✓.
        // The sky light of the day ✓ — the curve of the light is one ✓ — the
        // monster does not spawn ✓.
        if let Some(chunk) = world.get_chunk_mut(ChunkPos::new(5, 5)) {
            for x in 0..16 {
                for z in 0..16 {
                    for y in 0..128 {
                        chunk.set_sky_light(x, y, z, 15);
                    }
                }
            }
        }
        assert!(
            !mob_can_spawn_here(&world, MobType::Zombie, 80.5, 64.0, 80.5, &[], 0),
            "the monster does not spawn in the light"
        );
    }

    #[test]
    fn the_spawn_tables_match_the_java_biome_lists() {
        let spawning = SpawningConfig::default();
        assert_eq!(
            table_for_biome(&spawning, MobCategory::Monster, Biome::Plains),
            vec![
                ("spider", 10),
                ("zombie", 10),
                ("skeleton", 10),
                ("creeper", 10),
                ("slime", 10)
            ]
        );
        assert_eq!(
            table_for_biome(&spawning, MobCategory::Creature, Biome::Plains),
            vec![("sheep", 12), ("pig", 10), ("chicken", 10), ("cow", 8)]
        );
        // The forest and the taiga have wolves, the other biomes have not.
        assert_eq!(
            table_for_biome(&spawning, MobCategory::Creature, Biome::Forest),
            vec![
                ("sheep", 12),
                ("pig", 10),
                ("chicken", 10),
                ("cow", 8),
                ("wolf", 2)
            ]
        );
        assert_eq!(
            table_for_biome(&spawning, MobCategory::WaterCreature, Biome::Plains),
            vec![("squid", 10)]
        );
        // The hell only has ghosts and pig zombies.
        assert_eq!(
            table_for_biome(&spawning, MobCategory::Monster, Biome::Hell),
            vec![("ghast", 10), ("pig_zombie", 10)]
        );
    }

    #[test]
    fn every_configured_mob_has_a_name() {
        for name in MOB_SPAWN_NAMES {
            let mob_type = MobType::from_config_name(name).unwrap();
            assert_eq!(mob_type.config_name(), name);
        }
        assert_eq!(MobType::from_config_name("zombi"), None);
    }

    #[test]
    fn disabled_mobs_leave_the_spawn_table() {
        let mut spawning = SpawningConfig::default();
        spawning.mobs.get_mut("zombie").unwrap().enabled = false;
        spawning.mobs.get_mut("wolf").unwrap().weight = 0;
        spawning.mobs.get_mut("giant").unwrap().enabled = false;

        assert_eq!(
            table_for_biome(&spawning, MobCategory::Monster, Biome::Plains),
            vec![
                ("spider", 10),
                ("skeleton", 10),
                ("creeper", 10),
                ("slime", 10)
            ]
        );
        assert_eq!(
            table_for_biome(&spawning, MobCategory::Creature, Biome::Forest),
            vec![("sheep", 12), ("pig", 10), ("chicken", 10), ("cow", 8)]
        );
    }

    #[test]
    fn the_master_switches_disable_the_mobs() {
        let spawning = SpawningConfig {
            spawn_slimes: false,
            spawn_wolves: false,
            spawn_squids: false,
            ..Default::default()
        };

        assert_eq!(
            table_for_biome(&spawning, MobCategory::Monster, Biome::Plains),
            vec![
                ("spider", 10),
                ("zombie", 10),
                ("skeleton", 10),
                ("creeper", 10)
            ]
        );
        assert_eq!(
            table_for_biome(&spawning, MobCategory::Creature, Biome::Forest),
            vec![("sheep", 12), ("pig", 10), ("chicken", 10), ("cow", 8)]
        );
        assert!(table_for_biome(&spawning, MobCategory::WaterCreature, Biome::Plains).is_empty());
    }

    #[test]
    fn the_biome_filter_limits_the_mobs() {
        let spawning = SpawningConfig::default();
        let wolf = &spawning.mobs["wolf"];
        assert!(wolf.spawns_in(Biome::Forest));
        assert!(wolf.spawns_in(Biome::Taiga));
        assert!(!wolf.spawns_in(Biome::Plains));
        assert!(!wolf.spawns_in(Biome::Desert));

        let ghast = &spawning.mobs["ghast"];
        assert!(ghast.spawns_in(Biome::Hell));
        assert!(!ghast.spawns_in(Biome::Plains));

        let zombie = &spawning.mobs["zombie"];
        assert!(zombie.spawns_in(Biome::Desert));
        assert!(zombie.spawns_in(Biome::Sky));
        assert!(!zombie.spawns_in(Biome::Hell));
    }

    #[test]
    fn a_single_candidate_does_not_roll_the_dice() {
        let mut rng = JavaRandom::new(7);
        let entries = [(MobType::Squid, 10)];
        assert_eq!(pick_weighted(&entries, &mut rng), Some(MobType::Squid));
        // The random state was not touched.
        let mut other = JavaRandom::new(7);
        assert_eq!(other.next_i32(), rng.next_i32());
    }

    #[test]
    fn natural_spawns_follow_the_configured_mobs() {
        let spawning = SpawningConfig {
            chunk_sample_divisor: 1,
            ..Default::default()
        };
        let world = spawn_world(spawning.clone());

        let spawned = collect_spawns(&world, 1234, 512);
        assert!(!spawned.is_empty(), "the grass field must produce mobs");
        // The plains biome has no wolves and the giant never spawns.
        assert!(spawned
            .iter()
            .all(|spec| spec.mob_type != MobType::Wolf && spec.mob_type != MobType::Giant));
        // The mobs keep the distance to the players and to the world spawn.
        assert!(spawned.iter().all(|spec| {
            let dx = spec.x;
            let dy = spec.y - 64.0;
            let dz = spec.z;
            (dx * dx + dy * dy + dz * dz).sqrt() >= 24.0
        }));
        // The animals stand on the grass (`EntityAnimal.d()`).
        assert!(spawned
            .iter()
            .filter(|spec| spec.mob_type.is_animal())
            .all(
                |spec| world.get_block(spec.x as i32, spec.y as i32 - 1, spec.z as i32)
                    == blocks::GRASS
            ));
    }

    #[test]
    fn a_disabled_mob_never_spawns() {
        let mut spawning = SpawningConfig {
            chunk_sample_divisor: 1,
            ..Default::default()
        };
        spawning.mobs.get_mut("zombie").unwrap().enabled = false;
        spawning.mobs.get_mut("pig").unwrap().enabled = false;
        let world = spawn_world(spawning);

        let spawned = collect_spawns(&world, 1234, 512);
        assert!(!spawned.is_empty());
        assert!(spawned
            .iter()
            .all(|spec| spec.mob_type != MobType::Zombie && spec.mob_type != MobType::Pig));
    }

    #[test]
    fn the_spawn_group_size_is_respected() {
        let mut spawning = SpawningConfig {
            chunk_sample_divisor: 1,
            ..Default::default()
        };
        spawning.mobs.get_mut("zombie").unwrap().max_group = 1;
        // Only the zombie may spawn, so every group holds a single mob.
        for name in ["spider", "skeleton", "creeper", "slime"] {
            spawning.mobs.get_mut(name).unwrap().enabled = false;
        }
        for name in ["sheep", "pig", "chicken", "cow", "wolf"] {
            spawning.mobs.get_mut(name).unwrap().enabled = false;
        }
        spawning.mobs.get_mut("squid").unwrap().enabled = false;
        let world = spawn_world(spawning);

        // The position of a spawn attempt is rolled randomly, so the first
        // round that spawns a zombie is used to check the group size.
        for round in 0..512 {
            let mut rng = JavaRandom::new(4321 + round);
            let players = [(0.0, 64.0, 0.0)];
            let spawned =
                evaluate_natural_spawns(&world, &mut rng, &players, &[], true, true, 0, 0, 0);
            if spawned.is_empty() {
                continue;
            }
            // Every chunk holds at most one group of one mob.
            assert!(
                spawned.len() <= 2,
                "too many mobs in one round: {}",
                spawned.len()
            );
            assert!(spawned.iter().all(|spec| spec.mob_type == MobType::Zombie));
            return;
        }
        panic!("no zombie spawned");
    }

    #[test]
    fn the_spawn_chance_is_a_percentage() {
        // A chance of 0 percent means that the mob never spawns.
        let mut spawning = SpawningConfig {
            chunk_sample_divisor: 1,
            ..Default::default()
        };
        for name in [
            "spider", "zombie", "skeleton", "creeper", "slime", "sheep", "pig", "chicken", "cow",
            "wolf", "squid",
        ] {
            spawning.mobs.get_mut(name).unwrap().spawn_chance = 0;
        }
        let world = spawn_world(spawning);
        let spawned = collect_spawns(&world, 1234, 512);
        assert!(spawned.is_empty(), "no mob may spawn with a chance of 0");

        // A chance of 100 percent means that the mob always spawns.
        let spawning = SpawningConfig {
            chunk_sample_divisor: 1,
            ..Default::default()
        };
        let world = spawn_world(spawning);
        let spawned = collect_spawns(&world, 1234, 512);
        assert!(!spawned.is_empty());
    }

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
            let mut rng = JavaRandom::new(12345);
            let mut mob = Mob::new(idx as i32 + 1, mtype, 10.5, 64.0, -20.5, &mut rng);
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

            let nbt = mob.to_nbt((1, 2), (3, 4));
            let restored = Mob::from_nbt(mob.entity_id, &nbt, &mut rng).unwrap();
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
        let mut rng = JavaRandom::new(42);
        let mut creeper = Mob::new(1, MobType::Creeper, 0.0, 64.0, 0.0, &mut rng);
        let drops = creeper.get_death_drops(&mut rng, true);
        assert!(drops
            .iter()
            .any(|d| d.id == items::GOLD_RECORD || d.id == items::GREEN_RECORD));
    }

    #[test]
    fn the_pathfinder_does_not_go_through_the_water() {
        let mut world = World::new(WorldConfig {
            seed: 100,
            save_enabled: false,
            ..Default::default()
        });
        world.ensure_chunk_base(ChunkPos::new(0, 0));
        // A floor at y=63 and a pool of water in the middle of it.
        for x in 0..12 {
            for z in 0..12 {
                world.set_block(x, 63, z, blocks::STONE);
                world.set_block(x, 64, z, blocks::AIR);
                if (2..=5).contains(&x) {
                    world.set_block(x, 64, z, blocks::STATIONARY_WATER);
                }
            }
        }
        // The mob is in front of the pool, the goal is behind it.
        let path = find_path(&world, 0.5, 64.0, 5.5, 0.6, 0.9, 7.5, 64.5, 5.5, 16.0);
        assert!(!path.is_empty(), "the pathfinder should find a way around");
        for (x, y, z) in &path {
            // The water is not passable for the mobs of the reference.
            assert!(
                !((2..=5).contains(x) && *y == 64),
                "the path goes through the water at {x},{y},{z}"
            );
        }
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
        // The goal is the middle of the block (6, 65, 2), the mob is a pig
        // (0.6 wide and 0.9 high).
        let path = find_path(&world, 1.5, 64.0, 2.5, 0.6, 0.9, 6.5, 65.5, 2.5, 16.0);
        assert!(
            !path.is_empty(),
            "Pathfinder should find path up 1-block step"
        );
        let last = *path.last().unwrap();
        assert!((last.0 - 6).abs() <= 1 && last.1 == 65);
    }
}
