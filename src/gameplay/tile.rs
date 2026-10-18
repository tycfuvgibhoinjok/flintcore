use crate::gameplay::crafting::{furnace_fuel_time, furnace_smelt_result};
use crate::gameplay::items::ItemStack;
use crate::world::storage::nbt::NbtTag;

#[derive(Debug, Clone, PartialEq)]
pub enum TileEntityKind {
    Chest {
        items: Box<[Option<ItemStack>; 27]>,
    },
    Furnace {
        burn_time: i16,
        ticks_for_current_fuel: i16,
        cook_time: i16,
        items: Box<[Option<ItemStack>; 3]>,
    },
    Sign {
        lines: [String; 4],
    },
    MobSpawner {
        entity_id: String,
        delay: i16,
    },
    Dispenser {
        items: Box<[Option<ItemStack>; 9]>,
    },
    Note {
        note: u8,
    },
    RecordPlayer {
        record: i32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TileEntity {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub kind: TileEntityKind,
}

impl TileEntity {
    pub fn new_chest(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::Chest {
                items: Box::new([None; 27]),
            },
        }
    }

    pub fn new_furnace(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::Furnace {
                burn_time: 0,
                ticks_for_current_fuel: 0,
                cook_time: 0,
                items: Box::new([None; 3]),
            },
        }
    }

