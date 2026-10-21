//! The tags of the entities of the reference (`S-01`).
//!
//! Java `ChunkLoader.a(Chunk, World, NBTTagCompound)` writes the entities
//! of the chunk into the list `Entities`; `ChunkLoader.a(World,
//! NBTTagCompound)` reads them back.  Every entity is written by
//! `Entity.c(NBTTagCompound)`:
//!
//! - the name of the type (the table `EntityTypes`: `Item`, `Painting`,
//!   `Arrow`, `Snowball`, `PrimedTnt`, `Minecart`, `Boat`, `Mob` and the
//!   names of the mobs),
//! - the tags of `Entity.d(NBTTagCompound)`: `Pos`, `Motion`, `Rotation`,
//!   `FallDistance`, `Fire`, `Air`, `OnGround` and the UUID of the world
//!   and of the entity,
//! - the tags of the type itself (`Entity.b(NBTTagCompound)`).

use crate::gameplay::items::ItemStack;
use crate::world::storage::nbt::NbtTag;

/// Java `ItemStack.a(NBTTagCompound)`: the stack of an item.
pub fn item_to_nbt(stack: &ItemStack) -> NbtTag {
    let mut tag = NbtTag::compound();
    tag.insert("id", NbtTag::Short(stack.id));
    tag.insert("Count", NbtTag::Byte(stack.count as i8));
    tag.insert("Damage", NbtTag::Short(stack.damage));
    tag
}

/// Java `ItemStack.a(NBTTagCompound)`: the reading of a stack of an item.
pub fn item_from_nbt(tag: &NbtTag) -> Option<ItemStack> {
    let id = tag.get_i16("id")?;
    let count = tag.get_i8("Count").unwrap_or(1).max(1) as u8;
    let damage = tag.get_i16("Damage").unwrap_or(0);
    Some(ItemStack::new(id, count, damage))
}

/// Java `Entity.d(NBTTagCompound)`: the tags that every entity has.
#[derive(Debug, Clone, Copy)]
pub struct BaseEntity {
    /// The name of the type (`EntityTypes`).
    pub id: &'static str,
    /// `locX`, `locY + br` and `locZ`.
    pub pos: (f64, f64, f64),
    pub motion: (f64, f64, f64),
    /// The yaw and the pitch.
    pub rotation: (f32, f32),
    pub fall_distance: f32,
    pub fire_ticks: i32,
    pub air_ticks: i32,
    pub on_ground: bool,
}

impl BaseEntity {
    /// Writes the tags of `Entity.d(NBTTagCompound)`.
    pub fn write(&self, tag: &mut NbtTag, world_uuid: (i64, i64), entity_uuid: (i64, i64)) {
        tag.insert("id", NbtTag::String(self.id.to_string()));
        tag.insert(
            "Pos",
            NbtTag::List(vec![
                NbtTag::Double(self.pos.0),
                NbtTag::Double(self.pos.1),
                NbtTag::Double(self.pos.2),
            ]),
        );
        tag.insert(
            "Motion",
            NbtTag::List(vec![
                NbtTag::Double(self.motion.0),
                NbtTag::Double(self.motion.1),
                NbtTag::Double(self.motion.2),
            ]),
        );
        tag.insert(
            "Rotation",
            NbtTag::List(vec![
                NbtTag::Float(self.rotation.0),
                NbtTag::Float(self.rotation.1),
            ]),
        );
        tag.insert("FallDistance", NbtTag::Float(self.fall_distance));
        tag.insert("Fire", NbtTag::Short(self.fire_ticks as i16));
        tag.insert("Air", NbtTag::Short(self.air_ticks as i16));
        tag.insert("OnGround", NbtTag::Byte(i8::from(self.on_ground)));
        tag.insert("WorldUUIDLeast", NbtTag::Long(world_uuid.0));
        tag.insert("WorldUUIDMost", NbtTag::Long(world_uuid.1));
        tag.insert("UUIDLeast", NbtTag::Long(entity_uuid.0));
        tag.insert("UUIDMost", NbtTag::Long(entity_uuid.1));
    }

    /// Reads the tags of `Entity.e(NBTTagCompound)`.
    pub fn read(tag: &NbtTag, id: &'static str) -> Option<Self> {
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

        let mut motion = (0.0, 0.0, 0.0);
        if let Some(list) = tag.get_list("Motion") {
            if list.len() >= 3 {
                if let NbtTag::Double(v) = list[0] {
                    motion.0 = v;
                }
                if let NbtTag::Double(v) = list[1] {
                    motion.1 = v;
                }
                if let NbtTag::Double(v) = list[2] {
                    motion.2 = v;
                }
            }
        }

        let mut rotation = (0.0f32, 0.0f32);
        if let Some(list) = tag.get_list("Rotation") {
            if list.len() >= 2 {
                if let NbtTag::Float(v) = list[0] {
                    rotation.0 = v;
                }
                if let NbtTag::Float(v) = list[1] {
                    rotation.1 = v;
                }
            }
        }

        Some(Self {
            id,
            pos: (x, y, z),
            motion,
            rotation,
            fall_distance: tag.get_f32("FallDistance").unwrap_or(0.0),
            fire_ticks: tag.get_i16("Fire").unwrap_or(0) as i32,
            air_ticks: tag.get_i16("Air").unwrap_or(300) as i32,
            on_ground: tag.get_i8("OnGround").unwrap_or(0) != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tags_of_the_base_are_read_back() {
        let base = BaseEntity {
            id: "Item",
            pos: (1.0, 2.0, 3.0),
            motion: (0.1, 0.2, 0.3),
            rotation: (90.0, -10.0),
            fall_distance: 1.5,
            fire_ticks: 7,
            air_ticks: 300,
            on_ground: true,
        };
        let mut tag = NbtTag::compound();
        base.write(&mut tag, (11, 22), (33, 44));

        assert_eq!(tag.get_str("id"), Some("Item"));
        assert_eq!(tag.get_i64("WorldUUIDLeast"), Some(11));
        assert_eq!(tag.get_i64("WorldUUIDMost"), Some(22));
        assert_eq!(tag.get_i64("UUIDLeast"), Some(33));
        assert_eq!(tag.get_i64("UUIDMost"), Some(44));
        assert_eq!(tag.get_i16("Air"), Some(300));
        assert_eq!(tag.get_i8("OnGround"), Some(1));

        let restored = BaseEntity::read(&tag, "Item").unwrap();
        assert_eq!(restored.pos, (1.0, 2.0, 3.0));
        assert_eq!(restored.motion, (0.1, 0.2, 0.3));
        assert_eq!(restored.rotation, (90.0, -10.0));
        assert_eq!(restored.fall_distance, 1.5);
        assert_eq!(restored.fire_ticks, 7);
        assert!(restored.on_ground);
    }

    #[test]
    fn the_stack_of_an_item_is_read_back() {
        let stack = ItemStack::new(276, 1, 5);
        let tag = item_to_nbt(&stack);
        assert_eq!(tag.get_i16("id"), Some(276));
        assert_eq!(tag.get_i8("Count"), Some(1));
        assert_eq!(tag.get_i16("Damage"), Some(5));
        assert_eq!(item_from_nbt(&tag), Some(stack));
    }
}
