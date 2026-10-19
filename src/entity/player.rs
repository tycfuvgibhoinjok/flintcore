use std::collections::HashSet;

use crate::gameplay::inventory::{InventoryPlayer, OpenContainer};
use crate::gameplay::items::ItemStack;
use crate::net::ConnId;
use crate::world::chunk::ChunkPos;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiggingState {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub face: u8,
    pub start_tick: u64,
}

#[derive(Debug, Clone)]
pub struct Player {
    pub entity_id: i32,
    pub conn_id: ConnId,
    pub username: String,
    /// Java `EntityPlayer.dimension`: 0 = overworld, -1 = Nether.
    pub dimension: i32,
    /// Java `EntityPlayer.D`: portal cooldown in ticks after a dimension change.
    pub portal_cooldown: i32,
    /// Java `EntityPlayer.E`: touched a portal this tick, travel on the next tick.
    pub in_portal: bool,
    /// Java `EntityPlayer.F`: portal swirl timer (`+0.0125` per tick, travel at 1.0).
    pub portal_time: f32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The size of the box of the player (`Entity.length` and
    /// `Entity.width`).
    ///
    /// The player of the reference harness is created without the
    /// constructor, so it has no box at all (the size is zero). The scenarios
    /// that are compared with the reference use it.
    pub box_size: (f32, f32),
    pub prev_x: f64,
    pub prev_y: f64,
    pub prev_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub prev_yaw: f32,
    pub prev_pitch: f32,
    pub stance: f64,
    pub on_ground: bool,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub velocity_changed: bool,
    pub health: i16,
    pub last_health: i16,
    pub air_ticks: i16,
    pub fire_ticks: i16,
    pub fall_distance: f32,
    pub no_damage_ticks: u8,
    pub last_damage: i16,
    pub damage_remainder: i32,
    pub sneaking: bool,
    pub sleeping: bool,
    pub sleep_ticks: u8,
    pub vehicle_id: Option<i32>,
    pub bed_pos: Option<(i32, i32, i32)>,
    pub spawn_bed: Option<(i32, i32, i32)>,
    pub inventory: InventoryPlayer,
    pub open_container: Option<OpenContainer>,
    pub next_window_id: u8,
    pub editing_sign: Option<(i32, i32, i32)>,
    pub equipment_cache: [Option<ItemStack>; 5],
    /// `EntityPlayer.playerChunkCoordIntPairs`: chunks with active `PreChunk(true)` on the client.
    pub pre_chunk_sent: HashSet<ChunkPos>,
    /// Chunks whose `MapChunk` payload has been sent to the client.
    pub sent_chunks: HashSet<ChunkPos>,
    /// `EntityPlayer.chunkCoordIntPairQueue`: chunks waiting for `MapChunk` streaming.
    pub chunk_queue: Vec<ChunkPos>,
    pub last_eval_x: f64,
    pub last_eval_z: f64,
    pub digging: Option<DiggingState>,
    /// `ItemInWorldManager.i/j/k/l/m`: deferred break when client finishes digging early.
    pub pending_dig: Option<DiggingState>,
    pub last_place_tick: u64,
    pub last_place_item: Option<(u64, i16)>,
    pub last_drop_tick: u64,
    pub drop_count: u32,
    pub air_hover_ticks: u32,
    pub dead: bool,
}

impl Player {
    pub fn new(entity_id: i32, conn_id: ConnId, username: String, x: f64, y: f64, z: f64) -> Self {
        Self {
            entity_id,
            conn_id,
            username,
            dimension: 0,
            portal_cooldown: 0,
            in_portal: false,
            portal_time: 0.0,
            x,
            y,
            z,
            box_size: (0.6, 1.8),
            prev_x: x,
            prev_y: y,
            prev_z: z,
            yaw: 0.0,
            pitch: 0.0,
            prev_yaw: 0.0,
            prev_pitch: 0.0,
            stance: y,
            on_ground: false,
            mot_x: 0.0,
            mot_y: 0.0,
            mot_z: 0.0,
            velocity_changed: false,
            health: 20,
            last_health: 20,
            air_ticks: 300,
            fire_ticks: 0,
            fall_distance: 0.0,
            no_damage_ticks: 0,
            last_damage: 0,
            damage_remainder: 0,
            sneaking: false,
            sleeping: false,
            sleep_ticks: 0,
            vehicle_id: None,
            bed_pos: None,
            spawn_bed: None,
            inventory: InventoryPlayer::new(),
            open_container: None,
            next_window_id: 0,
            editing_sign: None,
            equipment_cache: [None; 5],
            pre_chunk_sent: HashSet::new(),
            sent_chunks: HashSet::new(),
            chunk_queue: Vec::new(),
            last_eval_x: x,
            last_eval_z: z,
            digging: None,
            pending_dig: None,
            last_place_tick: 0,
            last_place_item: None,
            last_drop_tick: 0,
            drop_count: 0,
            air_hover_ticks: 0,
            dead: false,
        }
    }

