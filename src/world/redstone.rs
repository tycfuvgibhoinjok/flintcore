//! 1:1 port of Minecraft Beta 1.7.3 Redstone, Pistons, Dispensers, Rails, and Random Block Ticks
//! from `.references/Project-Poseidon`.

use std::collections::HashSet;

use crate::gameplay::blocks;
use crate::gameplay::items::{self, ItemStack};
use crate::gameplay::tile::TileEntityKind;
use crate::util::rng::JavaRandom;
use crate::world::chunk::{ChunkPos, CHUNK_HEIGHT};
use crate::world::world::World;

/// Events emitted by block ticks / redstone / pistons for the server simulation loop to broadcast or spawn.
#[derive(Debug, Clone)]
pub enum WorldEvent {
    DropItem {
        x: f64,
        y: f64,
        z: f64,
        item: ItemStack,
    },
    DispenseItem {
        x: f64,
        y: f64,
        z: f64,
        dx: i32,
        dz: i32,
        item: ItemStack,
    },
    DispenseProjectile {
        x: f64,
        y: f64,
        z: f64,
        dx: f64,
        dy: f64,
        dz: f64,
        item_id: i16,
    },
    SpawnPrimedTnt {
        x: f64,
        y: f64,
        z: f64,
        fuse: i32,
    },
    PlayNote {
        x: i32,
        y: i16,
        z: i32,
        instrument: u8,
        pitch: u8,
    },
    LevelSound {
        effect_id: i32,
        x: i32,
        y: u8,
        z: i32,
        data: i32,
    },
}

/// Java `RedstoneUpdateInfo`: tracks redstone torch toggle timestamps for burnout (>= 8 toggles in 100 ticks).
#[derive(Debug, Clone, Copy)]
pub struct RedstoneUpdateInfo {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub time: i64,
}

const PISTON_DX: [i32; 6] = [0, 0, 0, 0, -1, 1];
const PISTON_DY: [i32; 6] = [-1, 1, 0, 0, 0, 0];
const PISTON_DZ: [i32; 6] = [0, 0, -1, 1, 0, 0];

impl World {
    pub fn take_world_events(&mut self) -> Vec<WorldEvent> {
        std::mem::take(&mut self.world_events)
    }

    // ---------------------------------------------------------------------
    // Power queries (`World.isBlockPowered`, `isBlockIndirectlyPowered`, etc.)
    // ---------------------------------------------------------------------

    /// Java `Block.isPowerSource()`.
    pub fn is_power_source_block(&self, id: u8) -> bool {
        match id {
            blocks::REDSTONE_WIRE => self.wire_can_provide_power,
            blocks::REDSTONE_TORCH_OFF
            | blocks::REDSTONE_TORCH_ON
            | blocks::LEVER
            | blocks::STONE_BUTTON
            | blocks::STONE_PLATE
            | blocks::WOOD_PLATE
            | blocks::DETECTOR_RAIL => true,
            _ => false,
        }
    }

    /// Java `Block.a(IBlockAccess, i, j, k, l)` — direct / weak power output along `face`.
    #[allow(clippy::if_same_then_else)]
    pub fn block_direct_power(&self, x: i32, y: i32, z: i32, face: u8) -> bool {
        let id = self.get_block(x, y, z);
        match id {
            blocks::REDSTONE_TORCH_ON => {
                let meta = self.get_data(x, y, z);
                if meta == 5 && face == 1 {
                    false
                } else if meta == 3 && face == 3 {
                    false
                } else if meta == 4 && face == 2 {
                    false
                } else if meta == 1 && face == 5 {
                    false
                } else {
                    !(meta == 2 && face == 4)
                }
            }
            blocks::LEVER | blocks::STONE_BUTTON => (self.get_data(x, y, z) & 8) > 0,
            blocks::STONE_PLATE | blocks::WOOD_PLATE => self.get_data(x, y, z) > 0,
            blocks::DETECTOR_RAIL => (self.get_data(x, y, z) & 8) != 0,
            blocks::REPEATER_ON => {
                let dir = self.get_data(x, y, z) & 3;
                (dir == 0 && face == 3)
                    || (dir == 1 && face == 4)
                    || (dir == 2 && face == 2)
                    || (dir == 3 && face == 5)
            }
            blocks::REDSTONE_WIRE => {
                if !self.wire_can_provide_power {
                    return false;
                }
                if self.get_data(x, y, z) == 0 {
                    return false;
                }
                if face == 1 {
                    return true;
                }
                let mut f_xm = self.can_wire_connect(x - 1, y, z, 1)
                    || (!self.is_solid_normal_at(x - 1, y, z)
                        && self.can_wire_connect(x - 1, y - 1, z, -1));
                let mut f_xp = self.can_wire_connect(x + 1, y, z, 3)
                    || (!self.is_solid_normal_at(x + 1, y, z)
                        && self.can_wire_connect(x + 1, y - 1, z, -1));
                let mut f_zm = self.can_wire_connect(x, y, z - 1, 2)
                    || (!self.is_solid_normal_at(x, y, z - 1)
                        && self.can_wire_connect(x, y - 1, z - 1, -1));
                let mut f_zp = self.can_wire_connect(x, y, z + 1, 0)
                    || (!self.is_solid_normal_at(x, y, z + 1)
                        && self.can_wire_connect(x, y - 1, z + 1, -1));

                if !self.is_solid_normal_at(x, y + 1, z) {
                    if self.is_solid_normal_at(x - 1, y, z)
                        && self.can_wire_connect(x - 1, y + 1, z, -1)
                    {
                        f_xm = true;
                    }
                    if self.is_solid_normal_at(x + 1, y, z)
                        && self.can_wire_connect(x + 1, y + 1, z, -1)
                    {
                        f_xp = true;
                    }
                    if self.is_solid_normal_at(x, y, z - 1)
                        && self.can_wire_connect(x, y + 1, z - 1, -1)
                    {
                        f_zm = true;
                    }
                    if self.is_solid_normal_at(x, y, z + 1)
                        && self.can_wire_connect(x, y + 1, z + 1, -1)
                    {
                        f_zp = true;
                    }
                }

                if !f_zm && !f_xp && !f_xm && !f_zp && (2..=5).contains(&face) {
                    true
                } else if face == 2 && f_zm && !f_xm && !f_xp {
                    true
                } else if face == 3 && f_zp && !f_xm && !f_xp {
                    true
                } else if face == 4 && f_xm && !f_zm && !f_zp {
                    true
                } else {
                    face == 5 && f_xp && !f_zm && !f_zp
                }
            }
            _ => false,
        }
    }