    pub fn new_dispenser(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::Dispenser {
                items: Box::new([None; 9]),
            },
        }
    }

    pub fn new_note(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::Note { note: 0 },
        }
    }

    pub fn new_record_player(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::RecordPlayer { record: 0 },
        }
    }

    pub fn new_sign(x: i32, y: i32, z: i32, lines: [String; 4]) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::Sign { lines },
        }
    }

    pub fn new_spawner(x: i32, y: i32, z: i32, entity_id: String) -> Self {
        Self {
            x,
            y,
            z,
            kind: TileEntityKind::MobSpawner {
                entity_id,
                delay: 20,
            },
        }
    }

    /// Tick a furnace tile entity (`TileEntityFurnace.g_()`).
    /// Returns `(dirty, Some(is_burning_now))` when the furnace block ID needs to switch between 61 and 62.
    pub fn tick_furnace(&mut self) -> (bool, Option<bool>) {
        let TileEntityKind::Furnace {
            burn_time,
            ticks_for_current_fuel,
            cook_time,
            items,
        } = &mut self.kind
        else {
            return (false, None);
        };

        let was_burning = *burn_time > 0;
        let mut dirty = false;

        let can_burn = Self::furnace_can_burn(items);
        if *burn_time > 0 && can_burn {
            *cook_time += 1;
            if *cook_time >= 200 {
                *cook_time = 0;
                Self::furnace_smelt_one(items);
                dirty = true;
            }
        } else {
            *cook_time = 0;
        }

        if *burn_time > 0 {
            *burn_time -= 1;
        }

        let can_burn_now = Self::furnace_can_burn(items);
        if *burn_time <= 0 && can_burn_now && items[1].is_some() {
            let fuel = furnace_fuel_time(items[1]);
            *ticks_for_current_fuel = fuel;
            *burn_time += fuel;
            if *burn_time > 0 {
                dirty = true;
                if let Some(mut fuel_stack) = items[1] {
                    fuel_stack.count = fuel_stack.count.saturating_sub(1);
                    items[1] = if fuel_stack.count > 0 {
                        Some(fuel_stack)
                    } else {
                        None
                    };
                }
            }
        }

        let is_burning = *burn_time > 0;
        let state_change = if was_burning != is_burning {
            dirty = true;
            Some(is_burning)
        } else {
            None
        };

        (dirty, state_change)
    }

    fn furnace_can_burn(items: &[Option<ItemStack>; 3]) -> bool {
        let Some(input) = items[0] else {
            return false;
        };
        let Some(result) = furnace_smelt_result(input.id) else {
            return false;
        };
        let Some(output) = items[2] else {
            return true;
        };
        if output.id != result.id || output.damage != result.damage {
            return false;
        }
        let max_stack = output.max_stack_size().min(result.max_stack_size());
        output.count + result.count <= max_stack
    }

    fn furnace_smelt_one(items: &mut [Option<ItemStack>; 3]) {
        if !Self::furnace_can_burn(items) {
            return;
        }
        let input = items[0].unwrap();
        let result = furnace_smelt_result(input.id).unwrap();

        match &mut items[2] {
            None => items[2] = Some(result),
            Some(out) if out.id == result.id && out.damage == result.damage => {
                out.count = out.count.saturating_add(result.count);
            }
            _ => {}
        }

        if let Some(mut inp) = items[0] {
            inp.count = inp.count.saturating_sub(1);
            items[0] = if inp.count > 0 { Some(inp) } else { None };
        }
    }

    pub fn to_nbt(&self) -> NbtTag {
        let mut root = NbtTag::compound();
        root.insert("x", NbtTag::Int(self.x));
        root.insert("y", NbtTag::Int(self.y));
        root.insert("z", NbtTag::Int(self.z));

        match &self.kind {
            TileEntityKind::Chest { items } => {
                root.insert("id", NbtTag::String("Chest".into()));
                let mut list = Vec::new();
                for (slot, stack) in items.iter().enumerate() {
                    if let Some(s) = stack {
                        let mut item_tag = NbtTag::compound();
                        item_tag.insert("Slot", NbtTag::Byte(slot as i8));
                        item_tag.insert("id", NbtTag::Short(s.id));
                        item_tag.insert("Count", NbtTag::Byte(s.count as i8));
                        item_tag.insert("Damage", NbtTag::Short(s.damage));
                        list.push(item_tag);
                    }
                }
                root.insert("Items", NbtTag::List(list));
            }
            TileEntityKind::Furnace {
                burn_time,
                cook_time,
                items,
                ..
            } => {
                root.insert("id", NbtTag::String("Furnace".into()));
                root.insert("BurnTime", NbtTag::Short(*burn_time));
                root.insert("CookTime", NbtTag::Short(*cook_time));
                let mut list = Vec::new();
                for (slot, stack) in items.iter().enumerate() {
                    if let Some(s) = stack {
                        let mut item_tag = NbtTag::compound();
                        item_tag.insert("Slot", NbtTag::Byte(slot as i8));
                        item_tag.insert("id", NbtTag::Short(s.id));
                        item_tag.insert("Count", NbtTag::Byte(s.count as i8));
                        item_tag.insert("Damage", NbtTag::Short(s.damage));
                        list.push(item_tag);
                    }
                }
                root.insert("Items", NbtTag::List(list));
            }
            TileEntityKind::Sign { lines } => {
                root.insert("id", NbtTag::String("Sign".into()));
                root.insert("Text1", NbtTag::String(lines[0].clone()));
                root.insert("Text2", NbtTag::String(lines[1].clone()));
                root.insert("Text3", NbtTag::String(lines[2].clone()));
                root.insert("Text4", NbtTag::String(lines[3].clone()));
            }
            TileEntityKind::MobSpawner { entity_id, delay } => {
                root.insert("id", NbtTag::String("MobSpawner".into()));
                root.insert("EntityId", NbtTag::String(entity_id.clone()));
                root.insert("Delay", NbtTag::Short(*delay));
            }
            TileEntityKind::Dispenser { items } => {
                root.insert("id", NbtTag::String("Trap".into()));
                let mut list = Vec::new();
                for (slot, stack) in items.iter().enumerate() {
                    if let Some(s) = stack {
                        let mut item_tag = NbtTag::compound();
                        item_tag.insert("Slot", NbtTag::Byte(slot as i8));
                        item_tag.insert("id", NbtTag::Short(s.id));
                        item_tag.insert("Count", NbtTag::Byte(s.count as i8));
                        item_tag.insert("Damage", NbtTag::Short(s.damage));
                        list.push(item_tag);
                    }
                }
                root.insert("Items", NbtTag::List(list));
            }
            TileEntityKind::Note { note } => {
                root.insert("id", NbtTag::String("Music".into()));
                root.insert("note", NbtTag::Byte(*note as i8));
            }
            TileEntityKind::RecordPlayer { record } => {
                root.insert("id", NbtTag::String("RecordPlayer".into()));
                if *record > 0 {
                    root.insert("Record", NbtTag::Int(*record));
                }
            }
        }

        root
    }

    pub fn from_nbt(tag: &NbtTag) -> Option<Self> {
        let x = tag.get_i32("x")?;
        let y = tag.get_i32("y")?;
        let z = tag.get_i32("z")?;
        let id = tag.get_str("id")?;

        let kind = match id {
            "Chest" => {
                let mut items = Box::new([None; 27]);
                if let Some(list) = tag.get_list("Items") {
                    for it in list {
                        if let (Some(slot), Some(item_id), Some(count), Some(damage)) = (
                            it.get_i8("Slot"),
                            it.get_i16("id"),
                            it.get_i8("Count"),
                            it.get_i16("Damage"),
                        ) {
                            let slot = slot as usize;
                            if slot < 27 {
                                items[slot] =
                                    Some(ItemStack::new(item_id, count.max(1) as u8, damage));
                            }
                        }
                    }
                }
                TileEntityKind::Chest { items }
            }
            "Furnace" => {
                let burn_time = tag.get_i16("BurnTime").unwrap_or(0);
                let cook_time = tag.get_i16("CookTime").unwrap_or(0);
                let mut items = Box::new([None; 3]);
                if let Some(list) = tag.get_list("Items") {
                    for it in list {
                        if let (Some(slot), Some(item_id), Some(count), Some(damage)) = (
                            it.get_i8("Slot"),
                            it.get_i16("id"),
                            it.get_i8("Count"),
                            it.get_i16("Damage"),
                        ) {
                            let slot = slot as usize;
                            if slot < 3 {
                                items[slot] =
                                    Some(ItemStack::new(item_id, count.max(1) as u8, damage));
                            }
                        }
                    }
                }
                let ticks_for_current_fuel = furnace_fuel_time(items[1]);
                TileEntityKind::Furnace {
                    burn_time,
                    ticks_for_current_fuel,
                    cook_time,
                    items,
                }
            }
            "Sign" => {
                let lines = [
                    tag.get_str("Text1").unwrap_or("").to_string(),
                    tag.get_str("Text2").unwrap_or("").to_string(),
                    tag.get_str("Text3").unwrap_or("").to_string(),
                    tag.get_str("Text4").unwrap_or("").to_string(),
                ];
                TileEntityKind::Sign { lines }
            }
            "MobSpawner" => {
                let entity_id = tag.get_str("EntityId").unwrap_or("Pig").to_string();
                let delay = tag.get_i16("Delay").unwrap_or(20);
                TileEntityKind::MobSpawner { entity_id, delay }
            }
            "Trap" => {
                let mut items = Box::new([None; 9]);
                if let Some(list) = tag.get_list("Items") {
                    for it in list {
                        if let (Some(slot), Some(item_id), Some(count), Some(damage)) = (
                            it.get_i8("Slot"),
                            it.get_i16("id"),
                            it.get_i8("Count"),
                            it.get_i16("Damage"),
                        ) {
                            let slot = slot as usize;
                            if slot < 9 {
                                items[slot] =
                                    Some(ItemStack::new(item_id, count.max(1) as u8, damage));
                            }
                        }
                    }
                }
                TileEntityKind::Dispenser { items }
            }
            "Music" => {
                let note = tag.get_i8("note").unwrap_or(0) as u8;
                TileEntityKind::Note { note }
            }
            "RecordPlayer" => {
                let record = tag.get_i32("Record").unwrap_or(0);
                TileEntityKind::RecordPlayer { record }
            }
            _ => return None,
        };

        Some(Self { x, y, z, kind })
    }
}