    pub fn next_window_id(&mut self) -> u8 {
        self.next_window_id = self.next_window_id % 100 + 1;
        self.next_window_id
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x as i32) >> 4, (self.z as i32) >> 4)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_position(
        &mut self,
        x: f64,
        y: f64,
        stance: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) {
        self.prev_x = self.x;
        self.prev_y = self.y;
        self.prev_z = self.z;
        self.prev_yaw = self.yaw;
        self.prev_pitch = self.pitch;

        self.x = x;
        self.y = y;
        self.stance = stance;
        self.z = z;
        self.yaw = yaw;
        self.pitch = pitch;
        self.on_ground = on_ground;
    }

    pub fn apply_knockback(&mut self, source_x: f64, source_z: f64) {
        let mut dx = self.x - source_x;
        let mut dz = self.z - source_z;
        let dist = (dx * dx + dz * dz).sqrt();
        if dist > 1e-4 {
            dx /= dist;
            dz /= dist;
            self.mot_x /= 2.0;
            self.mot_y /= 2.0;
            self.mot_z /= 2.0;
            self.mot_x += dx * 0.4;
            self.mot_y += 0.4;
            self.mot_z += dz * 0.4;
            self.velocity_changed = true;
        }
    }

    pub fn tick(&mut self) {
        if self.no_damage_ticks > 0 {
            self.no_damage_ticks -= 1;
        }
        if self.fire_ticks > 0 {
            self.fire_ticks -= 1;
        }
    }

    pub fn take_damage_direct(&mut self, amount: i16) -> (i16, bool) {
        if self.dead {
            return (0, false);
        }
        let eff_amount = if self.no_damage_ticks > 10 {
            if amount <= self.last_damage {
                return (0, false);
            }
            let diff = amount - self.last_damage;
            self.last_damage = amount;
            diff
        } else {
            self.last_damage = amount;
            self.no_damage_ticks = 20;
            amount
        };

        self.health = (self.health - eff_amount).max(0);

        let died = self.health == 0;
        if died {
            self.dead = true;
        }
        (eff_amount, died)
    }

    pub fn take_damage(&mut self, amount: i16) -> (i16, bool) {
        if self.dead {
            return (0, false);
        }
        let eff_amount = if self.no_damage_ticks > 10 {
            if amount <= self.last_damage {
                return (0, false);
            }
            let diff = amount - self.last_damage;
            self.last_damage = amount;
            diff
        } else {
            self.last_damage = amount;
            self.no_damage_ticks = 20;
            amount
        };

        // Armor defense reduction (`EntityHuman.c(int)` in `EntityHuman.java:498-506`)
        let armor_points = self.inventory.total_armor_points();
        let factor = (25 - armor_points).max(0);
        let k = (eff_amount as i32) * factor + self.damage_remainder;
        self.inventory.damage_armor(eff_amount);
        let actual_damage = (k / 25) as i16;
        self.damage_remainder = k % 25;

        self.health = (self.health - actual_damage).max(0);

        let died = self.health == 0;
        if died {
            self.dead = true;
        }
        (actual_damage, died)
    }

    pub fn heal(&mut self, amount: i16) {
        if !self.dead {
            self.health = (self.health + amount).min(20);
        }
    }

    pub fn respawn(&mut self, spawn_x: f64, spawn_y: f64, spawn_z: f64) {
        self.health = 20;
        self.last_health = 20;
        self.dead = false;
        self.x = spawn_x;
        self.y = spawn_y;
        self.stance = spawn_y;
        self.z = spawn_z;
        self.prev_x = spawn_x;
        self.prev_y = spawn_y;
        self.prev_z = spawn_z;
        self.last_eval_x = spawn_x;
        self.last_eval_z = spawn_z;
        self.fall_distance = 0.0;
        self.fire_ticks = 0;
        self.air_ticks = 300;
        self.no_damage_ticks = 0;
        self.last_damage = 0;
        self.damage_remainder = 0;
        self.mot_x = 0.0;
        self.mot_y = 0.0;
        self.mot_z = 0.0;
        self.velocity_changed = false;
        self.sneaking = false;
        self.sleeping = false;
        self.sleep_ticks = 0;
        self.vehicle_id = None;
        self.bed_pos = None;
        self.open_container = None;
        self.editing_sign = None;
        self.digging = None;
        self.pending_dig = None;
        self.pre_chunk_sent.clear();
        self.sent_chunks.clear();
        self.chunk_queue.clear();
    }

    pub fn equipment_changed(&self) -> Vec<(i16, Option<ItemStack>)> {
        let mut changes = Vec::new();
        for slot in 0..5 {
            let current = self.inventory.get_equipment(slot);
            if current != self.equipment_cache[slot as usize] {
                changes.push((slot, current));
            }
        }
        changes
    }

    pub fn sync_equipment_cache(&mut self) {
        for slot in 0..5 {
            self.equipment_cache[slot as usize] = self.inventory.get_equipment(slot);
        }
    }

    pub fn metadata_flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.fire_ticks > 0 {
            flags |= 0x01;
        }
        if self.sneaking {
            flags |= 0x02;
        }
        if self.vehicle_id.is_some() {
            flags |= 0x04;
        }
        flags
    }
}