    /// Java `Block.d(World, i, j, k, l)` — strong power output along `face` (capable of powering a solid block).
    pub fn block_strong_power(&self, x: i32, y: i32, z: i32, face: u8) -> bool {
        let id = self.get_block(x, y, z);
        match id {
            blocks::REDSTONE_TORCH_ON => face == 0 && self.block_direct_power(x, y, z, 0),
            blocks::LEVER => {
                let meta = self.get_data(x, y, z);
                if (meta & 8) == 0 {
                    return false;
                }
                let dir = meta & 7;
                (dir == 6 && face == 1)
                    || (dir == 5 && face == 1)
                    || (dir == 4 && face == 2)
                    || (dir == 3 && face == 3)
                    || (dir == 2 && face == 4)
                    || (dir == 1 && face == 5)
            }
            blocks::STONE_BUTTON => {
                let meta = self.get_data(x, y, z);
                if (meta & 8) == 0 {
                    return false;
                }
                let dir = meta & 7;
                (dir == 4 && face == 2)
                    || (dir == 3 && face == 3)
                    || (dir == 2 && face == 4)
                    || (dir == 1 && face == 5)
            }
            blocks::STONE_PLATE | blocks::WOOD_PLATE => self.get_data(x, y, z) > 0 && face == 1,
            blocks::DETECTOR_RAIL => (self.get_data(x, y, z) & 8) != 0 && face == 1,
            blocks::REPEATER_ON => self.block_direct_power(x, y, z, face),
            blocks::REDSTONE_WIRE => {
                self.wire_can_provide_power && self.block_direct_power(x, y, z, face)
            }
            _ => false,
        }
    }

    /// Java `BlockRedstoneWire.c(IBlockAccess, i, j, k, dir)`.
    pub fn can_wire_connect(&self, x: i32, y: i32, z: i32, dir: i32) -> bool {
        let id = self.get_block(x, y, z);
        if id == blocks::REDSTONE_WIRE {
            return true;
        }
        if id == blocks::AIR {
            return false;
        }
        if self.is_power_source_block(id) {
            return true;
        }
        if id == blocks::REPEATER_OFF || id == blocks::REPEATER_ON {
            let meta_dir = (self.get_data(x, y, z) & 3) as i32;
            let opp = match meta_dir {
                0 => 2,
                1 => 3,
                2 => 0,
                _ => 1,
            };
            return dir == meta_dir || dir == opp;
        }
        false
    }

    /// Java `World.isBlockFacePowered(i, j, k, l)`.
    pub fn is_block_face_powered(&self, x: i32, y: i32, z: i32, face: u8) -> bool {
        self.block_strong_power(x, y, z, face)
    }

    /// Java `World.isBlockPowered(i, j, k)`.
    pub fn is_block_powered(&self, x: i32, y: i32, z: i32) -> bool {
        self.is_block_face_powered(x, y - 1, z, 0)
            || self.is_block_face_powered(x, y + 1, z, 1)
            || self.is_block_face_powered(x, y, z - 1, 2)
            || self.is_block_face_powered(x, y, z + 1, 3)
            || self.is_block_face_powered(x - 1, y, z, 4)
            || self.is_block_face_powered(x + 1, y, z, 5)
    }

    /// Java `World.isBlockFaceIndirectlyPowered(i, j, k, l)`.
    pub fn is_block_face_indirectly_powered(&self, x: i32, y: i32, z: i32, face: u8) -> bool {
        if self.is_solid_normal_at(x, y, z) {
            self.is_block_powered(x, y, z)
        } else {
            self.block_direct_power(x, y, z, face)
        }
    }

    /// Java `World.isBlockIndirectlyPowered(i, j, k)`.
    pub fn is_block_indirectly_powered(&self, x: i32, y: i32, z: i32) -> bool {
        self.is_block_face_indirectly_powered(x, y - 1, z, 0)
            || self.is_block_face_indirectly_powered(x, y + 1, z, 1)
            || self.is_block_face_indirectly_powered(x, y, z - 1, 2)
            || self.is_block_face_indirectly_powered(x, y, z + 1, 3)
            || self.is_block_face_indirectly_powered(x - 1, y, z, 4)
            || self.is_block_face_indirectly_powered(x + 1, y, z, 5)
    }

    // ---------------------------------------------------------------------
    // BlockRedstoneWire (55)
    // ---------------------------------------------------------------------

    fn wire_power_at(&self, x: i32, y: i32, z: i32, cur_max: i32) -> i32 {
        if self.get_block(x, y, z) != blocks::REDSTONE_WIRE {
            cur_max
        } else {
            (self.get_data(x, y, z) as i32).max(cur_max)
        }
    }

    /// Java `BlockRedstoneWire.g(World, i, j, k)`.
    pub fn update_redstone_wire(&mut self, x: i32, y: i32, z: i32) {
        let mut to_notify = HashSet::new();
        self.calculate_wire_changes(x, y, z, x, y, z, &mut to_notify);
        for (nx, ny, nz) in to_notify {
            self.java_apply_physics(nx, ny, nz, blocks::REDSTONE_WIRE as i32);
        }
    }

