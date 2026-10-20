//! 1:1 port of Minecraft Beta 1.7.3 Redstone, Pistons, Dispensers, Rails, and Random Block Ticks
//! from `.references/Project-Poseidon`.

use std::collections::{BTreeSet, HashSet};

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
    LightningStrike {
        x: i32,
        y: i32,
        z: i32,
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
        if (id == blocks::FURNACE || id == blocks::LIT_FURNACE)
            && self.fixes.block_pistons_pushing_furnaces
        {
            return false;
        }
        if id == blocks::OBSIDIAN
            || id == blocks::BEDROCK
            || id == blocks::PORTAL
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

    /// Java `BlockPiston.c(World, i, j, k, EntityHuman)` (`BlockPiston.java:225-241`).
    #[allow(clippy::too_many_arguments)]
    pub fn compute_piston_orientation(
        x: i32,
        y: i32,
        z: i32,
        player_x: f64,
        player_feet_y: f64,
        player_z: f64,
        yaw: f32,
    ) -> u8 {
        if (player_x as f32 - x as f32).abs() < 2.0 && (player_z as f32 - z as f32).abs() < 2.0 {
            let d0 = player_feet_y + 1.82;
            if d0 - (y as f64) > 2.0 {
                return 1;
            }
            if (y as f64) - d0 > 0.0 {
                return 0;
            }
        }
        let l = (((yaw * 4.0 / 360.0) as f64 + 0.5).floor() as i32) & 3;
        match l {
            0 => 2,
            1 => 5,
            2 => 3,
            _ => 4,
        }
    }

    pub fn flush_pending_piston_syncs(&mut self) {
        if self.pending_piston_syncs.is_empty() {
            return;
        }
        let now = self.live_time;
        let mut ready = Vec::new();
        self.pending_piston_syncs.retain(|&(t, x, y, z)| {
            if now >= t {
                ready.push((x, y, z));
                false
            } else {
                true
            }
        });
        for (x, y, z) in ready {
            self.flag_dirty(x, y, z);
        }
    }

    pub fn update_piston_state(&mut self, x: i32, y: i32, z: i32, piston_id: u8) {
        if self.piston_updating {
            return;
        }
        let meta = self.get_data(x, y, z);
        let dir = (meta & 7) as usize;
        if dir > 5 {
            return;
        }
        let is_extended = (meta & 8) != 0;
        let powered = self.is_piston_powered(x, y, z, dir);
        let is_sticky = piston_id == blocks::STICKY_PISTON;

        if powered && !is_extended {
            self.piston_updating = true;
            let extended = self.try_extend_piston(x, y, z, dir, is_sticky);
            self.piston_updating = false;
            if extended {
                self.world_events.push(WorldEvent::PlayNote {
                    x,
                    y: y as i16,
                    z,
                    instrument: 0,
                    pitch: dir as u8,
                });
            }
        } else if !powered && is_extended {
            self.piston_updating = true;
            self.java_set_raw_data(x, y, z, dir as u8);
            self.retract_piston(x, y, z, dir, is_sticky);
            self.piston_updating = false;
            let sync_time = self.live_time + 2;
            self.pending_piston_syncs.push((sync_time, x, y, z));
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
                // Poseidon `world.settings.pistons.other-fixes.enabled`:
                // break fragile blocks with `setRawTypeId` so their removal
                // cannot trigger neighbour physics that re-enters the piston.
                if self.fixes.piston_other_fixes {
                    self.java_set_raw_type_id(cx, cy, cz, blocks::AIR);
                } else {
                    self.java_set_type_id(cx, cy, cz, blocks::AIR);
                }
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

        // Set base metadata bit 8 BEFORE moving blocks (`BlockPiston.java:85`)
        self.java_set_raw_data(x, y, z, (dir as u8) | 8);
        let sync_time = self.live_time + 2;
        self.pending_piston_syncs.push((sync_time, x, y, z));

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
            self.java_set_raw_type_id_and_data(tx, ty, tz, bid, bmeta);
            self.java_apply_physics(tx, ty, tz, bid as i32);
            self.pending_piston_syncs.push((sync_time, tx, ty, tz));
        }

        // Place Piston Extension (34) at (x + dx, y + dy, z + dz)
        let ext_meta = (dir as u8) | if is_sticky { 8 } else { 0 };
        let hx = x + dx;
        let hy = y + dy;
        let hz = z + dz;
        self.java_set_raw_type_id_and_data(hx, hy, hz, blocks::PISTON_HEAD, ext_meta);
        self.java_apply_physics(hx, hy, hz, blocks::PISTON_HEAD as i32);
        self.pending_piston_syncs.push((sync_time, hx, hy, hz));
        true
    }

    fn retract_piston(&mut self, x: i32, y: i32, z: i32, dir: usize, is_sticky: bool) {
        let dx = PISTON_DX[dir];
        let dy = PISTON_DY[dir];
        let dz = PISTON_DZ[dir];
        let hx = x + dx;
        let hy = y + dy;
        let hz = z + dz;
        let sync_time = self.live_time + 2;

        if self.get_block(hx, hy, hz) == blocks::PISTON_HEAD {
            self.java_set_raw_type_id_and_data(hx, hy, hz, blocks::AIR, 0);
            self.java_apply_physics(hx, hy, hz, blocks::PISTON_HEAD as i32);
            self.pending_piston_syncs.push((sync_time, hx, hy, hz));
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
                    self.java_set_raw_type_id_and_data(hx, hy, hz, pull_id, pull_meta);
                    self.java_apply_physics(hx, hy, hz, pull_id as i32);
                    self.pending_piston_syncs.push((sync_time, px, py, pz));
                    self.pending_piston_syncs.push((sync_time, hx, hy, hz));
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // Rails (`BlockMinecartTrack.java` & `MinecartTrackLogic.java`: 66 / 27 / 28)
    // ---------------------------------------------------------------------

    pub fn is_rail_block(id: u8) -> bool {
        matches!(
            id,
            blocks::RAIL | blocks::POWERED_RAIL | blocks::DETECTOR_RAIL
        )
    }

    pub fn is_rail_at(&self, x: i32, y: i32, z: i32) -> bool {
        Self::is_rail_block(self.get_block(x, y, z))
    }

    fn has_rail_in_column(&self, x: i32, y: i32, z: i32) -> bool {
        self.is_rail_at(x, y, z) || self.is_rail_at(x, y + 1, z) || self.is_rail_at(x, y - 1, z)
    }

    fn find_rail_in_column(&self, x: i32, y: i32, z: i32) -> Option<(i32, i32, i32)> {
        if self.is_rail_at(x, y, z) {
            Some((x, y, z))
        } else if self.is_rail_at(x, y + 1, z) {
            Some((x, y + 1, z))
        } else if self.is_rail_at(x, y - 1, z) {
            Some((x, y - 1, z))
        } else {
            None
        }
    }

    /// Java `MinecartTrackLogic.a(int meta)` — return the 2 connected `(x, y, z)` endpoints for a rail.
    pub fn rail_connected_endpoints(x: i32, y: i32, z: i32, shape: u8) -> [(i32, i32, i32); 2] {
        match shape {
            0 => [(x, y, z - 1), (x, y, z + 1)],
            1 => [(x - 1, y, z), (x + 1, y, z)],
            2 => [(x - 1, y, z), (x + 1, y + 1, z)],
            3 => [(x - 1, y + 1, z), (x + 1, y, z)],
            4 => [(x, y + 1, z - 1), (x, y, z + 1)],
            5 => [(x, y, z - 1), (x, y + 1, z + 1)],
            6 => [(x + 1, y, z), (x, y, z + 1)],
            7 => [(x - 1, y, z), (x, y, z + 1)],
            8 => [(x - 1, y, z), (x, y, z - 1)],
            9 => [(x + 1, y, z), (x, y, z - 1)],
            _ => [(x, y, z - 1), (x, y, z + 1)],
        }
    }

    /// Java `MinecartTrackLogic.a(boolean powered, boolean initialPlace)` (`MinecartTrackLogic.java:230-353`).
    pub fn update_rail_track(&mut self, x: i32, y: i32, z: i32, initial_place: bool) {
        let id = self.get_block(x, y, z);
        if !Self::is_rail_block(id) {
            return;
        }
        let is_special = id == blocks::POWERED_RAIL || id == blocks::DETECTOR_RAIL;
        let powered = self.is_block_indirectly_powered(x, y, z);

        let f_north = self.can_rail_connect_to(x, y, z - 1, x, z);
        let f_south = self.can_rail_connect_to(x, y, z + 1, x, z);
        let f_west = self.can_rail_connect_to(x - 1, y, z, x, z);
        let f_east = self.can_rail_connect_to(x + 1, y, z, x, z);

        let mut shape: i32 = -1;
        if (f_north || f_south) && !f_west && !f_east {
            shape = 0;
        }
        if (f_west || f_east) && !f_north && !f_south {
            shape = 1;
        }
        if !is_special {
            if f_south && f_east && !f_north && !f_west {
                shape = 6;
            }
            if f_south && f_west && !f_north && !f_east {
                shape = 7;
            }
            if f_north && f_west && !f_south && !f_east {
                shape = 8;
            }
            if f_north && f_east && !f_south && !f_west {
                shape = 9;
            }
        }
        if shape == -1 {
            if f_north || f_south {
                shape = 0;
            }
            if f_west || f_east {
                shape = 1;
            }
            if !is_special {
                if powered {
                    if f_south && f_east {
                        shape = 6;
                    }
                    if f_west && f_south {
                        shape = 7;
                    }
                    if f_east && f_north {
                        shape = 9;
                    }
                    if f_north && f_west {
                        shape = 8;
                    }
                } else {
                    if f_north && f_west {
                        shape = 8;
                    }
                    if f_east && f_north {
                        shape = 9;
                    }
                    if f_west && f_south {
                        shape = 7;
                    }
                    if f_south && f_east {
                        shape = 6;
                    }
                }
            }
        }

        if shape == 0 {
            if self.is_rail_at(x, y + 1, z - 1) {
                shape = 4;
            }
            if self.is_rail_at(x, y + 1, z + 1) {
                shape = 5;
            }
        }
        if shape == 1 {
            if self.is_rail_at(x + 1, y + 1, z) {
                shape = 2;
            }
            if self.is_rail_at(x - 1, y + 1, z) {
                shape = 3;
            }
        }
        if shape < 0 {
            shape = 0;
        }

        let new_meta = if is_special {
            (self.get_data(x, y, z) & 8) | (shape as u8)
        } else {
            shape as u8
        };

        if initial_place || self.get_data(x, y, z) != new_meta {
            self.java_set_data(x, y, z, new_meta);
            let endpoints = Self::rail_connected_endpoints(x, y, z, shape as u8);
            for (ex, ey, ez) in endpoints {
                if let Some((nx, ny, nz)) = self.find_rail_in_column(ex, ey, ez) {
                    self.connect_neighbor_rail_toward(nx, ny, nz, x, y, z);
                }
            }
        }
    }

    fn active_rail_connections(&self, rx: i32, ry: i32, rz: i32) -> Vec<(i32, i32, i32)> {
        let id = self.get_block(rx, ry, rz);
        let raw_meta = self.get_data(rx, ry, rz);
        let shape = if id == blocks::POWERED_RAIL || id == blocks::DETECTOR_RAIL {
            raw_meta & 7
        } else {
            raw_meta
        };
        let endpoints = Self::rail_connected_endpoints(rx, ry, rz, shape);
        let mut active = Vec::with_capacity(2);
        for (ex, ey, ez) in endpoints {
            if let Some((nx, ny, nz)) = self.find_rail_in_column(ex, ey, ez) {
                let nid = self.get_block(nx, ny, nz);
                let nmeta = self.get_data(nx, ny, nz);
                let nshape = if nid == blocks::POWERED_RAIL || nid == blocks::DETECTOR_RAIL {
                    nmeta & 7
                } else {
                    nmeta
                };
                let n_ends = Self::rail_connected_endpoints(nx, ny, nz, nshape);
                if n_ends.iter().any(|&(tx, _, tz)| tx == rx && tz == rz) {
                    active.push((nx, ny, nz));
                }
            }
        }
        active
    }

    fn can_rail_connect_to(&self, cx: i32, cy: i32, cz: i32, from_x: i32, from_z: i32) -> bool {
        let Some((rx, ry, rz)) = self.find_rail_in_column(cx, cy, cz) else {
            return false;
        };
        let active = self.active_rail_connections(rx, ry, rz);
        if active
            .iter()
            .any(|&(ax, _, az)| ax == from_x && az == from_z)
        {
            return true;
        }
        active.len() < 2
    }

    /// Java `MinecartTrackLogic.d(MinecartTrackLogic)` (`MinecartTrackLogic.java:152-217`).
    fn connect_neighbor_rail_toward(
        &mut self,
        rx: i32,
        ry: i32,
        rz: i32,
        target_x: i32,
        target_y: i32,
        target_z: i32,
    ) {
        let mut active = self.active_rail_connections(rx, ry, rz);
        if !active
            .iter()
            .any(|&(ax, _, az)| ax == target_x && az == target_z)
        {
            if active.len() >= 2 {
                return;
            }
            active.push((target_x, target_y, target_z));
        }

        let id = self.get_block(rx, ry, rz);
        let is_special = id == blocks::POWERED_RAIL || id == blocks::DETECTOR_RAIL;

        let f_north = active.iter().any(|&(ax, _, az)| ax == rx && az == rz - 1);
        let f_south = active.iter().any(|&(ax, _, az)| ax == rx && az == rz + 1);
        let f_west = active.iter().any(|&(ax, _, az)| ax == rx - 1 && az == rz);
        let f_east = active.iter().any(|&(ax, _, az)| ax == rx + 1 && az == rz);

        let mut shape: i32 = -1;
        if f_north || f_south {
            shape = 0;
        }
        if f_west || f_east {
            shape = 1;
        }
        if !is_special {
            if f_south && f_east && !f_north && !f_west {
                shape = 6;
            }
            if f_south && f_west && !f_north && !f_east {
                shape = 7;
            }
            if f_north && f_west && !f_south && !f_east {
                shape = 8;
            }
            if f_north && f_east && !f_south && !f_west {
                shape = 9;
            }
        }
        if shape == 0 {
            if self.is_rail_at(rx, ry + 1, rz - 1) {
                shape = 4;
            }
            if self.is_rail_at(rx, ry + 1, rz + 1) {
                shape = 5;
            }
        }
        if shape == 1 {
            if self.is_rail_at(rx + 1, ry + 1, rz) {
                shape = 2;
            }
            if self.is_rail_at(rx - 1, ry + 1, rz) {
                shape = 3;
            }
        }
        if shape < 0 {
            shape = 0;
        }
        let new_meta = if is_special {
            (self.get_data(rx, ry, rz) & 8) | (shape as u8)
        } else {
            shape as u8
        };
        self.java_set_data(rx, ry, rz, new_meta);
    }

    /// Java `BlockMinecartTrack.doPhysics` (`BlockMinecartTrack.java:82-140`).
    pub fn do_physics_rail(&mut self, x: i32, y: i32, z: i32, id: u8, neighbor_id: i32) {
        let meta = self.get_data(x, y, z);
        let is_special = id == blocks::POWERED_RAIL || id == blocks::DETECTOR_RAIL;
        let shape = if is_special { meta & 7 } else { meta };

        let mut invalid = !self.is_solid_normal_at(x, y - 1, z);
        if shape == 2 && !self.is_solid_normal_at(x + 1, y, z) {
            invalid = true;
        }
        if shape == 3 && !self.is_solid_normal_at(x - 1, y, z) {
            invalid = true;
        }
        if shape == 4 && !self.is_solid_normal_at(x, y, z - 1) {
            invalid = true;
        }
        if shape == 5 && !self.is_solid_normal_at(x, y, z + 1) {
            invalid = true;
        }

        if invalid {
            self.world_events.push(WorldEvent::DropItem {
                x: x as f64,
                y: y as f64,
                z: z as f64,
                item: ItemStack::new(id as i16, 1, 0),
            });
            self.java_set_type_id(x, y, z, blocks::AIR);
        } else if id == blocks::POWERED_RAIL {
            let powered = self.is_block_indirectly_powered(x, y, z)
                || self.is_block_indirectly_powered(x, y + 1, z)
                || self.powered_rail_chain(x, y, z, meta, true, 0)
                || self.powered_rail_chain(x, y, z, meta, false, 0);
            let mut changed = false;
            if powered && (meta & 8) == 0 {
                self.java_set_data(x, y, z, shape | 8);
                changed = true;
            } else if !powered && (meta & 8) != 0 {
                self.java_set_data(x, y, z, shape);
                changed = true;
            }
            if changed {
                self.java_apply_physics(x, y - 1, z, id as i32);
                if (2..=5).contains(&shape) {
                    self.java_apply_physics(x, y + 1, z, id as i32);
                }
            }
        } else if neighbor_id > 0 && self.is_power_source_block(neighbor_id as u8) && !is_special {
            let count = usize::from(self.has_rail_in_column(x, y, z - 1))
                + usize::from(self.has_rail_in_column(x, y, z + 1))
                + usize::from(self.has_rail_in_column(x - 1, y, z))
                + usize::from(self.has_rail_in_column(x + 1, y, z));
            if count == 3 {
                self.update_rail_track(x, y, z, false);
            }
        }
    }

    fn powered_rail_chain(
        &self,
        mut x: i32,
        mut y: i32,
        mut z: i32,
        meta: u8,
        forward: bool,
        depth: usize,
    ) -> bool {
        if depth >= 8 {
            return false;
        }
        let mut shape = meta & 7;
        let mut check_down = true;
        match shape {
            0 => {
                if forward {
                    z += 1;
                } else {
                    z -= 1;
                }
            }
            1 => {
                if forward {
                    x -= 1;
                } else {
                    x += 1;
                }
            }
            2 => {
                if forward {
                    x -= 1;
                } else {
                    x += 1;
                    y += 1;
                    check_down = false;
                }
                shape = 1;
            }
            3 => {
                if forward {
                    x -= 1;
                    y += 1;
                    check_down = false;
                } else {
                    x += 1;
                }
                shape = 1;
            }
            4 => {
                if forward {
                    z += 1;
                } else {
                    z -= 1;
                    y += 1;
                    check_down = false;
                }
                shape = 0;
            }
            5 => {
                if forward {
                    z += 1;
                    y += 1;
                    check_down = false;
                } else {
                    z -= 1;
                }
                shape = 0;
            }
            _ => {}
        }
        if self.powered_rail_step(x, y, z, forward, depth, shape) {
            return true;
        }
        check_down && self.powered_rail_step(x, y - 1, z, forward, depth, shape)
    }

    fn powered_rail_step(
        &self,
        x: i32,
        y: i32,
        z: i32,
        forward: bool,
        depth: usize,
        prev_shape: u8,
    ) -> bool {
        if self.get_block(x, y, z) != blocks::POWERED_RAIL {
            return false;
        }
        let meta = self.get_data(x, y, z);
        let shape = meta & 7;
        if prev_shape == 1 && (shape == 0 || shape == 4 || shape == 5) {
            return false;
        }
        if prev_shape == 0 && (shape == 1 || shape == 2 || shape == 3) {
            return false;
        }
        if (meta & 8) != 0 {
            if self.is_block_indirectly_powered(x, y, z)
                || self.is_block_indirectly_powered(x, y + 1, z)
            {
                return true;
            }
            return self.powered_rail_chain(x, y, z, meta, forward, depth + 1);
        }
        false
    }

    // ---------------------------------------------------------------------
    // Minecart Physics (`EntityMinecart.java`: `h`, `move`, `m_`)
    // ---------------------------------------------------------------------

    /// Java `EntityMinecart.h(double d0, double d1, double d2)` (`EntityMinecart.java:592-656`).
    pub fn snap_to_rail_h(&self, mut d0: f64, mut d1: f64, mut d2: f64) -> Option<(f64, f64, f64)> {
        let i = d0.floor() as i32;
        let mut j = d1.floor() as i32;
        let k = d2.floor() as i32;

        if self.is_rail_at(i, j - 1, k) {
            j -= 1;
        }

        let l = self.get_block(i, j, k);
        if Self::is_rail_block(l) {
            let mut i1 = self.get_data(i, j, k);
            if l == blocks::POWERED_RAIL || l == blocks::DETECTOR_RAIL {
                i1 &= 7;
            }

            let aint = crate::entity::projectile::RAIL_MATRIX[(i1 as usize).min(9)];
            let d4 = i as f64 + 0.5 + (aint[0][0] as f64) * 0.5;
            let d5 = j as f64 + 0.5 + (aint[0][1] as f64) * 0.5;
            let d6 = k as f64 + 0.5 + (aint[0][2] as f64) * 0.5;
            let d7 = i as f64 + 0.5 + (aint[1][0] as f64) * 0.5;
            let d8 = j as f64 + 0.5 + (aint[1][1] as f64) * 0.5;
            let d9 = k as f64 + 0.5 + (aint[1][2] as f64) * 0.5;
            let d10 = d7 - d4;
            let d11 = (d8 - d5) * 2.0;
            let d12 = d9 - d6;

            let d3 = if d10 == 0.0 {
                d2 - k as f64
            } else if d12 == 0.0 {
                d0 - i as f64
            } else {
                let d13 = d0 - d4;
                let d14 = d2 - d6;
                (d13 * d10 + d14 * d12) * 2.0
            };

            d0 = d4 + d10 * d3;
            d1 = d5 + d11 * d3;
            d2 = d6 + d12 * d3;
            if d11 < 0.0 {
                d1 += 1.0;
            }
            if d11 > 0.0 {
                d1 += 0.5;
            }
            Some((d0, d1, d2))
        } else {
            None
        }
    }

    /// Java `Entity.move(double d0, double d1, double d2)` (`Entity.java:353-641`) for `EntityMinecart`.
    /// Returns `Some(fall_damage)` if the minecart landed from a fall and should transfer damage to its passenger.
    pub fn move_minecart_aabb(
        &mut self,
        cart: &mut crate::entity::projectile::Minecart,
        mut d0: f64,
        mut d1: f64,
        mut d2: f64,
    ) -> Option<i16> {
        let d5 = d0;
        let d6 = d1;
        let d7 = d2;
        let (mut bb_a, mut bb_b, mut bb_c, mut bb_d, mut bb_e, mut bb_f) = cart.bounding_box();

        let qa = bb_a + d0.min(0.0);
        let qb = bb_b + d1.min(0.0);
        let qc = bb_c + d2.min(0.0);
        let qd = bb_d + d0.max(0.0);
        let qe = bb_e + d1.max(0.0);
        let qf = bb_f + d2.max(0.0);

        let min_bx = qa.floor() as i32;
        let max_bx = (qd + 1.0).floor() as i32;
        let min_by = qb.floor() as i32 - 1;
        let max_by = (qe + 1.0).floor() as i32;
        let min_bz = qc.floor() as i32;
        let max_bz = (qf + 1.0).floor() as i32;

        let mut block_bbs = Vec::new();
        for bx in min_bx..max_bx {
            for bz in min_bz..max_bz {
                if !self.is_chunk_loaded(ChunkPos::new(bx >> 4, bz >> 4)) {
                    continue;
                }
                for by in min_by..max_by {
                    if !(0..128).contains(&by) {
                        continue;
                    }
                    let bid = self.get_block(bx, by, bz);
                    if !blocks::is_solid(bid) {
                        continue;
                    }
                    let ba = bx as f64;
                    let bb = by as f64;
                    let bc = bz as f64;
                    let bd = (bx + 1) as f64;
                    let bf = (bz + 1) as f64;
                    let be = match bid {
                        blocks::STEP => by as f64 + 0.5,
                        blocks::SOUL_SAND => by as f64 + 0.875,
                        blocks::FENCE => {
                            by as f64
                                + if self.mechanics.use_modern_fence_bounding_boxes {
                                    1.0
                                } else {
                                    1.5
                                }
                        }
                        blocks::BED => by as f64 + 0.5625,
                        _ => (by + 1) as f64,
                    };
                    if qd > ba && qa < bd && qe > bb && qb < be && qf > bc && qc < bf {
                        block_bbs.push((ba, bb, bc, bd, be, bf));
                    }
                }
            }
        }

        // 1. Sweep Y (`AxisAlignedBB.b`)
        for &(ba, bb, bc, bd, be, bf) in &block_bbs {
            if bb_d > ba && bb_a < bd && bb_f > bc && bb_c < bf {
                if d1 > 0.0 && bb_e <= bb {
                    let diff = bb - bb_e;
                    if diff < d1 {
                        d1 = diff;
                    }
                }
                if d1 < 0.0 && bb_b >= be {
                    let diff = be - bb_b;
                    if diff > d1 {
                        d1 = diff;
                    }
                }
            }
        }
        bb_b += d1;
        bb_e += d1;

        // 2. Sweep X (`AxisAlignedBB.a`)
        for &(ba, bb, bc, bd, be, bf) in &block_bbs {
            if bb_e > bb && bb_b < be && bb_f > bc && bb_c < bf {
                if d0 > 0.0 && bb_d <= ba {
                    let diff = ba - bb_d;
                    if diff < d0 {
                        d0 = diff;
                    }
                }
                if d0 < 0.0 && bb_a >= bd {
                    let diff = bd - bb_a;
                    if diff > d0 {
                        d0 = diff;
                    }
                }
            }
        }
        bb_a += d0;
        bb_d += d0;

        // 3. Sweep Z (`AxisAlignedBB.c`)
        for &(ba, bb, bc, bd, be, bf) in &block_bbs {
            if bb_d > ba && bb_a < bd && bb_e > bb && bb_b < be {
                if d2 > 0.0 && bb_f <= bc {
                    let diff = bc - bb_f;
                    if diff < d2 {
                        d2 = diff;
                    }
                }
                if d2 < 0.0 && bb_c >= bf {
                    let diff = bf - bb_c;
                    if diff > d2 {
                        d2 = diff;
                    }
                }
            }
        }
        bb_c += d2;
        bb_f += d2;

        cart.x = (bb_a + bb_d) / 2.0;
        cart.y = bb_b + crate::entity::projectile::MINECART_HEIGHT_OFFSET;
        cart.z = (bb_c + bb_f) / 2.0;
        cart.on_ground = d6 != d1 && d6 < 0.0;

        let mut passenger_fall_dmg = None;
        if cart.on_ground {
            if cart.fall_distance > 0.0 {
                let dmg = ((cart.fall_distance - 3.0).ceil() as i16).max(0);
                if dmg > 0 {
                    passenger_fall_dmg = Some(dmg);
                }
                cart.fall_distance = 0.0;
            }
        } else if d1 < 0.0 {
            cart.fall_distance -= d1 as f32;
        }

        if d5 != d0 {
            cart.mot_x = 0.0;
        }
        if d6 != d1 {
            cart.mot_y = 0.0;
        }
        if d7 != d2 {
            cart.mot_z = 0.0;
        }

        // Entity.java:592-611 — invoke `Block.a` on blocks overlapping `boundingBox`
        let l = (bb_a + 0.001).floor() as i32;
        let i1 = (bb_b + 0.001).floor() as i32;
        let j1 = (bb_c + 0.001).floor() as i32;
        let k = (bb_d - 0.001).floor() as i32;
        let k1 = (bb_e - 0.001).floor() as i32;
        let l1 = (bb_f - 0.001).floor() as i32;

        if self.chunks_area_loaded(l, i1.max(0), j1, k, k1.min(127), l1) {
            for ix in l..=k {
                for iy in i1.max(0)..=k1.min(127) {
                    for iz in j1..=l1 {
                        if self.get_block(ix, iy, iz) == blocks::DETECTOR_RAIL {
                            // BlockMinecartDetector.f: AABB `[ix + 0.125 .. ix + 0.875, iy .. iy + 0.25, iz + 0.125 .. iz + 0.875]`
                            if bb_d > ix as f64 + 0.125
                                && bb_a < ix as f64 + 0.875
                                && bb_e > iy as f64
                                && bb_b < iy as f64 + 0.25
                                && bb_f > iz as f64 + 0.125
                                && bb_c < iz as f64 + 0.875
                            {
                                self.active_detector_rails.insert((ix, iy, iz));
                                let meta = self.get_data(ix, iy, iz);
                                if (meta & 8) == 0 {
                                    self.java_set_data(ix, iy, iz, meta | 8);
                                    self.java_apply_physics(
                                        ix,
                                        iy,
                                        iz,
                                        blocks::DETECTOR_RAIL as i32,
                                    );
                                    self.java_apply_physics(
                                        ix,
                                        iy - 1,
                                        iz,
                                        blocks::DETECTOR_RAIL as i32,
                                    );
                                    self.java_schedule_tick(ix, iy, iz, blocks::DETECTOR_RAIL, 20);
                                }
                            }
                        }
                    }
                }
            }
        }

        passenger_fall_dmg
    }

    /// Java `EntityMinecart.m_()` (`EntityMinecart.java:209-590`).
    /// Returns `Some(fall_damage)` if the minecart landed and transferred fall damage to its passenger.
    pub fn tick_single_minecart(
        &mut self,
        cart: &mut crate::entity::projectile::Minecart,
    ) -> Option<i16> {
        if cart.time_since_hit > 0 {
            cart.time_since_hit -= 1;
        }
        if cart.damage > 0 {
            cart.damage -= 1;
        }
        if cart.y < -64.0 {
            cart.dead = true;
            return None;
        }

        cart.last_x = cart.x;
        cart.last_y = cart.y;
        cart.last_z = cart.z;
        cart.mot_y -= 0.03999999910593033;

        let i = cart.x.floor() as i32;
        let mut j = cart.y.floor() as i32;
        let k = cart.z.floor() as i32;

        if self.is_rail_at(i, j - 1, k) {
            j -= 1;
        }

        let d4 = cart.max_speed;
        let mut flag = false;
        let d0 = 0.0078125_f64;
        let l = self.get_block(i, j, k);
        let passenger_fall_dmg;

        if Self::is_rail_block(l) {
            let vec3d = self.snap_to_rail_h(cart.x, cart.y, cart.z);
            let mut i1 = self.get_data(i, j, k);
            cart.y = j as f64;
            let mut flag1 = false;
            let mut flag2 = false;

            if l == blocks::POWERED_RAIL {
                flag1 = (i1 & 8) != 0;
                flag2 = !flag1;
            }
            if l == blocks::POWERED_RAIL || l == blocks::DETECTOR_RAIL {
                i1 &= 7;
            }
            if (2..=5).contains(&i1) {
                cart.y = (j + 1) as f64;
            }

            if i1 == 2 {
                cart.mot_x -= d0;
            }
            if i1 == 3 {
                cart.mot_x += d0;
            }
            if i1 == 4 {
                cart.mot_z += d0;
            }
            if i1 == 5 {
                cart.mot_z -= d0;
            }

            let aint = crate::entity::projectile::RAIL_MATRIX[(i1 as usize).min(9)];
            let mut d5 = (aint[1][0] - aint[0][0]) as f64;
            let mut d6 = (aint[1][2] - aint[0][2]) as f64;
            let d7 = (d5 * d5 + d6 * d6).sqrt();
            let d8 = cart.mot_x * d5 + cart.mot_z * d6;
            if d8 < 0.0 {
                d5 = -d5;
                d6 = -d6;
            }

            let mut d9 = (cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z).sqrt();
            cart.mot_x = d9 * d5 / d7;
            cart.mot_z = d9 * d6 / d7;

            if flag2 {
                let d10_spd = (cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z).sqrt();
                if d10_spd < 0.03 {
                    cart.mot_x *= 0.0;
                    cart.mot_y *= 0.0;
                    cart.mot_z *= 0.0;
                } else {
                    cart.mot_x *= 0.5;
                    cart.mot_y *= 0.0;
                    cart.mot_z *= 0.5;
                }
            }

            let d11 = i as f64 + 0.5 + (aint[0][0] as f64) * 0.5;
            let d12 = k as f64 + 0.5 + (aint[0][2] as f64) * 0.5;
            let d13 = i as f64 + 0.5 + (aint[1][0] as f64) * 0.5;
            let d14 = k as f64 + 0.5 + (aint[1][2] as f64) * 0.5;
            d5 = d13 - d11;
            d6 = d14 - d12;

            let d10 = if d5 == 0.0 {
                cart.x = i as f64 + 0.5;
                cart.z - k as f64
            } else if d6 == 0.0 {
                cart.z = k as f64 + 0.5;
                cart.x - i as f64
            } else {
                let d16 = cart.x - d11;
                let d15 = cart.z - d12;
                (d16 * d5 + d15 * d6) * 2.0
            };

            cart.x = d11 + d5 * d10;
            cart.z = d12 + d6 * d10;
            cart.set_position(
                cart.x,
                cart.y + crate::entity::projectile::MINECART_HEIGHT_OFFSET,
                cart.z,
            );

            let mut d16 = cart.mot_x;
            let mut d15 = cart.mot_z;
            if cart.passenger_id.is_some() {
                d16 *= 0.75;
                d15 *= 0.75;
            }
            d16 = d16.clamp(-d4, d4);
            d15 = d15.clamp(-d4, d4);

            passenger_fall_dmg = self.move_minecart_aabb(cart, d16, 0.0, d15);

            if aint[0][1] != 0
                && (cart.x.floor() as i32) - i == aint[0][0]
                && (cart.z.floor() as i32) - k == aint[0][2]
            {
                cart.set_position(cart.x, cart.y + aint[0][1] as f64, cart.z);
            } else if aint[1][1] != 0
                && (cart.x.floor() as i32) - i == aint[1][0]
                && (cart.z.floor() as i32) - k == aint[1][2]
            {
                cart.set_position(cart.x, cart.y + aint[1][1] as f64, cart.z);
            }

            if cart.passenger_id.is_some() || !cart.slow_when_empty {
                cart.mot_x *= 0.996999979019165;
                cart.mot_y *= 0.0;
                cart.mot_z *= 0.996999979019165;
            } else {
                if cart.cart_type == 2 {
                    let d17 = ((cart.push_x * cart.push_x + cart.push_z * cart.push_z).sqrt()
                        as f32) as f64;
                    if d17 > 0.01 {
                        flag = true;
                        cart.push_x /= d17;
                        cart.push_z /= d17;
                        let d18 = 0.04;
                        cart.mot_x *= 0.800000011920929;
                        cart.mot_y *= 0.0;
                        cart.mot_z *= 0.800000011920929;
                        cart.mot_x += cart.push_x * d18;
                        cart.mot_z += cart.push_z * d18;
                    } else {
                        cart.mot_x *= 0.8999999761581421;
                        cart.mot_y *= 0.0;
                        cart.mot_z *= 0.8999999761581421;
                    }
                }
                cart.mot_x *= 0.9599999785423279;
                cart.mot_y *= 0.0;
                cart.mot_z *= 0.9599999785423279;
            }

            let vec3d1 = self.snap_to_rail_h(cart.x, cart.y, cart.z);
            if let (Some(v1), Some(v0)) = (vec3d1, vec3d) {
                let d19 = (v0.1 - v1.1) * 0.05;
                d9 = (cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z).sqrt();
                if d9 > 0.0 {
                    cart.mot_x = cart.mot_x / d9 * (d9 + d19);
                    cart.mot_z = cart.mot_z / d9 * (d9 + d19);
                }
                cart.set_position(cart.x, v1.1, cart.z);
            }

            let j1 = cart.x.floor() as i32;
            let k1 = cart.z.floor() as i32;
            if j1 != i || k1 != k {
                d9 = (cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z).sqrt();
                cart.mot_x = d9 * ((j1 - i) as f64);
                cart.mot_z = d9 * ((k1 - k) as f64);
            }

            if cart.cart_type == 2 {
                let d20 =
                    ((cart.push_x * cart.push_x + cart.push_z * cart.push_z).sqrt() as f32) as f64;
                if d20 > 0.01 && cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z > 0.001 {
                    cart.push_x /= d20;
                    cart.push_z /= d20;
                    if cart.push_x * cart.mot_x + cart.push_z * cart.mot_z < 0.0 {
                        cart.push_x = 0.0;
                        cart.push_z = 0.0;
                    } else {
                        cart.push_x = cart.mot_x;
                        cart.push_z = cart.mot_z;
                    }
                }
            }

            if flag1 {
                let d20 = (cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z).sqrt();
                if d20 > 0.01 {
                    let d21 = 0.06;
                    cart.mot_x += cart.mot_x / d20 * d21;
                    cart.mot_z += cart.mot_z / d20 * d21;
                } else if i1 == 1 {
                    if self.is_solid_normal_at(i - 1, j, k) {
                        cart.mot_x = 0.02;
                    } else if self.is_solid_normal_at(i + 1, j, k) {
                        cart.mot_x = -0.02;
                    }
                } else if i1 == 0 {
                    if self.is_solid_normal_at(i, j, k - 1) {
                        cart.mot_z = 0.02;
                    } else if self.is_solid_normal_at(i, j, k + 1) {
                        cart.mot_z = -0.02;
                    }
                }
            }
        } else {
            cart.mot_x = cart.mot_x.clamp(-d4, d4);
            cart.mot_z = cart.mot_z.clamp(-d4, d4);
            if cart.on_ground {
                cart.mot_x *= cart.derailed_x;
                cart.mot_y *= cart.derailed_y;
                cart.mot_z *= cart.derailed_z;
            }
            passenger_fall_dmg = self.move_minecart_aabb(cart, cart.mot_x, cart.mot_y, cart.mot_z);
            if !cart.on_ground {
                cart.mot_x *= cart.flying_x;
                cart.mot_y *= cart.flying_y;
                cart.mot_z *= cart.flying_z;
            }
        }

        cart.pitch = 0.0;
        let d22 = cart.last_x - cart.x;
        let d23 = cart.last_z - cart.z;
        if d22 * d22 + d23 * d23 > 0.001 {
            cart.yaw = (d23.atan2(d22) * 180.0 / std::f64::consts::PI) as f32;
            if cart.yaw_flipped {
                cart.yaw += 180.0;
            }
        }

        let mut d24 = (cart.yaw - cart.last_yaw) as f64;
        while d24 >= 180.0 {
            d24 -= 360.0;
        }
        while d24 < -180.0 {
            d24 += 360.0;
        }
        if !(-170.0..170.0).contains(&d24) {
            cart.yaw += 180.0;
            cart.yaw_flipped = !cart.yaw_flipped;
        }
        cart.yaw %= 360.0;
        cart.pitch %= 360.0;
        cart.last_yaw = cart.yaw;

        if flag && cart.rng.next_i32_bound(4) == 0 {
            cart.fuel -= 1;
            if cart.fuel < 0 {
                cart.push_x = 0.0;
                cart.push_z = 0.0;
            }
        }

        passenger_fall_dmg
    }

    // ---------------------------------------------------------------------
    // Fire (`BlockFire.java`: ID 51)
    // ---------------------------------------------------------------------

    /// Java `BlockFire.a[]` — chance to encourage fire spread.
    pub fn fire_encourage_chance(id: u8) -> i32 {
        match id {
            blocks::WOOD_PLANKS | blocks::FENCE | blocks::WOOD_STAIRS | blocks::LOG => 5,
            blocks::TNT => 15,
            blocks::LEAVES | blocks::BOOKSHELF | blocks::WOOL => 30,
            blocks::TALL_GRASS => 60,
            _ => 0,
        }
    }

    /// Java `BlockFire.b[]` — ability for block to be consumed by fire.
    pub fn fire_burn_ability(id: u8) -> i32 {
        match id {
            blocks::LOG => 5,
            blocks::WOOD_PLANKS | blocks::FENCE | blocks::WOOD_STAIRS | blocks::BOOKSHELF => 20,
            blocks::LEAVES | blocks::WOOL => 60,
            blocks::TNT | blocks::TALL_GRASS => 100,
            _ => 0,
        }
    }

    fn fire_encourage_for(&self, id: u8) -> i32 {
        if (id == blocks::FENCE || id == blocks::WOOD_STAIRS)
            && !self.mechanics.flammable_fences_stairs
        {
            return 0;
        }
        Self::fire_encourage_chance(id)
    }

    fn fire_burn_for(&self, id: u8) -> i32 {
        if (id == blocks::FENCE || id == blocks::WOOD_STAIRS)
            && !self.mechanics.flammable_fences_stairs
        {
            return 0;
        }
        Self::fire_burn_ability(id)
    }

    /// Java `BlockFire.g(World, i, j, k)`: does any of the 6 neighbors have `a[id] > 0`?
    pub fn has_flammable_neighbor(&self, x: i32, y: i32, z: i32) -> bool {
        self.fire_encourage_for(self.get_block(x + 1, y, z)) > 0
            || self.fire_encourage_for(self.get_block(x - 1, y, z)) > 0
            || self.fire_encourage_for(self.get_block(x, y - 1, z)) > 0
            || self.fire_encourage_for(self.get_block(x, y + 1, z)) > 0
            || self.fire_encourage_for(self.get_block(x, y, z - 1)) > 0
            || self.fire_encourage_for(self.get_block(x, y, z + 1)) > 0
    }

    fn max_neighbor_fire_encourage(&self, x: i32, y: i32, z: i32) -> i32 {
        if self.get_block(x, y, z) != blocks::AIR {
            return 0;
        }
        let mut m = 0;
        for (nx, ny, nz) in [
            (x + 1, y, z),
            (x - 1, y, z),
            (x, y - 1, z),
            (x, y + 1, z),
            (x, y, z - 1),
            (x, y, z + 1),
        ] {
            m = m.max(self.fire_encourage_for(self.get_block(nx, ny, nz)));
        }
        m
    }

    /// Java `BlockFire.a(World, i, j, k, bound, Random, age)` (`BlockFire.java:154-186`).
    fn try_burn_neighbor_block(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        bound: i32,
        age: i32,
        rng: &mut JavaRandom,
    ) {
        if !(0..128).contains(&y) {
            return;
        }
        let target_id = self.get_block(x, y, z);
        let ability = self.fire_burn_for(target_id);
        if rng.next_i32_bound(bound) < ability {
            let is_tnt = target_id == blocks::TNT;
            let keep_fire = if self.mechanics.pre_1_6_fire {
                rng.next_i32_bound(2) == 0
            } else {
                rng.next_i32_bound(age + 10) < 5 && !self.is_being_rained_on(x, y, z)
            };
            if keep_fire {
                let new_age = if self.mechanics.pre_1_6_fire {
                    0
                } else {
                    (age + rng.next_i32_bound(5) / 4).min(15) as u8
                };
                self.java_set_type_id_and_data(x, y, z, blocks::FIRE, new_age);
            } else {
                self.java_set_type_id(x, y, z, blocks::AIR);
            }
            if is_tnt {
                self.ignite_tnt(x, y, z, false);
            }
        }
    }

    /// Java `BlockFire.a(World, i, j, k, Random)` (`BlockFire.java:61-152`).
    pub fn tick_fire(&mut self, x: i32, y: i32, z: i32) {
        let on_netherrack = y > 0 && self.get_block(x, y - 1, z) == blocks::NETHERRACK;
        let old_fire = self.mechanics.pre_1_6_fire;
        if !old_fire
            && !self.is_solid_normal_at(x, y - 1, z)
            && !self.has_flammable_neighbor(x, y, z)
        {
            self.java_set_type_id(x, y, z, blocks::AIR);
            return;
        }
        if !on_netherrack
            && self.is_raining()
            && (self.is_being_rained_on(x, y, z)
                || self.is_being_rained_on(x - 1, y, z)
                || self.is_being_rained_on(x + 1, y, z)
                || self.is_being_rained_on(x, y, z - 1)
                || self.is_being_rained_on(x, y, z + 1))
        {
            self.java_set_type_id(x, y, z, blocks::AIR);
            return;
        }

        let mut rng = std::mem::replace(&mut self.tick_random, JavaRandom::new(0));
        let age = self.get_data(x, y, z) as i32;
        if old_fire {
            if age < 15 {
                self.java_set_data(x, y, z, (age + 1) as u8);
                self.java_schedule_tick(x, y, z, blocks::FIRE, 10);
            }
        } else {
            if age < 15 {
                let next_age = (age + rng.next_i32_bound(3) / 2).min(15) as u8;
                self.java_set_raw_data(x, y, z, next_age);
            }
            self.java_schedule_tick(x, y, z, blocks::FIRE, 40);
        }

        if !on_netherrack && !self.has_flammable_neighbor(x, y, z) {
            if !self.is_solid_normal_at(x, y - 1, z) || age > 3 {
                self.java_set_type_id(x, y, z, blocks::AIR);
            }
            self.tick_random = rng;
            return;
        }

        if !on_netherrack
            && self.fire_encourage_for(self.get_block(x, y - 1, z)) == 0
            && age == 15
            && rng.next_i32_bound(4) == 0
        {
            self.java_set_type_id(x, y, z, blocks::AIR);
            self.tick_random = rng;
            return;
        }

        if !self.mechanics.fire_spread {
            self.tick_random = rng;
            return;
        }

        if !old_fire || (age % 2 == 0 && age > 2) {
            self.try_burn_neighbor_block(x + 1, y, z, 300, age, &mut rng);
            self.try_burn_neighbor_block(x - 1, y, z, 300, age, &mut rng);
            self.try_burn_neighbor_block(x, y - 1, z, 250, age, &mut rng);
            self.try_burn_neighbor_block(x, y + 1, z, 250, age, &mut rng);
            self.try_burn_neighbor_block(x, y, z - 1, 300, age, &mut rng);
            self.try_burn_neighbor_block(x, y, z + 1, 300, age, &mut rng);

            for ix in (x - 1)..=(x + 1) {
                for iz in (z - 1)..=(z + 1) {
                    for iy in (y - 1)..=(y + 4) {
                        if !(1..127).contains(&iy) || (ix == x && iy == y && iz == z) {
                            continue;
                        }
                        let mut chance_bound = 100;
                        if iy > y + 1 {
                            chance_bound += (iy - (y + 1)) * 100;
                        }
                        let enc = self.max_neighbor_fire_encourage(ix, iy, iz);
                        if enc > 0 {
                            let spread_threshold = if old_fire {
                                enc
                            } else {
                                (enc + 40) / (age + 30)
                            };
                            if spread_threshold > 0
                                && rng.next_i32_bound(chance_bound) <= spread_threshold
                                && (!self.is_raining()
                                    || (!self.is_being_rained_on(ix, iy, iz)
                                        && !self.is_being_rained_on(ix - 1, iy, iz)
                                        && !self.is_being_rained_on(ix + 1, iy, iz)
                                        && !self.is_being_rained_on(ix, iy, iz - 1)
                                        && !self.is_being_rained_on(ix, iy, iz + 1)))
                            {
                                let new_age = if old_fire {
                                    0
                                } else {
                                    (age + rng.next_i32_bound(5) / 4).min(15) as u8
                                };
                                self.java_set_type_id_and_data(ix, iy, iz, blocks::FIRE, new_age);
                            }
                        }
                    }
                }
            }
        }

        self.tick_random = rng;
    }

    // ---------------------------------------------------------------------
    // Weather, Portals, Crops, and Block Helpers (`World.java`, `Block*.java`)
    // ---------------------------------------------------------------------

    pub fn is_raining(&self) -> bool {
        self.level_data.borrow().raining
    }

    pub fn is_thundering(&self) -> bool {
        self.level_data.borrow().thundering
    }

    /// Java `World.e(int i, int j)` (`World.java:1008-1024`): Y above highest solid or liquid block.
    pub fn top_solid_or_liquid_block(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let Some(chunk) = self.chunks.get(&pos) else {
            return -1;
        };
        let lx = (x & 15) as usize;
        let lz = (z & 15) as usize;
        for y in (1..=127).rev() {
            let id = chunk.get_block(lx, y, lz);
            let mat = blocks::material(id);
            if mat.blocks_movement() || mat.is_liquid() {
                return y as i32 + 1;
            }
        }
        -1
    }

    /// Java `World.s(int i, int j, int k)` (`World.java:2420-2432`): true if `(x, y, z)` is exposed to liquid rain.
    pub fn is_being_rained_on(&self, x: i32, y: i32, z: i32) -> bool {
        if !self.is_raining() {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return false;
        }
        if self.top_solid_or_liquid_block(x, z) > y {
            return false;
        }
        let seed = self.seed();
        let mut mgr = crate::world::biome::WorldChunkManager::new(seed);
        let biome = mgr.biome_at(x, z);
        biome.can_rain()
    }

    /// Java `World.i()` (`World.java:1770-1850`): tick weather timers; returns `Some(new_raining)` when rain state changes.
    pub fn tick_weather(&mut self) -> Option<bool> {
        if !self.mechanics.do_weather {
            let mut ld = self.level_data.borrow_mut();
            let was_raining = ld.raining;
            ld.raining = false;
            ld.thundering = false;
            return if was_raining { Some(false) } else { None };
        }
        let mut rng = std::mem::replace(&mut self.tick_random, JavaRandom::new(0));
        let mut ld = self.level_data.borrow_mut();
        let was_raining = ld.raining;

        if ld.thunder_time <= 0 {
            if ld.thundering {
                ld.thunder_time = rng.next_i32_bound(12000) + 3600;
            } else {
                ld.thunder_time = rng.next_i32_bound(168_000) + 12000;
            }
        } else {
            ld.thunder_time -= 1;
            if ld.thunder_time <= 0 {
                ld.thundering = !ld.thundering;
            }
        }

        if ld.rain_time <= 0 {
            if ld.raining {
                ld.rain_time = rng.next_i32_bound(12000) + 12000;
            } else {
                ld.rain_time = rng.next_i32_bound(168_000) + 12000;
            }
        } else {
            ld.rain_time -= 1;
            if ld.rain_time <= 0 {
                ld.raining = !ld.raining;
            }
        }

        let new_raining = ld.raining;
        drop(ld);
        self.tick_random = rng;
        if new_raining != was_raining {
            Some(new_raining)
        } else {
            None
        }
    }

    /// Java `World.y()` (`World.java:1852-1868`): clear rain and thunder (e.g. when waking from beds).
    /// Returns true if it was raining before clearing.
    pub fn reset_weather(&mut self) -> bool {
        let mut ld = self.level_data.borrow_mut();
        let was_raining = ld.raining;
        ld.rain_time = 0;
        ld.raining = false;
        ld.thunder_time = 0;
        ld.thundering = false;
        was_raining
    }

    /// Java `EntityWeatherStorm` constructor block ignition (`EntityWeatherStorm.java:41-73`).
    pub fn strike_lightning_at(&mut self, x: i32, y: i32, z: i32) {
        let mut rng = std::mem::replace(&mut self.tick_random, JavaRandom::new(0));
        if self.chunks_area_loaded(x - 10, y - 10, z - 10, x + 10, y + 10, z + 10) {
            if (1..128).contains(&y)
                && self.get_block(x, y, z) == blocks::AIR
                && (self.is_solid_normal_at(x, y - 1, z) || self.has_flammable_neighbor(x, y, z))
            {
                self.java_set_type_id(x, y, z, blocks::FIRE);
            }
            for _ in 0..4 {
                let fx = x + rng.next_i32_bound(3) - 1;
                let fy = y + rng.next_i32_bound(3) - 1;
                let fz = z + rng.next_i32_bound(3) - 1;
                if (1..128).contains(&fy)
                    && self.get_block(fx, fy, fz) == blocks::AIR
                    && (self.is_solid_normal_at(fx, fy - 1, fz)
                        || self.has_flammable_neighbor(fx, fy, fz))
                {
                    self.java_set_type_id(fx, fy, fz, blocks::FIRE);
                }
            }
        }
        self.tick_random = rng;
        self.world_events
            .push(WorldEvent::LightningStrike { x, y, z });
    }

    /// Java `BlockCrops.h(World, i, j, k)` (`BlockCrops.java:40-79`).
    pub fn crop_growth_rate(&self, x: i32, y: i32, z: i32) -> f32 {
        let mut f = 1.0f32;
        let id = blocks::CROPS;
        let n = self.get_block(x, y, z - 1);
        let s = self.get_block(x, y, z + 1);
        let w = self.get_block(x - 1, y, z);
        let e = self.get_block(x + 1, y, z);
        let nw = self.get_block(x - 1, y, z - 1);
        let ne = self.get_block(x + 1, y, z - 1);
        let se = self.get_block(x + 1, y, z + 1);
        let sw = self.get_block(x - 1, y, z + 1);
        let flag = w == id || e == id;
        let flag1 = n == id || s == id;
        let flag2 = nw == id || ne == id || se == id || sw == id;

        for lx in (x - 1)..=(x + 1) {
            for lz in (z - 1)..=(z + 1) {
                let soil = self.get_block(lx, y - 1, lz);
                let mut f1 = 0.0f32;
                if soil == blocks::FARMLAND {
                    f1 = 1.0;
                    if self.get_data(lx, y - 1, lz) > 0 {
                        f1 = 3.0;
                    }
                }
                if lx != x || lz != z {
                    f1 /= 4.0;
                }
                f += f1;
            }
        }
        if flag2 || (flag && flag1) {
            f /= 2.0;
        }
        f
    }

    /// Java `BlockSoil.b(World, i, j, k, Entity)` (`BlockSoil.java:53-71`): 1/4 chance to trample farmland into dirt.
    pub fn trample_farmland(&mut self, x: i32, y: i32, z: i32) {
        if (0..128).contains(&y) && self.get_block(x, y, z) == blocks::FARMLAND {
            let roll = self.tick_random.next_i32_bound(4);
            if roll == 0 {
                self.java_set_type_id(x, y, z, blocks::DIRT);
            }
        }
    }

    /// Java `BlockPortal.a_(World, i, j, k)` (`BlockPortal.java:44-118`): validate 4x5 obsidian frame and fill 2x3 with PORTAL.
    pub fn try_create_portal(&mut self, mut x: i32, y: i32, mut z: i32) -> bool {
        if !self.allow_nether {
            return false;
        }
        let mut b0 = 0i32;
        let mut b1 = 0i32;
        if self.get_block(x - 1, y, z) == blocks::OBSIDIAN
            || self.get_block(x + 1, y, z) == blocks::OBSIDIAN
        {
            b0 = 1;
        }
        if self.get_block(x, y, z - 1) == blocks::OBSIDIAN
            || self.get_block(x, y, z + 1) == blocks::OBSIDIAN
        {
            b1 = 1;
        }
        if b0 == b1 {
            return false;
        }
        if self.get_block(x - b0, y, z - b1) == blocks::AIR {
            x -= b0;
            z -= b1;
        }
        for l in -1..=2 {
            for i1 in -1..=3 {
                let is_frame = l == -1 || l == 2 || i1 == -1 || i1 == 3;
                if (l != -1 && l != 2) || (i1 != -1 && i1 != 3) {
                    let ny = y + i1;
                    if !(0..128).contains(&ny) {
                        return false;
                    }
                    let id = self.get_block(x + b0 * l, ny, z + b1 * l);
                    if is_frame {
                        if id != blocks::OBSIDIAN {
                            return false;
                        }
                    } else if id != blocks::AIR && id != blocks::FIRE {
                        return false;
                    }
                }
            }
        }
        let prev_suppress = self.suppress_physics;
        self.suppress_physics = true;
        for l in 0..2 {
            for i1 in 0..3 {
                self.java_set_type_id(x + b0 * l, y + i1, z + b1 * l, blocks::PORTAL);
            }
        }
        self.suppress_physics = prev_suppress;
        true
    }

    /// Java `BlockPortal.doPhysics(World, i, j, k, l)` (`BlockPortal.java:120-157`).
    pub fn do_physics_portal(&mut self, x: i32, y: i32, z: i32) {
        let mut b0 = 0i32;
        let mut b1 = 1i32;
        if self.get_block(x - 1, y, z) == blocks::PORTAL
            || self.get_block(x + 1, y, z) == blocks::PORTAL
        {
            b0 = 1;
            b1 = 0;
        }
        let mut bottom_y = y;
        while bottom_y > 0 && self.get_block(x, bottom_y - 1, z) == blocks::PORTAL {
            bottom_y -= 1;
        }
        if bottom_y == 0 || self.get_block(x, bottom_y - 1, z) != blocks::OBSIDIAN {
            self.java_set_type_id(x, y, z, blocks::AIR);
            return;
        }
        let mut height = 1i32;
        while height < 4
            && bottom_y + height < 128
            && self.get_block(x, bottom_y + height, z) == blocks::PORTAL
        {
            height += 1;
        }
        if height != 3
            || bottom_y + 3 >= 128
            || self.get_block(x, bottom_y + 3, z) != blocks::OBSIDIAN
        {
            self.java_set_type_id(x, y, z, blocks::AIR);
            return;
        }
        let adj_x = self.get_block(x - 1, y, z) == blocks::PORTAL
            || self.get_block(x + 1, y, z) == blocks::PORTAL;
        let adj_z = self.get_block(x, y, z - 1) == blocks::PORTAL
            || self.get_block(x, y, z + 1) == blocks::PORTAL;
        if adj_x && adj_z {
            self.java_set_type_id(x, y, z, blocks::AIR);
            return;
        }
        let valid_side_1 = self.get_block(x + b0, y, z + b1) == blocks::OBSIDIAN
            && self.get_block(x - b0, y, z - b1) == blocks::PORTAL;
        let valid_side_2 = self.get_block(x - b0, y, z - b1) == blocks::OBSIDIAN
            && self.get_block(x + b0, y, z + b1) == blocks::PORTAL;
        if !valid_side_1 && !valid_side_2 {
            self.java_set_type_id(x, y, z, blocks::AIR);
        }
    }

    // ---------------------------------------------------------------------
    // PortalTravelAgent (`PortalTravelAgent.java`)
    // ---------------------------------------------------------------------

    /// Java `PortalTravelAgent.b(World, Entity)` (lines 24-88): nearest portal
    /// block within ±128 blocks, snapped to the bottom-center of the column.
    pub fn find_portal(&mut self, x: f64, y: f64, z: f64) -> Option<(f64, f64, f64)> {
        const RADIUS: i32 = 128;
        let ox = x.floor() as i32;
        let oz = z.floor() as i32;
        let mut best = -1.0f64;
        let (mut bx, mut by, mut bz) = (0i32, 0i32, 0i32);

        for px in ox - RADIUS..=ox + RADIUS {
            let dx = px as f64 + 0.5 - x;
            for pz in oz - RADIUS..=oz + RADIUS {
                let dz = pz as f64 + 0.5 - z;
                let mut py = 127i32;
                while py >= 0 {
                    if self.get_block(px, py, pz) == blocks::PORTAL {
                        while py > 0 && self.get_block(px, py - 1, pz) == blocks::PORTAL {
                            py -= 1;
                        }
                        let dy = py as f64 + 0.5 - y;
                        let dist = dx * dx + dy * dy + dz * dz;
                        if best < 0.0 || dist < best {
                            best = dist;
                            bx = px;
                            by = py;
                            bz = pz;
                        }
                    }
                    py -= 1;
                }
            }
        }

        if best < 0.0 {
            return None;
        }

        let mut sx = bx as f64 + 0.5;
        let sy = by as f64 + 0.5;
        let mut sz = bz as f64 + 0.5;
        if self.get_block(bx - 1, by, bz) == blocks::PORTAL {
            sx -= 0.5;
        }
        if self.get_block(bx + 1, by, bz) == blocks::PORTAL {
            sx += 0.5;
        }
        if self.get_block(bx, by, bz - 1) == blocks::PORTAL {
            sz -= 0.5;
        }
        if self.get_block(bx, by, bz + 1) == blocks::PORTAL {
            sz += 0.5;
        }
        Some((sx, sy, sz))
    }

    /// Java `PortalTravelAgent.c(World, Entity)` (lines 90-324): find a clear
    /// 4x5 footprint (or clamp to y 70..=118 and carve one), build the
    /// obsidian frame, and fill the 2x3 interior with PORTAL.
    pub fn create_portal(&mut self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        const RADIUS: i32 = 16;
        let ox = x.floor() as i32;
        let oy = y.floor() as i32;
        let oz = z.floor() as i32;
        let mut best = -1.0f64;
        let (mut bx, mut by, mut bz) = (ox, oy, oz);
        let mut rotation = 0i32;
        let start_rotation = self.tick_random.next_i32_bound(4);

        // First pass: prefer a 4x5x4 clear volume above solid ground.
        for px in ox - RADIUS..=ox + RADIUS {
            let dx = px as f64 + 0.5 - x;
            for pz in oz - RADIUS..=oz + RADIUS {
                let dz = pz as f64 + 0.5 - z;
                let mut py = 127i32;
                while py >= 0 {
                    if self.get_block(px, py, pz) == blocks::AIR {
                        while py > 0 && self.get_block(px, py - 1, pz) == blocks::AIR {
                            py -= 1;
                        }
                        for rot in start_rotation..start_rotation + 4 {
                            let side_a = rot % 2;
                            let side_b = 1 - side_a;
                            let (side_a, side_b) = if rot % 4 >= 2 {
                                (-side_a, -side_b)
                            } else {
                                (side_a, side_b)
                            };
                            let mut clear = true;
                            'candidate: for depth in 0..3 {
                                for width in 0..4i32 {
                                    for hy in -1..4i32 {
                                        let cx = px + (width - 1) * side_a + depth * side_b;
                                        let cy = py + hy;
                                        let cz = pz + (width - 1) * side_b - depth * side_a;
                                        let buildable = blocks::material_is_buildable(
                                            self.get_block(cx, cy, cz),
                                        );
                                        let empty = self.get_block(cx, cy, cz) == blocks::AIR;
                                        if (hy < 0 && !buildable) || (hy >= 0 && !empty) {
                                            clear = false;
                                            break 'candidate;
                                        }
                                    }
                                }
                            }
                            if !clear {
                                // Java `continue label271`: abandon every
                                // rotation for this column and scan downward.
                                break;
                            }
                            let dy = py as f64 + 0.5 - y;
                            let dist = dx * dx + dy * dy + dz * dz;
                            if best < 0.0 || dist < best {
                                best = dist;
                                bx = px;
                                by = py;
                                bz = pz;
                                rotation = rot % 4;
                            }
                        }
                    }
                    py -= 1;
                }
            }
        }

        // Second pass: any 2x3 pocket (used when no 4x5 volume exists).
        if best < 0.0 {
            for px in ox - RADIUS..=ox + RADIUS {
                let dx = px as f64 + 0.5 - x;
                for pz in oz - RADIUS..=oz + RADIUS {
                    let dz = pz as f64 + 0.5 - z;
                    let mut py = 127i32;
                    while py >= 0 {
                        if self.get_block(px, py, pz) == blocks::AIR {
                            while py > 0 && self.get_block(px, py - 1, pz) == blocks::AIR {
                                py -= 1;
                            }
                            for rot in start_rotation..start_rotation + 2 {
                                let side_a = rot % 2;
                                let side_b = 1 - side_a;
                                let mut clear = true;
                                'candidate2: for width in 0..4i32 {
                                    for hy in -1..4i32 {
                                        let cx = px + (width - 1) * side_a;
                                        let cy = py + hy;
                                        let cz = pz + (width - 1) * side_b;
                                        let buildable = blocks::material_is_buildable(
                                            self.get_block(cx, cy, cz),
                                        );
                                        let empty = self.get_block(cx, cy, cz) == blocks::AIR;
                                        if (hy < 0 && !buildable) || (hy >= 0 && !empty) {
                                            clear = false;
                                            break 'candidate2;
                                        }
                                    }
                                }
                                if !clear {
                                    // Java `continue label219` (second pass).
                                    break;
                                }
                                let dy = py as f64 + 0.5 - y;
                                let dist = dx * dx + dy * dy + dz * dz;
                                if best < 0.0 || dist < best {
                                    best = dist;
                                    bx = px;
                                    by = py;
                                    bz = pz;
                                    rotation = rot % 2;
                                }
                            }
                        }
                        py -= 1;
                    }
                }
            }
        }

        let mut axis_a = rotation % 2;
        let mut axis_b = 1 - axis_a;
        if rotation % 4 >= 2 {
            axis_a = -axis_a;
            axis_b = -axis_b;
        }

        let mut base_y = by;
        if best < 0.0 {
            base_y = base_y.clamp(70, 118);
            for depth in -1..=1 {
                for width in 1..3 {
                    for hy in -1..3 {
                        let cx = bx + (width - 1) * axis_a + depth * axis_b;
                        let cy = base_y + hy;
                        let cz = bz + (width - 1) * axis_b - depth * axis_a;
                        let id = if hy < 0 {
                            blocks::OBSIDIAN
                        } else {
                            blocks::AIR
                        };
                        self.java_set_type_id(cx, cy, cz, id);
                    }
                }
            }
        }

        for _depth in 0..4 {
            self.suppress_physics = true;
            for width in 0..4 {
                for hy in -1..4 {
                    let cx = bx + (width - 1) * axis_a;
                    let cy = base_y + hy;
                    let cz = bz + (width - 1) * axis_b;
                    let edge = width == 0 || width == 3 || hy == -1 || hy == 3;
                    let id = if edge {
                        blocks::OBSIDIAN
                    } else {
                        blocks::PORTAL
                    };
                    self.java_set_type_id(cx, cy, cz, id);
                }
            }
            self.suppress_physics = false;
            for width in 0..4 {
                for hy in -1..4 {
                    let cx = bx + (width - 1) * axis_a;
                    let cy = base_y + hy;
                    let cz = bz + (width - 1) * axis_b;
                    let id = self.get_block(cx, cy, cz);
                    self.java_apply_physics(cx, cy, cz, id as i32);
                }
            }
        }

        (bx as f64 + 0.5, base_y as f64 + 0.5, bz as f64 + 0.5)
    }

    /// Java `PortalTravelAgent.a(World, Entity)`: snap to an existing portal or
    /// create one, then snap to the result.
    pub fn find_or_create_portal(&mut self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        if let Some(pos) = self.find_portal(x, y, z) {
            return pos;
        }
        let created = self.create_portal(x, y, z);
        self.find_portal(created.0, created.1, created.2)
            .unwrap_or(created)
    }

    // ---------------------------------------------------------------------
    // Random Block Ticks (`World.j()`)
    // ---------------------------------------------------------------------

    pub fn tick_random_blocks(&mut self, player_positions: &[(f64, f64)]) {
        if player_positions.is_empty() {
            return;
        }

        // The set is sorted: the order of the chunks decides the draws of
        // the random of the world, and the order of a hash table of Rust is
        // random for every process.
        let mut active_chunks = BTreeSet::new();
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

            // 1. Thunderstorm lightning strike (1/100000 per chunk per tick, World.java:1927-1937)
            if rng.next_i32_bound(100_000) == 0 && self.is_raining() && self.is_thundering() {
                lcg = lcg.wrapping_mul(3).wrapping_add(1_013_904_223);
                let k = lcg >> 2;
                let wx = base_x + (k & 15);
                let wz = base_z + ((k >> 8) & 15);
                let wy = self.top_solid_or_liquid_block(wx, wz);
                if self.is_being_rained_on(wx, wy, wz) {
                    self.tick_random = rng;
                    self.strike_lightning_at(wx, wy, wz);
                    rng = std::mem::replace(&mut self.tick_random, JavaRandom::new(0));
                }
            }

            // 2. Snow & ice formation in cold biomes (1/16 per chunk per tick, World.java:1941-1976)
            if rng.next_i32_bound(16) == 0 {
                lcg = lcg.wrapping_mul(3).wrapping_add(1_013_904_223);
                let k = lcg >> 2;
                let lx = k & 15;
                let lz = (k >> 8) & 15;
                let wx = base_x + lx;
                let wz = base_z + lz;
                let wy = self.top_solid_or_liquid_block(wx, wz);
                if (0..128).contains(&wy) {
                    let mut mgr = crate::world::biome::WorldChunkManager::new(self.seed());
                    if mgr.biome_at(wx, wz).can_spawn_snow()
                        && self.java_get_saved_light(
                            crate::world::lighting::LightType::Block,
                            wx,
                            wy,
                            wz,
                        ) < 10
                    {
                        let below = if wy > 0 {
                            self.get_block(wx, wy - 1, wz)
                        } else {
                            0
                        };
                        let at = self.get_block(wx, wy, wz);
                        if self.is_raining()
                            && at == blocks::AIR
                            && below != blocks::AIR
                            && below != blocks::ICE
                            && blocks::is_opaque_cube(below)
                            && blocks::material(below).blocks_movement()
                        {
                            self.java_set_type_id(wx, wy, wz, blocks::SNOW);
                        }
                        if wy > 0
                            && below == blocks::STATIONARY_WATER
                            && self.get_data(wx, wy - 1, wz) == 0
                            && (!self.mechanics.ice_generate_only_when_snowing || self.is_raining())
                        {
                            self.java_set_type_id(wx, wy - 1, wz, blocks::ICE);
                        }
                    }
                }
            }

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
                            if meta < 7 {
                                let rate = self.crop_growth_rate(wx, y, wz);
                                let bound = ((100.0f32 / rate) as i32).max(1);
                                if rng.next_i32_bound(bound) == 0 {
                                    self.java_set_data(wx, y, wz, meta + 1);
                                }
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
                        // BlockSoil.a: 1/5 chance to check hydration / rain
                        if rng.next_i32_bound(5) == 0 {
                            let mut has_water = false;
                            'water: for dx in -4..=4 {
                                for dy in 0..=1 {
                                    for dz in -4..=4 {
                                        if blocks::is_water(self.get_block(
                                            wx + dx,
                                            y + dy,
                                            wz + dz,
                                        )) {
                                            has_water = true;
                                            break 'water;
                                        }
                                    }
                                }
                            }
                            let rained_on = self.is_being_rained_on(wx, y + 1, wz);
                            if !has_water && !rained_on {
                                let meta = self.get_data(wx, y, wz);
                                if meta > 0 {
                                    self.java_set_data(wx, y, wz, meta - 1);
                                } else if self.get_block(wx, y + 1, wz) != blocks::CROPS {
                                    self.java_set_type_id(wx, y, wz, blocks::DIRT);
                                }
                            } else {
                                self.java_set_data(wx, y, wz, 7);
                            }
                        }
                    }
                    blocks::STATIONARY_LAVA => {
                        // BlockStationary.a (BlockStationary.java:34-76): ignite nearby burnable blocks
                        let count = rng.next_i32_bound(3);
                        let mut tx = wx;
                        let mut ty = y;
                        let mut tz = wz;
                        for _ in 0..count {
                            tx += rng.next_i32_bound(3) - 1;
                            ty += 1;
                            tz += rng.next_i32_bound(3) - 1;
                            if !(0..128).contains(&ty) {
                                break;
                            }
                            let tid = self.get_block(tx, ty, tz);
                            if tid == blocks::AIR {
                                let near_burnable =
                                    blocks::material(self.get_block(tx - 1, ty, tz)).is_burnable()
                                        || blocks::material(self.get_block(tx + 1, ty, tz))
                                            .is_burnable()
                                        || blocks::material(self.get_block(tx, ty, tz - 1))
                                            .is_burnable()
                                        || blocks::material(self.get_block(tx, ty, tz + 1))
                                            .is_burnable()
                                        || blocks::material(self.get_block(tx, ty - 1, tz))
                                            .is_burnable()
                                        || blocks::material(self.get_block(tx, ty + 1, tz))
                                            .is_burnable();
                                if near_burnable {
                                    self.java_set_type_id(tx, ty, tz, blocks::FIRE);
                                    break;
                                }
                            } else if blocks::material(tid).blocks_movement() {
                                break;
                            }
                        }
                    }
                    blocks::BROWN_MUSHROOM | blocks::RED_MUSHROOM => {
                        // BlockMushroom.a (BlockMushroom.java:17-51)
                        if y > 0
                            && (self.light_at(wx, y, wz) >= 13
                                || !blocks::is_opaque_cube(self.get_block(wx, y - 1, wz)))
                        {
                            self.world_events.push(WorldEvent::DropItem {
                                x: wx as f64,
                                y: y as f64,
                                z: wz as f64,
                                item: ItemStack::new(block_id as i16, 1, 0),
                            });
                            self.java_set_type_id(wx, y, wz, blocks::AIR);
                        } else if self.mechanics.mushroom_spread && rng.next_i32_bound(100) == 0 {
                            let tx = wx + rng.next_i32_bound(3) - 1;
                            let ty = y + rng.next_i32_bound(2) - rng.next_i32_bound(2);
                            let tz = wz + rng.next_i32_bound(3) - 1;
                            if (1..128).contains(&ty)
                                && self.get_block(tx, ty, tz) == blocks::AIR
                                && self.light_at(tx, ty, tz) < 13
                                && blocks::is_opaque_cube(self.get_block(tx, ty - 1, tz))
                            {
                                let _ = rng.next_i32_bound(3);
                                let _ = rng.next_i32_bound(3);
                                self.java_set_type_id(tx, ty, tz, block_id);
                            }
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
                            self.world_events.push(WorldEvent::DropItem {
                                x: wx as f64,
                                y: y as f64,
                                z: wz as f64,
                                item: ItemStack::new(crate::gameplay::items::SNOW_BALL, 1, 0),
                            });
                            self.java_set_type_id(wx, y, wz, blocks::AIR);
                        }
                    }
                    blocks::SNOW_BLOCK => {
                        let block_light = self.java_get_saved_light(
                            crate::world::lighting::LightType::Block,
                            wx,
                            y,
                            wz,
                        );
                        if block_light > 11 {
                            for _ in 0..4 {
                                self.world_events.push(WorldEvent::DropItem {
                                    x: wx as f64,
                                    y: y as f64,
                                    z: wz as f64,
                                    item: ItemStack::new(crate::gameplay::items::SNOW_BALL, 1, 0),
                                });
                            }
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
                        self.tick_random = rng;
                        self.tick_fire(wx, y, wz);
                        rng = std::mem::replace(&mut self.tick_random, JavaRandom::new(0));
                    }
                    blocks::LEAVES => {
                        if !self.mechanics.leaf_decay {
                            continue;
                        }
                        let meta = self.get_data(wx, y, wz);
                        if (meta & 8) != 0 {
                            // Exact 4-step 3D Manhattan BFS (`BlockLeaves.java:39-131`)
                            if self.chunks_area_loaded(wx - 5, y - 5, wz - 5, wx + 5, y + 5, wz + 5)
                            {
                                let mut grid = [-1i8; 13 * 13 * 13];
                                for dx in -4..=4i32 {
                                    for dy in -4..=4i32 {
                                        for dz in -4..=4i32 {
                                            let id = self.get_block(wx + dx, y + dy, wz + dz);
                                            let idx = ((dx + 6) * 169 + (dy + 6) * 13 + (dz + 6))
                                                as usize;
                                            if id == blocks::LOG {
                                                grid[idx] = 0;
                                            } else if id == blocks::LEAVES {
                                                grid[idx] = -2;
                                            } else {
                                                grid[idx] = -1;
                                            }
                                        }
                                    }
                                }
                                for step in 1..=4i8 {
                                    for dx in -4..=4i32 {
                                        for dy in -4..=4i32 {
                                            for dz in -4..=4i32 {
                                                let idx =
                                                    ((dx + 6) * 169 + (dy + 6) * 13 + (dz + 6))
                                                        as usize;
                                                if grid[idx] == step - 1 {
                                                    for (nx, ny, nz) in [
                                                        (dx - 1, dy, dz),
                                                        (dx + 1, dy, dz),
                                                        (dx, dy - 1, dz),
                                                        (dx, dy + 1, dz),
                                                        (dx, dy, dz - 1),
                                                        (dx, dy, dz + 1),
                                                    ] {
                                                        let n_idx = ((nx + 6) * 169
                                                            + (ny + 6) * 13
                                                            + (nz + 6))
                                                            as usize;
                                                        if grid[n_idx] == -2 {
                                                            grid[n_idx] = step;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                let center_val = grid[6 * 169 + 6 * 13 + 6];
                                if center_val >= 0 {
                                    self.java_set_raw_data(wx, y, wz, meta & !8);
                                } else {
                                    if rng.next_i32_bound(20) == 0 {
                                        let sapling_meta =
                                            if self.mechanics.drop_saplings_of_leaf_type {
                                                (meta & 3) as i16
                                            } else {
                                                0
                                            };
                                        self.world_events.push(WorldEvent::DropItem {
                                            x: wx as f64,
                                            y: y as f64,
                                            z: wz as f64,
                                            item: ItemStack::new(
                                                blocks::SAPLING as i16,
                                                1,
                                                sapling_meta,
                                            ),
                                        });
                                    }
                                    self.java_set_type_id(wx, y, wz, blocks::AIR);
                                }
                            } else {
                                self.java_set_raw_data(wx, y, wz, meta & !8);
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
        assert_eq!(world.get_block(5, 64, 5), blocks::STICKY_PISTON);
    }

    #[test]
    fn piston_never_deletes_itself_when_breaking_power_source_or_retracting() {
        let mut world = make_flat_world();
        // Piston facing UP (dir = 1) at (5, 64, 5) with a Lever on top of it at (5, 65, 5)
        world.java_set_type_id_and_data(5, 64, 5, blocks::PISTON, 1);
        world.java_set_type_id_and_data(5, 65, 5, blocks::LEVER, 13);
        world.notify_attachment_neighbors(5, 65, 5, blocks::LEVER, 5);

        // Even after breaking its own lever at (5, 65, 5) and retracting, the piston base at (5, 64, 5) MUST remain intact!
        assert_eq!(world.get_block(5, 64, 5), blocks::PISTON);

        // Also test DOWN-facing piston (dir = 0) at (8, 66, 8) extending and retracting
        world.java_set_type_id_and_data(8, 66, 8, blocks::PISTON, 0);
        world.java_set_type_id_and_data(8, 66, 7, blocks::REDSTONE_TORCH_ON, 5);
        assert_eq!(world.get_block(8, 66, 8), blocks::PISTON);
        assert_eq!(world.get_block(8, 65, 8), blocks::PISTON_HEAD);

        world.java_set_type_id(8, 66, 7, blocks::AIR);
        assert_eq!(world.get_block(8, 66, 8), blocks::PISTON);
        assert_eq!(world.get_block(8, 65, 8), blocks::AIR);
    }

    #[test]
    fn rails_connect_along_both_axes_and_form_curves() {
        let mut world = make_flat_world();
        // Place two rails along X axis: (4, 64, 4) and (5, 64, 4)
        world.java_set_type_id(4, 64, 4, blocks::RAIL);
        world.java_set_type_id(5, 64, 4, blocks::RAIL);
        assert_eq!(
            world.get_data(4, 64, 4),
            1,
            "Rail should orient along X axis (meta = 1)"
        );
        assert_eq!(
            world.get_data(5, 64, 4),
            1,
            "Second rail should orient along X axis (meta = 1)"
        );

        // Place a third rail at (5, 64, 5) (south of (5, 64, 4)) to form a corner at (5, 64, 4)
        world.java_set_type_id(5, 64, 5, blocks::RAIL);
        assert_eq!(
            world.get_data(5, 64, 4),
            7,
            "Corner rail with west and south neighbors should curve to meta = 7"
        );
    }

    #[test]
    fn fire_spreads_and_burns_flammable_blocks() {
        let mut world = make_flat_world();
        world.java_set_type_id(10, 64, 10, blocks::WOOD_PLANKS);
        world.java_set_type_id(10, 65, 10, blocks::FIRE);

        let mut changed = false;
        for _ in 0..40 {
            if world.get_block(10, 65, 10) == blocks::FIRE {
                world.tick_fire(10, 65, 10);
            }
            if world.get_block(10, 64, 10) != blocks::WOOD_PLANKS {
                changed = true;
                break;
            }
        }
        assert!(changed, "Fire should eventually burn adjacent wood planks");
    }

    #[test]
    fn nether_portal_frame_ignition_and_breaking() {
        let mut world = make_flat_world();
        // Build a 4x5 Obsidian frame along X: x = 0..=3, y = 64..=68, z = 0
        for dx in 0..=3 {
            for dy in 0..=4 {
                if dx == 0 || dx == 3 || dy == 0 || dy == 4 {
                    world.java_set_type_id(dx, 64 + dy, 0, blocks::OBSIDIAN);
                }
            }
        }
        // Light fire inside at (1, 65, 0) -> should fill 2x3 interior (x=1..=2, y=65..=67) with PORTAL (90)
        world.java_set_type_id(1, 65, 0, blocks::FIRE);
        for dx in 1..=2 {
            for dy in 1..=3 {
                assert_eq!(
                    world.get_block(dx, 64 + dy, 0),
                    blocks::PORTAL,
                    "Interior ({dx}, {}, 0) should be PORTAL",
                    64 + dy
                );
            }
        }

        // Break one obsidian frame block at (0, 65, 0) -> entire 2x3 portal must shatter to AIR!
        world.java_set_type_id(0, 65, 0, blocks::AIR);
        for dx in 1..=2 {
            for dy in 1..=3 {
                assert_eq!(
                    world.get_block(dx, 64 + dy, 0),
                    blocks::AIR,
                    "Portal block ({dx}, {}, 0) should break when frame is broken",
                    64 + dy
                );
            }
        }
    }

    #[test]
    fn crop_growth_rate_hydration_and_diagonal_penalty() {
        let mut world = make_flat_world();
        // Dry farmland at (5, 63, 5) with crops at (5, 64, 5)
        world.java_set_type_id_and_data(5, 63, 5, blocks::FARMLAND, 0);
        world.java_set_type_id(5, 64, 5, blocks::CROPS);
        let dry_rate = world.crop_growth_rate(5, 64, 5);
        assert!((dry_rate - 2.0).abs() < 1e-4);

        // Hydrated farmland at (5, 63, 5) (meta = 7) -> +3.0 instead of +1.0 -> rate = 4.0
        world.java_set_data(5, 63, 5, 7);
        let wet_rate = world.crop_growth_rate(5, 64, 5);
        assert!((wet_rate - 4.0).abs() < 1e-4);

        // Diagonal crop at (6, 64, 6) halves the growth rate (`flag2 == true` -> 4.0 / 2.0 = 2.0)
        world.java_set_type_id(6, 64, 6, blocks::CROPS);
        let diag_rate = world.crop_growth_rate(5, 64, 5);
        assert!((diag_rate - 2.0).abs() < 1e-4);
    }

    #[test]
    fn weather_cycle_and_reset() {
        let mut world = make_flat_world();
        {
            let mut ld = world.level_data.borrow_mut();
            ld.raining = false;
            ld.rain_time = 1;
            ld.thundering = false;
            ld.thunder_time = 1;
        }
        assert_eq!(world.tick_weather(), Some(true));
        assert!(world.is_raining());
        assert!(world.is_thundering());

        assert!(world.reset_weather());
        assert!(!world.is_raining());
        assert!(!world.is_thundering());
    }

    #[test]
    fn minecart_curve_slope_and_powered_rail_physics() {
        use crate::entity::projectile::Minecart;

        let mut world = make_flat_world();
        // Build an L-shaped track:
        // (1, 64, 4) = solid wall behind powered rail
        // (2, 64, 3) = redstone torch powering the golden rail at (2, 64, 4)
        // (2, 64, 4) = powered rail (lit, W-E, meta = 1 | 8 = 9)
        // (3, 64, 4) = straight rail (W-E, meta = 1)
        // (4, 64, 4) = curve rail connecting West (-X) and South (+Z) (meta = 7)
        // (4, 64, 5) = straight rail (N-S, meta = 0)
        // (4, 64, 6) = unpowered golden rail (braking, N-S, meta = 0)
        world.java_set_type_id(1, 64, 4, blocks::STONE);
        world.java_set_type_id_and_data(2, 64, 3, blocks::REDSTONE_TORCH_ON, 5);
        world.java_set_type_id(2, 64, 4, blocks::POWERED_RAIL);
        world.java_set_type_id(3, 64, 4, blocks::RAIL);
        world.java_set_type_id(4, 64, 4, blocks::RAIL);
        world.java_set_type_id(4, 64, 5, blocks::RAIL);
        world.java_set_type_id(4, 64, 6, blocks::POWERED_RAIL);

        // Spawn stationary minecart on the powered rail at (2, 64, 4)
        let mut cart = Minecart::new(1, 0, 2.5, 64.5, 4.5);
        assert!((cart.y - 64.85).abs() < 1e-6);

        // Tick 1: wall kick-off should start moving the minecart in +X (`mot_x = 0.02`), and snap Y to `64.5`
        world.tick_single_minecart(&mut cart);
        assert!(
            cart.mot_x > 0.0,
            "Powered rail next to wall should kick cart in +X"
        );
        assert!((cart.y - 64.5).abs() < 1e-6);

        // Tick until the cart rounds the corner at (4, 64, 4) and enters (4, 64, 5) moving in +Z
        let mut turned_south = false;
        for _ in 0..40 {
            world.tick_single_minecart(&mut cart);
            if cart.x.floor() as i32 == 4 && cart.z.floor() as i32 == 5 && cart.mot_z > 0.05 {
                turned_south = true;
                break;
            }
        }
        assert!(
            turned_south,
            "Minecart should smoothly turn 90 degrees onto +Z rail at (4, 64, 5); pos=({}, {}), mot=({}, {})",
            cart.x,
            cart.z,
            cart.mot_x,
            cart.mot_z
        );

        // Continue ticking onto the unpowered golden rail at (4, 64, 6) -> should brake to a stop!
        for _ in 0..30 {
            world.tick_single_minecart(&mut cart);
        }
        assert_eq!(cart.mot_x, 0.0);
        assert_eq!(cart.mot_z, 0.0);
    }

    #[test]
    fn minecart_detector_rail_stays_powered_while_occupied() {
        use crate::entity::projectile::Minecart;

        let mut world = make_flat_world();
        world.live_enable();
        world.java_set_type_id_and_data(5, 64, 5, blocks::DETECTOR_RAIL, 0);

        let mut cart = Minecart::new(1, 0, 5.5, 64.5, 5.5);

        // Tick for 35 ticks while the cart stays stationary on the detector rail
        for _ in 0..35 {
            world.active_detector_rails.clear();
            world.tick_single_minecart(&mut cart);
            world.live_tick_ex(false);
            assert_ne!(
                world.get_data(5, 64, 5) & 8,
                0,
                "Detector rail must remain powered while minecart sits on it"
            );
        }

        // Now move the minecart off the detector rail and advance 25 ticks -> detector rail must turn OFF
        world.active_detector_rails.clear();
        for _ in 0..25 {
            world.live_tick_ex(false);
        }
        assert_eq!(
            world.get_data(5, 64, 5) & 8,
            0,
            "Detector rail must turn off 20 ticks after minecart leaves"
        );
    }

    #[test]
    fn minecart_nbt_round_trip_and_furnace_traction() {
        use crate::entity::projectile::Minecart;

        let mut world = make_flat_world();
        for x in 2..=8 {
            world.java_set_type_id_and_data(x, 64, 5, blocks::RAIL, 1);
        }

        let mut powered = Minecart::new(10, 2, 3.5, 64.5, 5.5);
        powered.fuel = 1200;
        powered.push_x = 1.0;
        powered.push_z = 0.0;

        for _ in 0..10 {
            world.tick_single_minecart(&mut powered);
        }
        assert!(
            powered.x > 3.8 && powered.mot_x > 0.1,
            "Furnace minecart with fuel and push_x should propel itself along the track"
        );

        let nbt = powered.to_nbt();
        let restored = Minecart::from_nbt(10, &nbt).expect("valid Minecart NBT");
        assert_eq!(restored.cart_type, 2);
        assert_eq!(restored.fuel, powered.fuel);
        assert!((restored.x - powered.x).abs() < 1e-6);
    }

    #[test]
    fn custom_spawn_and_poseidon_uberbukkit_config_toggles() {
        let mut world = World::new(WorldConfig {
            seed: 42,
            save_enabled: false,
            spawn_x: 12,
            spawn_y: 65,
            spawn_z: -5,
            teleport_to_highest_safe_block: false,
            ..Default::default()
        });
        for cx in -1..=1 {
            for cz in -1..=1 {
                world.ensure_chunk_base(ChunkPos::new(cx, cz));
            }
        }
        // Place a solid ceiling at (12, 70, -5) above our indoor spawn at (12, 65, -5)
        world.set_block(12, 64, -5, blocks::STONE);
        world.set_block(12, 65, -5, blocks::AIR);
        world.set_block(12, 66, -5, blocks::AIR);
        world.set_block(12, 70, -5, blocks::STONE);

        // 1. With `teleport_to_highest_safe_block = false`, `safe_spawn_point` returns exact `(12, 65, -5)`
        assert_eq!(world.safe_spawn_point(), (12, 65, -5));

        // 2. Even with `teleport_to_highest_safe_block = true`, custom `spawn_y = 65` stays at 65 if (12, 65..66, -5) is air!
        let mut world2 = World::new(WorldConfig {
            seed: 42,
            save_enabled: false,
            spawn_x: 12,
            spawn_y: 65,
            spawn_z: -5,
            teleport_to_highest_safe_block: true,
            ..Default::default()
        });
        world2.ensure_chunk_base(ChunkPos::new(0, -1));
        world2.set_block(12, 64, -5, blocks::STONE);
        world2.set_block(12, 65, -5, blocks::AIR);
        world2.set_block(12, 66, -5, blocks::AIR);
        world2.set_block(12, 70, -5, blocks::STONE);
        assert_eq!(world2.safe_spawn_point(), (12, 65, -5));

        // 3. Test sponge water absorption when `sponge_absorption_enabled = true`
        world2.mechanics.sponge_absorption_enabled = true;
        world2.mechanics.sponge_radius = 2;
        world2.set_block(5, 65, 5, blocks::STATIONARY_WATER);
        world2.java_set_type_id(5, 65, 4, blocks::SPONGE);
        assert_eq!(
            world2.get_block(5, 65, 5),
            blocks::AIR,
            "Sponge should absorb nearby water when sponge_absorption_enabled is true"
        );
    }

    #[test]
    fn portal_travel_agent_finds_and_snaps_to_existing_portal() {
        let mut world = make_flat_world();

        // 4x5 obsidian frame in the x/y plane at z = 10, with a 2x3 portal.
        for x in 10..=13 {
            for y in 63..=67 {
                let edge = x == 10 || x == 13 || y == 63 || y == 67;
                world.set_block(
                    x,
                    y,
                    10,
                    if edge {
                        blocks::OBSIDIAN
                    } else {
                        blocks::PORTAL
                    },
                );
            }
        }

        let found = world.find_portal(12.5, 65.0, 10.5).expect("portal");
        // Java centers on the bottom of the nearest column and expands by
        // half a block towards adjacent portal columns along both axes.
        assert_eq!(found, (12.0, 64.5, 10.5));
    }

    #[test]
    fn portal_travel_agent_creates_frame_when_missing() {
        let mut world = make_flat_world();
        let pos = world.find_or_create_portal(5.0, 64.0, 5.0);

        let mut portals = 0;
        let mut obsidian = 0;
        for x in -16..=26 {
            for y in 60..=75 {
                for z in -16..=26 {
                    match world.get_block(x, y, z) {
                        blocks::PORTAL => portals += 1,
                        blocks::OBSIDIAN => obsidian += 1,
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(portals, 6, "2x3 portal interior");
        assert!(obsidian >= 14, "obsidian frame, got {obsidian}");
        assert!((60.5..=118.5).contains(&pos.1));
    }
}
