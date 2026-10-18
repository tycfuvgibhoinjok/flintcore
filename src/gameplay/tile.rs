use crate::gameplay::items::ItemStack;
use crate::world::storage::nbt::NbtTag;

#[derive(Debug, Clone, PartialEq)]
pub enum TileEntityKind {
    Chest {
        items: Box<[Option<ItemStack>; 27]>,
    },
    Furnace {
        burn_time: i16,
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
                TileEntityKind::Furnace {
                    burn_time,
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
            _ => return None,
        };

        Some(Self { x, y, z, kind })
    }
}