    /// Java `BlockRedstoneWire.a(World, i, j, k, l, i1, j1)`.
    #[allow(clippy::too_many_arguments)]
    fn calculate_wire_changes(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        sx: i32,
        sy: i32,
        sz: i32,
        to_notify: &mut HashSet<(i32, i32, i32)>,
    ) {
        let old_power = self.get_data(x, y, z) as i32;
        let mut new_power = 0i32;

        self.wire_can_provide_power = false;
        let powered_indirectly = self.is_block_indirectly_powered(x, y, z);
        self.wire_can_provide_power = true;

        if powered_indirectly {
            new_power = 15;
        } else {
            for dir in 0..4 {
                let mut nx = x;
                let mut nz = z;
                match dir {
                    0 => nx -= 1,
                    1 => nx += 1,
                    2 => nz -= 1,
                    _ => nz += 1,
                }
                if nx != sx || y != sy || nz != sz {
                    new_power = self.wire_power_at(nx, y, nz, new_power);
                }
                if self.is_solid_normal_at(nx, y, nz) && !self.is_solid_normal_at(x, y + 1, z) {
                    if nx != sx || y + 1 != sy || nz != sz {
                        new_power = self.wire_power_at(nx, y + 1, nz, new_power);
                    }
                } else if !self.is_solid_normal_at(nx, y, nz)
                    && (nx != sx || y - 1 != sy || nz != sz)
                {
                    new_power = self.wire_power_at(nx, y - 1, nz, new_power);
                }
            }
            if new_power > 0 {
                new_power -= 1;
            } else {
                new_power = 0;
            }
        }

        if old_power != new_power {
            let prev_suppress = self.suppress_physics;
            self.suppress_physics = true;
            self.java_set_data(x, y, z, new_power as u8);
            self.suppress_physics = prev_suppress;

            for dir in 0..4 {
                let mut nx = x;
                let mut nz = z;
                let mut ny = y - 1;
                match dir {
                    0 => nx -= 1,
                    1 => nx += 1,
                    2 => nz -= 1,
                    _ => nz += 1,
                }
                if self.is_solid_normal_at(nx, y, nz) {
                    ny += 2;
                }

                let adj_power = self.wire_power_at(nx, y, nz, -1);
                let cur = (self.get_data(x, y, z) as i32).saturating_sub(1);
                if adj_power >= 0 && adj_power != cur {
                    self.calculate_wire_changes(nx, y, nz, x, y, z, to_notify);
                }

                let diag_power = self.wire_power_at(nx, ny, nz, -1);
                let cur2 = (self.get_data(x, y, z) as i32).saturating_sub(1);
                if diag_power >= 0 && diag_power != cur2 {
                    self.calculate_wire_changes(nx, ny, nz, x, y, z, to_notify);
                }
            }

            if old_power == 0 || new_power == 0 {
                to_notify.insert((x, y, z));
                to_notify.insert((x - 1, y, z));
                to_notify.insert((x + 1, y, z));
                to_notify.insert((x, y - 1, z));
                to_notify.insert((x, y + 1, z));
                to_notify.insert((x, y, z - 1));
                to_notify.insert((x, y, z + 1));
            }
        }
    }

    pub fn notify_wire_neighbors_at(&mut self, x: i32, y: i32, z: i32) {
        if self.get_block(x, y, z) == blocks::REDSTONE_WIRE {
            let id = blocks::REDSTONE_WIRE as i32;
            self.java_apply_physics(x, y, z, id);
            self.java_apply_physics(x - 1, y, z, id);
            self.java_apply_physics(x + 1, y, z, id);
            self.java_apply_physics(x, y, z - 1, id);
            self.java_apply_physics(x, y, z + 1, id);
            self.java_apply_physics(x, y - 1, z, id);
            self.java_apply_physics(x, y + 1, z, id);
        }
    }

    pub fn on_wire_place_or_remove(&mut self, x: i32, y: i32, z: i32) {
        self.update_redstone_wire(x, y, z);
        self.java_apply_physics(x, y + 1, z, blocks::REDSTONE_WIRE as i32);
        self.java_apply_physics(x, y - 1, z, blocks::REDSTONE_WIRE as i32);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            self.notify_wire_neighbors_at(x + dx, y, z + dz);
            if self.is_solid_normal_at(x + dx, y, z + dz) {
                self.notify_wire_neighbors_at(x + dx, y + 1, z + dz);
            } else {
                self.notify_wire_neighbors_at(x + dx, y - 1, z + dz);
            }
        }
    }

    // ---------------------------------------------------------------------
    // BlockRedstoneTorch (75 / 76)
    // ---------------------------------------------------------------------

    fn is_torch_input_powered(&self, x: i32, y: i32, z: i32) -> bool {
        let meta = self.get_data(x, y, z);
        (meta == 5 && self.is_block_face_indirectly_powered(x, y - 1, z, 0))
            || (meta == 3 && self.is_block_face_indirectly_powered(x, y, z - 1, 2))
            || (meta == 4 && self.is_block_face_indirectly_powered(x, y, z + 1, 3))
            || (meta == 1 && self.is_block_face_indirectly_powered(x - 1, y, z, 4))
            || (meta == 2 && self.is_block_face_indirectly_powered(x + 1, y, z, 5))
    }

    fn check_torch_burnout(&mut self, x: i32, y: i32, z: i32, record: bool) -> bool {
        if record {
            self.torch_burnout_list.push(RedstoneUpdateInfo {
                x,
                y,
                z,
                time: self.live_time,
            });
        }
        let mut count = 0;
        for info in &self.torch_burnout_list {
            if info.x == x && info.y == y && info.z == z {
                count += 1;
                if count >= 8 {
                    return true;
                }
            }
        }
        false
    }

    pub fn tick_redstone_torch(&mut self, x: i32, y: i32, z: i32, is_on: bool) {
        let input_powered = self.is_torch_input_powered(x, y, z);
        let cur_time = self.live_time;
        self.torch_burnout_list
            .retain(|info| cur_time - info.time <= 100);

        let meta = self.get_data(x, y, z);
        if is_on {
            if input_powered {
                self.java_set_type_id_and_data(x, y, z, blocks::REDSTONE_TORCH_OFF, meta);
                if self.check_torch_burnout(x, y, z, true) {
                    self.world_events.push(WorldEvent::LevelSound {
                        effect_id: 1004,
                        x,
                        y: y as u8,
                        z,
                        data: 0,
                    });
                }
            }
        } else if !input_powered && !self.check_torch_burnout(x, y, z, false) {
            self.java_set_type_id_and_data(x, y, z, blocks::REDSTONE_TORCH_ON, meta);
        }
    }

    pub fn notify_torch_neighbors(&mut self, x: i32, y: i32, z: i32, id: u8) {
        let bid = id as i32;
        self.java_apply_physics(x, y - 1, z, bid);
        self.java_apply_physics(x, y + 1, z, bid);
        self.java_apply_physics(x - 1, y, z, bid);
        self.java_apply_physics(x + 1, y, z, bid);
        self.java_apply_physics(x, y, z - 1, bid);
        self.java_apply_physics(x, y, z + 1, bid);
    }

    // ---------------------------------------------------------------------
    // BlockDiode / Repeater (93 / 94)
    // ---------------------------------------------------------------------

    pub fn is_repeater_receiving_power(&self, x: i32, y: i32, z: i32, meta: u8) -> bool {
        match meta & 3 {
            0 => {
                self.is_block_face_indirectly_powered(x, y, z + 1, 3)
                    || (self.get_block(x, y, z + 1) == blocks::REDSTONE_WIRE
                        && self.get_data(x, y, z + 1) > 0)
            }
            1 => {
                self.is_block_face_indirectly_powered(x - 1, y, z, 4)
                    || (self.get_block(x - 1, y, z) == blocks::REDSTONE_WIRE
                        && self.get_data(x - 1, y, z) > 0)
            }
            2 => {
                self.is_block_face_indirectly_powered(x, y, z - 1, 2)
                    || (self.get_block(x, y, z - 1) == blocks::REDSTONE_WIRE
                        && self.get_data(x, y, z - 1) > 0)
            }
            _ => {
                self.is_block_face_indirectly_powered(x + 1, y, z, 5)
                    || (self.get_block(x + 1, y, z) == blocks::REDSTONE_WIRE
                        && self.get_data(x + 1, y, z) > 0)
            }
        }
    }

    pub fn repeater_delay(meta: u8) -> i64 {
        const DELAYS: [i64; 4] = [2, 4, 6, 8];
        DELAYS[((meta & 12) >> 2) as usize]
    }

    pub fn tick_repeater(&mut self, x: i32, y: i32, z: i32, is_on: bool) {
        let meta = self.get_data(x, y, z);
        let powered = self.is_repeater_receiving_power(x, y, z, meta);
        if is_on && !powered {
            self.java_set_type_id_and_data(x, y, z, blocks::REPEATER_OFF, meta);
        } else if !is_on {
            self.java_set_type_id_and_data(x, y, z, blocks::REPEATER_ON, meta);
            if !powered {
                self.java_schedule_tick(x, y, z, blocks::REPEATER_ON, Self::repeater_delay(meta));
            }
        }
    }

    pub fn do_physics_repeater(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if !self.is_solid_normal_at(x, y - 1, z) {
            self.java_set_type_id(x, y, z, blocks::AIR);
            self.world_events.push(WorldEvent::DropItem {
                x: x as f64,
                y: y as f64,
                z: z as f64,
                item: ItemStack::new(items::DIODE, 1, 0),
            });
            return;
        }
        let meta = self.get_data(x, y, z);
        let powered = self.is_repeater_receiving_power(x, y, z, meta);
        let is_on = id == blocks::REPEATER_ON;
        if (is_on && !powered) || (!is_on && powered) {
            self.java_schedule_tick(x, y, z, id, Self::repeater_delay(meta));
        }
    }

    // ---------------------------------------------------------------------
    // Lever (69), Button (77), Pressure Plates (70 / 72), Detector Rail (28)
    // ---------------------------------------------------------------------

    pub fn notify_attachment_neighbors(&mut self, x: i32, y: i32, z: i32, id: u8, dir: u8) {
        let bid = id as i32;
        self.java_apply_physics(x, y, z, bid);
        match dir {
            1 => self.java_apply_physics(x - 1, y, z, bid),
            2 => self.java_apply_physics(x + 1, y, z, bid),
            3 => self.java_apply_physics(x, y, z - 1, bid),
            4 => self.java_apply_physics(x, y, z + 1, bid),
            _ => self.java_apply_physics(x, y - 1, z, bid),
        }
    }

    pub fn tick_button(&mut self, x: i32, y: i32, z: i32) {
        let meta = self.get_data(x, y, z);
        if (meta & 8) != 0 {
            let dir = meta & 7;
            self.java_set_data(x, y, z, dir);
            self.notify_attachment_neighbors(x, y, z, blocks::STONE_BUTTON, dir);
        }
    }

    pub fn set_pressure_plate_state(&mut self, x: i32, y: i32, z: i32, id: u8, pressed: bool) {
        let was_pressed = self.get_data(x, y, z) == 1;
        if pressed && !was_pressed {
            self.java_set_data(x, y, z, 1);
            self.java_apply_physics(x, y, z, id as i32);
            self.java_apply_physics(x, y - 1, z, id as i32);
        } else if !pressed && was_pressed {
            self.java_set_data(x, y, z, 0);
            self.java_apply_physics(x, y, z, id as i32);
            self.java_apply_physics(x, y - 1, z, id as i32);
        }
        if pressed {
            self.java_schedule_tick(x, y, z, id, 20);
        }
    }

    // ---------------------------------------------------------------------
    // Doors (64 / 71), Trapdoor (96), NoteBlock (25), Dispenser (23), TNT (46)
    // ---------------------------------------------------------------------

    pub fn power_door(&mut self, x: i32, y: i32, z: i32, door_id: u8, powered: bool) {
        let meta = self.get_data(x, y, z);
        if (meta & 8) != 0 {
            if self.get_block(x, y - 1, z) == door_id {
                self.power_door(x, y - 1, z, door_id, powered);
            }
            return;
        }
        let is_open = (meta & 4) > 0;
        if is_open != powered {
            let new_meta = meta ^ 4;
            self.java_set_data(x, y, z, new_meta);
            if self.get_block(x, y + 1, z) == door_id {
                self.java_set_data(x, y + 1, z, new_meta | 8);
            }
            self.world_events.push(WorldEvent::LevelSound {
                effect_id: 1003,
                x,
                y: y as u8,
                z,
                data: 0,
            });
        }
    }

    pub fn power_trapdoor(&mut self, x: i32, y: i32, z: i32, powered: bool) {
        let meta = self.get_data(x, y, z);
        let is_open = (meta & 4) > 0;
        if is_open != powered {
            self.java_set_data(x, y, z, meta ^ 4);
            self.world_events.push(WorldEvent::LevelSound {
                effect_id: 1003,
                x,
                y: y as u8,
                z,
                data: 0,
            });
        }
    }

    pub fn trigger_note_block(&mut self, x: i32, y: i32, z: i32) {
        if self.get_block(x, y + 1, z) != blocks::AIR {
            return;
        }
        let note = match self.get_tile_entity(x, y, z) {
            Some(te) => match &te.kind {
                TileEntityKind::Note { note, .. } => *note,
                _ => 0,
            },
            None => 0,
        };
        let below_mat = blocks::material(self.get_block(x, y - 1, z));
        let instrument = match below_mat {
            blocks::Material::Stone => 1,
            blocks::Material::Sand => 2,
            blocks::Material::Shatterable => 3,
            blocks::Material::Wood => 4,
            _ => 0,
        };
        self.world_events.push(WorldEvent::PlayNote {
            x,
            y: y as i16,
            z,
            instrument,
            pitch: note,
        });
    }

    pub fn tick_dispenser(&mut self, x: i32, y: i32, z: i32) {
        let meta = self.get_data(x, y, z);
        let (dx, dz) = match meta {
            2 => (0, -1),
            3 => (0, 1),
            4 => (-1, 0),
            _ => (1, 0),
        };
        let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
        let mut chosen_slot: Option<usize> = None;
        let mut count_non_empty = 0i32;

        if let Some(te) = self.get_tile_entity(x, y, z) {
            if let TileEntityKind::Dispenser { items } = &te.kind {
                for (idx, slot) in items.iter().enumerate() {
                    if slot.is_some() {
                        count_non_empty += 1;
                        if rng.next_i32_bound(count_non_empty) == 0 {
                            chosen_slot = Some(idx);
                        }
                    }
                }
            }
        }
        self.world_random = rng;

        let mut dispensed: Option<ItemStack> = None;
        if let Some(slot_idx) = chosen_slot {
            if let Some(te) = self.get_tile_entity_mut(x, y, z) {
                if let TileEntityKind::Dispenser { items } = &mut te.kind {
                    if let Some(mut stack) = items[slot_idx] {
                        dispensed = Some(ItemStack::new(stack.id, 1, stack.damage));
                        stack.count = stack.count.saturating_sub(1);
                        items[slot_idx] = if stack.count > 0 { Some(stack) } else { None };
                    }
                }
            }
        }

        let Some(item) = dispensed else {
            self.world_events.push(WorldEvent::LevelSound {
                effect_id: 1001,
                x,
                y: y as u8,
                z,
                data: 0,
            });
            return;
        };

        let sx = x as f64 + (dx as f64) * 0.6 + 0.5;
        let sy = y as f64 + 0.5;
        let sz = z as f64 + (dz as f64) * 0.6 + 0.5;

        if item.id == items::ARROW || item.id == items::EGG || item.id == items::SNOW_BALL {
            self.world_events.push(WorldEvent::DispenseProjectile {
                x: sx,
                y: sy,
                z: sz,
                dx: dx as f64,
                dy: 0.1,
                dz: dz as f64,
                item_id: item.id,
            });
            self.world_events.push(WorldEvent::LevelSound {
                effect_id: 1002,
                x,
                y: y as u8,
                z,
                data: 0,
            });
        } else {
            self.world_events.push(WorldEvent::DispenseItem {
                x: sx,
                y: sy - 0.3,
                z: sz,
                dx,
                dz,
                item,
            });
            self.world_events.push(WorldEvent::LevelSound {
                effect_id: 1000,
                x,
                y: y as u8,
                z,
                data: 0,
            });
        }
        self.world_events.push(WorldEvent::LevelSound {
            effect_id: 2000,
            x,
            y: y as u8,
            z,
            data: (dx + 1) + (dz + 1) * 3,
        });
    }

    pub fn ignite_tnt(&mut self, x: i32, y: i32, z: i32, short_fuse: bool) {
        if self.get_block(x, y, z) == blocks::TNT {
            self.java_set_type_id(x, y, z, blocks::AIR);
        }
        let fuse = if short_fuse {
            10 + (self.live_time.abs() as i32 % 20)
        } else {
            80
        };
        self.world_events.push(WorldEvent::SpawnPrimedTnt {
            x: x as f64 + 0.5,
            y: y as f64 + 0.5,
            z: z as f64 + 0.5,
            fuse,
        });
    }

    // ---------------------------------------------------------------------
    // BlockPiston (33 / 29) & BlockPistonExtension (34)
    // ---------------------------------------------------------------------

    fn is_piston_powered(&self, x: i32, y: i32, z: i32, dir: usize) -> bool {
        if dir != 0 && self.is_block_face_indirectly_powered(x, y - 1, z, 0) {
            return true;
        }
        if dir != 1 && self.is_block_face_indirectly_powered(x, y + 1, z, 1) {
            return true;
        }
        if dir != 2 && self.is_block_face_indirectly_powered(x, y, z - 1, 2) {
            return true;
        }
        if dir != 3 && self.is_block_face_indirectly_powered(x, y, z + 1, 3) {
            return true;
        }
        if dir != 5 && self.is_block_face_indirectly_powered(x + 1, y, z, 5) {
            return true;
        }
        if dir != 4 && self.is_block_face_indirectly_powered(x - 1, y, z, 4) {
            return true;
        }
        // Quasi-connectivity (`BlockPiston.f` lines 106-108)
        self.is_block_face_indirectly_powered(x, y, z, 0)
            || self.is_block_face_indirectly_powered(x, y + 2, z, 1)
            || self.is_block_face_indirectly_powered(x, y + 1, z - 1, 2)
            || self.is_block_face_indirectly_powered(x, y + 1, z + 1, 3)
            || self.is_block_face_indirectly_powered(x - 1, y + 1, z, 4)
            || self.is_block_face_indirectly_powered(x + 1, y + 1, z, 5)
    }

    /// Java `Block.e()` piston mobility reaction:
    /// `0` = normal push/pull, `1` = breaks and drops item when pushed, `2` = immovable.
    fn block_piston_mobility(id: u8) -> u8 {
        if id == blocks::AIR {
            return 0;
        }
        if id == blocks::OBSIDIAN
            || id == blocks::BEDROCK
            || id == blocks::PORTAL
            || id == blocks::PISTON_HEAD
            || id == blocks::PISTON_MOVING
        {
            return 2;
        }
        let mat = blocks::material(id);
        match mat {
            blocks::Material::Plant
            | blocks::Material::Orientable
            | blocks::Material::Water
            | blocks::Material::Lava
            | blocks::Material::Fire
            | blocks::Material::SnowLayer
            | blocks::Material::Cactus
            | blocks::Material::Cake
            | blocks::Material::Web => 1,
            _ => {
                if id == blocks::WOODEN_DOOR
                    || id == blocks::IRON_DOOR
                    || id == blocks::STONE_PLATE
                    || id == blocks::WOOD_PLATE
                    || id == blocks::BED
                {
                    1
                } else {
                    0
                }
            }
        }
    }

    fn piston_can_push_block(&self, id: u8, x: i32, y: i32, z: i32, allow_fragile: bool) -> bool {
        if id == blocks::AIR {
            return true;
        }
        if id == blocks::OBSIDIAN
            || id == blocks::BEDROCK
            || id == blocks::PORTAL
            || id == blocks::FURNACE
            || id == blocks::LIT_FURNACE
            || id == blocks::CHEST
            || id == blocks::DISPENSER
            || id == blocks::NOTE_BLOCK
            || id == blocks::JUKEBOX
            || id == blocks::SIGN
            || id == blocks::WALL_SIGN
            || id == blocks::MOB_SPAWNER
        {
            return false;
        }
        if id == blocks::PISTON || id == blocks::STICKY_PISTON {
            return (self.get_data(x, y, z) & 8) == 0;
        }
        if blocks::hardness(id) < 0.0 {
            return false;
        }
        let mob = Self::block_piston_mobility(id);
        if mob == 2 {
            return false;
        }
        if !allow_fragile && mob == 1 {
            return false;
        }
        self.get_tile_entity(x, y, z).is_none()
    }

    pub fn update_piston_state(&mut self, x: i32, y: i32, z: i32, piston_id: u8) {
        let meta = self.get_data(x, y, z);
        let dir = (meta & 7) as usize;
        if dir > 5 {
            return;
        }
        let is_extended = (meta & 8) != 0;
        let powered = self.is_piston_powered(x, y, z, dir);
        let is_sticky = piston_id == blocks::STICKY_PISTON;

        if powered && !is_extended {
            if self.try_extend_piston(x, y, z, dir, is_sticky) {
                self.java_set_data(x, y, z, (dir as u8) | 8);
                self.world_events.push(WorldEvent::PlayNote {
                    x,
                    y: y as i16,
                    z,
                    instrument: 0,
                    pitch: dir as u8,
                });
            }
        } else if !powered && is_extended {
            self.retract_piston(x, y, z, dir, is_sticky);
            self.java_set_data(x, y, z, dir as u8);
            self.world_events.push(WorldEvent::PlayNote {
                x,
                y: y as i16,
                z,
                instrument: 1,
                pitch: dir as u8,
            });
        }
    }

    fn try_extend_piston(&mut self, x: i32, y: i32, z: i32, dir: usize, is_sticky: bool) -> bool {
        let dx = PISTON_DX[dir];
        let dy = PISTON_DY[dir];
        let dz = PISTON_DZ[dir];

        let mut cx = x + dx;
        let mut cy = y + dy;
        let mut cz = z + dz;
        let mut count = 0usize;

        while count < 13 {
            if cy <= 0 || cy >= 127 {
                return false;
            }
            let id = self.get_block(cx, cy, cz);
            if id == blocks::AIR {
                break;
            }
            if !self.piston_can_push_block(id, cx, cy, cz, true) {
                return false;
            }
            if Self::block_piston_mobility(id) == 1 {
                // Fragile block breaks and drops item, terminating the push chain
                let meta = self.get_data(cx, cy, cz);
                let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
                let drops = blocks::get_block_drops(id, meta, &mut rng, None);
                self.world_random = rng;
                for item in drops {
                    self.world_events.push(WorldEvent::DropItem {
                        x: cx as f64,
                        y: cy as f64,
                        z: cz as f64,
                        item,
                    });
                }
                self.java_set_type_id(cx, cy, cz, blocks::AIR);
                break;
            }
            if count == 12 {
                return false;
            }
            cx += dx;
            cy += dy;
            cz += dz;
            count += 1;
        }

        // Shift pushed blocks from back to front by 1 block
        for step in (0..count).rev() {
            let sx = x + dx * (step as i32 + 1);
            let sy = y + dy * (step as i32 + 1);
            let sz = z + dz * (step as i32 + 1);
            let tx = sx + dx;
            let ty = sy + dy;
            let tz = sz + dz;
            let bid = self.get_block(sx, sy, sz);
            let bmeta = self.get_data(sx, sy, sz);
            self.java_set_type_id_and_data(tx, ty, tz, bid, bmeta);
        }

        // Place Piston Extension (34) at (x + dx, y + dy, z + dz)
        let ext_meta = (dir as u8) | if is_sticky { 8 } else { 0 };
        self.java_set_type_id_and_data(x + dx, y + dy, z + dz, blocks::PISTON_HEAD, ext_meta);
        true
    }

    fn retract_piston(&mut self, x: i32, y: i32, z: i32, dir: usize, is_sticky: bool) {
        let dx = PISTON_DX[dir];
        let dy = PISTON_DY[dir];
        let dz = PISTON_DZ[dir];
        let hx = x + dx;
        let hy = y + dy;
        let hz = z + dz;

        if self.get_block(hx, hy, hz) == blocks::PISTON_HEAD {
            self.java_set_type_id(hx, hy, hz, blocks::AIR);
        }

        if is_sticky {
            let px = x + dx * 2;
            let py = y + dy * 2;
            let pz = z + dz * 2;
            if (0..128).contains(&py) {
                let pull_id = self.get_block(px, py, pz);
                let pull_meta = self.get_data(px, py, pz);
                if pull_id != blocks::AIR
                    && self.piston_can_push_block(pull_id, px, py, pz, false)
                    && (Self::block_piston_mobility(pull_id) == 0
                        || pull_id == blocks::PISTON
                        || pull_id == blocks::STICKY_PISTON)
                {
                    self.java_set_type_id(px, py, pz, blocks::AIR);
                    self.java_set_type_id_and_data(hx, hy, hz, pull_id, pull_meta);
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // Rails (66 / 27 / 28)
    // ---------------------------------------------------------------------

    pub fn update_powered_rail(&mut self, x: i32, y: i32, z: i32) {
        let meta = self.get_data(x, y, z);
        let was_powered = (meta & 8) != 0;
        let powered = self.is_block_indirectly_powered(x, y, z);
        if was_powered != powered {
            let new_meta = if powered { meta | 8 } else { meta & 7 };
            self.java_set_data(x, y, z, new_meta);
            self.java_apply_physics(x, y - 1, z, blocks::POWERED_RAIL as i32);
        }
    }

    // ---------------------------------------------------------------------
    // Random Block Ticks (`World.j()`)
    // ---------------------------------------------------------------------

    pub fn tick_random_blocks(&mut self, player_positions: &[(f64, f64)]) {
        if player_positions.is_empty() {
            return;
        }

        let mut active_chunks = HashSet::new();
        for &(px, pz) in player_positions {
            let cx = (px.floor() as i32) >> 4;
            let cz = (pz.floor() as i32) >> 4;
            for dx in -9..=9 {
                for dz in -9..=9 {
                    let pos = ChunkPos::new(cx + dx, cz + dz);
                    if self.chunks.contains_key(&pos) {
                        active_chunks.insert(pos);
                    }
                }
            }
        }

        let mut lcg = self.random_tick_lcg;
        let mut rng = std::mem::replace(&mut self.tick_random, JavaRandom::new(0));

        for pos in active_chunks {
            let base_x = pos.x * 16;
            let base_z = pos.z * 16;
            for _ in 0..80 {
                lcg = lcg.wrapping_mul(3).wrapping_add(1_013_904_223);
                let k = lcg >> 2;
                let lx = (k & 15) as usize;
                let lz = ((k >> 8) & 15) as usize;
                let y = (k >> 16) & 127;

                let block_id = match self.chunks.get(&pos) {
                    Some(c) => c.get_block(lx, y as usize, lz),
                    None => continue,
                };
                if block_id == blocks::AIR {
                    continue;
                }
                let wx = base_x + lx as i32;
                let wz = base_z + lz as i32;

                match block_id {
                    blocks::GRASS => {
                        // BlockGrass.a: decay under dark/opaque block or spread to nearby dirt
                        if y < 127
                            && self.light_at(wx, y + 1, wz) < 4
                            && blocks::light_opacity(self.get_block(wx, y + 1, wz)) > 2
                        {
                            if rng.next_i32_bound(4) == 0 {
                                self.java_set_type_id(wx, y, wz, blocks::DIRT);
                            }
                        } else if y < 127 && self.light_at(wx, y + 1, wz) >= 9 {
                            let tx = wx + rng.next_i32_bound(3) - 1;
                            let ty = y + rng.next_i32_bound(5) - 3;
                            let tz = wz + rng.next_i32_bound(3) - 1;
                            if (0..127).contains(&ty)
                                && self.get_block(tx, ty, tz) == blocks::DIRT
                                && self.light_at(tx, ty + 1, tz) >= 4
                                && blocks::light_opacity(self.get_block(tx, ty + 1, tz)) <= 2
                            {
                                self.java_set_type_id(tx, ty, tz, blocks::GRASS);
                            }
                        }
                    }
                    blocks::CROPS => {
                        if y < 127 && self.light_at(wx, y + 1, wz) >= 9 {
                            let meta = self.get_data(wx, y, wz);
                            if meta < 7 && rng.next_i32_bound(10) == 0 {
                                self.java_set_data(wx, y, wz, meta + 1);
                            }
                        }
                    }
                    blocks::SAPLING => {
                        if y < 127
                            && self.light_at(wx, y + 1, wz) >= 9
                            && rng.next_i32_bound(7) == 0
                        {
                            let meta = self.get_data(wx, y, wz);
                            if (meta & 8) == 0 {
                                self.java_set_data(wx, y, wz, meta | 8);
                            } else {
                                let _ = self.grow_sapling_tree(wx, y, wz);
                            }
                        }
                    }
                    blocks::REED | blocks::CACTUS => {
                        if y < 127 && self.get_block(wx, y + 1, wz) == blocks::AIR {
                            let mut height = 1;
                            while y - height >= 0 && self.get_block(wx, y - height, wz) == block_id
                            {
                                height += 1;
                            }
                            if height < 3 {
                                let meta = self.get_data(wx, y, wz);
                                if meta >= 15 {
                                    self.java_set_type_id(wx, y + 1, wz, block_id);
                                    self.java_set_data(wx, y, wz, 0);
                                } else {
                                    self.java_set_data(wx, y, wz, meta + 1);
                                }
                            }
                        }
                    }
                    blocks::FARMLAND => {
                        // Hydrate if water within 4x1x4, else dry out and revert if no crops above
                        let mut has_water = false;
                        'water: for dx in -4..=4 {
                            for dy in 0..=1 {
                                for dz in -4..=4 {
                                    if blocks::is_water(self.get_block(wx + dx, y + dy, wz + dz)) {
                                        has_water = true;
                                        break 'water;
                                    }
                                }
                            }
                        }
                        let meta = self.get_data(wx, y, wz);
                        if has_water {
                            if meta < 7 {
                                self.java_set_data(wx, y, wz, 7);
                            }
                        } else if meta > 0 {
                            self.java_set_data(wx, y, wz, meta - 1);
                        } else if self.get_block(wx, y + 1, wz) != blocks::CROPS {
                            self.java_set_type_id(wx, y, wz, blocks::DIRT);
                        }
                    }
                    blocks::SNOW => {
                        let block_light = self.java_get_saved_light(
                            crate::world::lighting::LightType::Block,
                            wx,
                            y,
                            wz,
                        );
                        if block_light > 11 {
                            self.java_set_type_id(wx, y, wz, blocks::AIR);
                        }
                    }
                    blocks::ICE => {
                        let block_light = self.java_get_saved_light(
                            crate::world::lighting::LightType::Block,
                            wx,
                            y,
                            wz,
                        );
                        if block_light > 11 - blocks::light_opacity(blocks::ICE) as i32 {
                            self.java_set_type_id(wx, y, wz, blocks::STATIONARY_WATER);
                        }
                    }
                    blocks::LIT_REDSTONE_ORE => {
                        self.java_set_type_id(wx, y, wz, blocks::REDSTONE_ORE);
                    }
                    blocks::FIRE => {
                        let below = self.get_block(wx, y - 1, wz);
                        if below != blocks::NETHERRACK && rng.next_i32_bound(4) == 0 {
                            self.java_set_type_id(wx, y, wz, blocks::AIR);
                        }
                    }
                    blocks::LEAVES => {
                        let meta = self.get_data(wx, y, wz);
                        if (meta & 8) != 0 {
                            // Check if any LOG is within 4 blocks
                            let mut near_log = false;
                            'log_scan: for dx in -4..=4 {
                                for dy in -4..=4 {
                                    for dz in -4..=4 {
                                        if self.get_block(wx + dx, y + dy, wz + dz) == blocks::LOG {
                                            near_log = true;
                                            break 'log_scan;
                                        }
                                    }
                                }
                            }
                            if near_log {
                                self.java_set_raw_data(wx, y, wz, meta & !8);
                            } else {
                                if rng.next_i32_bound(20) == 0 {
                                    self.world_events.push(WorldEvent::DropItem {
                                        x: wx as f64,
                                        y: y as f64,
                                        z: wz as f64,
                                        item: ItemStack::new(
                                            blocks::SAPLING as i16,
                                            1,
                                            (meta & 3) as i16,
                                        ),
                                    });
                                }
                                self.java_set_type_id(wx, y, wz, blocks::AIR);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        self.random_tick_lcg = lcg;
        self.tick_random = rng;
        let _ = CHUNK_HEIGHT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;

    fn make_flat_world() -> World {
        let mut world = World::new(WorldConfig {
            seed: 42,
            save_enabled: false,
            ..Default::default()
        });
        for cx in -1..=1 {
            for cz in -1..=1 {
                world.ensure_chunk_base(ChunkPos::new(cx, cz));
            }
        }
        for x in -8..=24 {
            for z in -8..=24 {
                for y in 60..70 {
                    world.set_block(x, y, z, if y == 63 { blocks::STONE } else { blocks::AIR });
                }
            }
        }
        world
    }

    #[test]
    fn redstone_wire_propagates_and_opens_iron_door() {
        let mut world = make_flat_world();
        // Place Lever at (0, 64, 0) on floor (meta = 5), Redstone Wire at (1, 64, 0) and (2, 64, 0),
        // and Iron Door at (3, 64, 0) + (3, 65, 0)
        world.java_set_type_id_and_data(0, 64, 0, blocks::LEVER, 5);
        world.java_set_type_id(1, 64, 0, blocks::REDSTONE_WIRE);
        world.java_set_type_id(2, 64, 0, blocks::REDSTONE_WIRE);
        world.java_set_type_id_and_data(3, 64, 0, blocks::IRON_DOOR, 0);
        world.java_set_type_id_and_data(3, 65, 0, blocks::IRON_DOOR, 8);

        assert_eq!(world.get_data(1, 64, 0), 0);
        assert_eq!(world.get_data(3, 64, 0) & 4, 0);

        // Flip Lever ON (meta = 5 | 8 = 13)
        world.java_set_data(0, 64, 0, 13);
        world.notify_attachment_neighbors(0, 64, 0, blocks::LEVER, 5);

        assert_eq!(world.get_data(1, 64, 0), 15);
        assert_eq!(world.get_data(2, 64, 0), 14);
        assert_ne!(
            world.get_data(3, 64, 0) & 4,
            0,
            "Iron door should open when powered"
        );

        // Flip Lever OFF (meta = 5)
        world.java_set_data(0, 64, 0, 5);
        world.notify_attachment_neighbors(0, 64, 0, blocks::LEVER, 5);

        assert_eq!(world.get_data(1, 64, 0), 0);
        assert_eq!(world.get_data(2, 64, 0), 0);
        assert_eq!(
            world.get_data(3, 64, 0) & 4,
            0,
            "Iron door should close when unpowered"
        );
    }

    #[test]
    fn redstone_torch_inverter_and_sticky_piston_push_pull() {
        let mut world = make_flat_world();
        // Sticky Piston at (5, 64, 5) facing +X (dir = 5), Cobblestone at (6, 64, 5)
        world.java_set_type_id_and_data(5, 64, 5, blocks::STICKY_PISTON, 5);
        world.java_set_type_id(6, 64, 5, blocks::COBBLESTONE);

        // Place Redstone Torch ON at (5, 64, 4) (south of piston) -> extends piston!
        world.java_set_type_id_and_data(5, 64, 4, blocks::REDSTONE_TORCH_ON, 5);

        assert_eq!(
            world.get_block(6, 64, 5),
            blocks::PISTON_HEAD,
            "Piston head should be at (6, 64, 5)"
        );
        assert_eq!(
            world.get_block(7, 64, 5),
            blocks::COBBLESTONE,
            "Cobblestone should be pushed to (7, 64, 5)"
        );

        // Remove Redstone Torch -> Sticky Piston retracts and pulls Cobblestone back to (6, 64, 5)!
        world.java_set_type_id(5, 64, 4, blocks::AIR);

        assert_eq!(
            world.get_block(6, 64, 5),
            blocks::COBBLESTONE,
            "Sticky piston should pull cobblestone back to (6, 64, 5)"
        );
        assert_eq!(world.get_block(7, 64, 5), blocks::AIR);
    }
}
