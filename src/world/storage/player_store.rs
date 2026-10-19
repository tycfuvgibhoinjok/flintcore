use std::fs;
use std::path::{Path, PathBuf};

use crate::entity::player::Player;
use crate::gameplay::items::ItemStack;
use crate::util::Result;
use crate::world::storage::nbt::NbtTag;

pub struct PlayerStore {
    players_dir: PathBuf,
}

impl PlayerStore {
    pub fn new(world_dir: &Path) -> Self {
        Self {
            players_dir: world_dir.join("players"),
        }
    }

    pub fn player_file(&self, username: &str) -> PathBuf {
        self.players_dir.join(format!("{username}.dat"))
    }

    pub fn load_player(&self, player: &mut Player) -> Result<bool> {
        let file = self.player_file(&player.username);
        if !file.exists() {
            return Ok(false);
        }

        let bytes = fs::read(&file)?;
        let (_name, root) = NbtTag::from_gzip(&bytes)?;

        if let Some(pos) = root.get_list("Pos") {
            if pos.len() == 3 {
                if let (NbtTag::Double(x), NbtTag::Double(y), NbtTag::Double(z)) =
                    (&pos[0], &pos[1], &pos[2])
                {
                    player.x = *x;
                    player.y = *y;
                    player.z = *z;
                }
            }
        }

        if let Some(rot) = root.get_list("Rotation") {
            if rot.len() == 2 {
                if let (NbtTag::Float(yaw), NbtTag::Float(pitch)) = (&rot[0], &rot[1]) {
                    player.yaw = *yaw;
                    player.pitch = *pitch;
                }
            }
        }

        if let Some(dimension) = root.get_i32("Dimension") {
            player.dimension = dimension;
        }

        if let Some(health) = root.get_i16("Health") {
            player.health = health.clamp(0, 20);
        }

        if let Some(air) = root.get_i16("Air") {
            player.air_ticks = air;
        }

        if let Some(fire) = root.get_i16("Fire") {
            player.fire_ticks = fire;
        }

        if let (Some(sx), Some(sy), Some(sz)) = (
            root.get_i32("SpawnX"),
            root.get_i32("SpawnY"),
            root.get_i32("SpawnZ"),
        ) {
            player.spawn_bed = Some((sx, sy, sz));
        }

        if let Some(on_ground) = root.get_i8("OnGround") {
            player.on_ground = on_ground != 0;
        }

        if let Some(inventory_list) = root.get_list("Inventory") {
            for item_tag in inventory_list {
                if let (Some(slot), Some(id), Some(count), Some(damage)) = (
                    item_tag.get_i8("Slot"),
                    item_tag.get_i16("id"),
                    item_tag.get_i8("Count"),
                    item_tag.get_i16("Damage"),
                ) {
                    let stack = ItemStack::new(id, count.max(1) as u8, damage);
                    let slot = slot as usize;
                    if slot < 36 {
                        player.inventory.items[slot] = Some(stack);
                    } else if (100..=103).contains(&slot) {
                        player.inventory.armor[slot - 100] = Some(stack);
                    }
                }
            }
        }

        Ok(true)
    }

    pub fn save_player(&self, player: &Player) -> Result<()> {
        fs::create_dir_all(&self.players_dir)?;

        let mut root = NbtTag::compound();
        root.insert(
            "Pos",
            NbtTag::List(vec![
                NbtTag::Double(player.x),
                NbtTag::Double(player.y),
                NbtTag::Double(player.z),
            ]),
        );
        root.insert(
            "Motion",
            NbtTag::List(vec![
                NbtTag::Double(0.0),
                NbtTag::Double(0.0),
                NbtTag::Double(0.0),
            ]),
        );
        root.insert(
            "Rotation",
            NbtTag::List(vec![NbtTag::Float(player.yaw), NbtTag::Float(player.pitch)]),
        );
        root.insert("FallDistance", NbtTag::Float(player.fall_distance));
        root.insert("Fire", NbtTag::Short(player.fire_ticks));
        root.insert("Air", NbtTag::Short(player.air_ticks));
        root.insert(
            "OnGround",
            NbtTag::Byte(if player.on_ground { 1 } else { 0 }),
        );
        root.insert("Dimension", NbtTag::Int(player.dimension));
        root.insert("Health", NbtTag::Short(player.health));
        if let Some((sx, sy, sz)) = player.spawn_bed {
            root.insert("SpawnX", NbtTag::Int(sx));
            root.insert("SpawnY", NbtTag::Int(sy));
            root.insert("SpawnZ", NbtTag::Int(sz));
        }

        let mut inventory_list = Vec::new();
        for slot in 0..36 {
            if let Some(stack) = player.inventory.items[slot] {
                let mut item = NbtTag::compound();
                item.insert("Slot", NbtTag::Byte(slot as i8));
                item.insert("id", NbtTag::Short(stack.id));
                item.insert("Count", NbtTag::Byte(stack.count as i8));
                item.insert("Damage", NbtTag::Short(stack.damage));
                inventory_list.push(item);
            }
        }

        for armor_idx in 0..4 {
            if let Some(stack) = player.inventory.armor[armor_idx] {
                let mut item = NbtTag::compound();
                item.insert("Slot", NbtTag::Byte((100 + armor_idx) as i8));
                item.insert("id", NbtTag::Short(stack.id));
                item.insert("Count", NbtTag::Byte(stack.count as i8));
                item.insert("Damage", NbtTag::Short(stack.damage));
                inventory_list.push(item);
            }
        }
        root.insert("Inventory", NbtTag::List(inventory_list));

        let bytes = root.to_gzip("")?;
        let final_file = self.player_file(&player.username);
        let tmp_file = self
            .players_dir
            .join(format!("_tmp_{}.dat", player.username));

        fs::write(&tmp_file, &bytes)?;
        if final_file.exists() {
            let _ = fs::remove_file(&final_file);
        }
        fs::rename(&tmp_file, &final_file)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_store_round_trip() {
        let temp_dir =
            std::env::temp_dir().join(format!("flintcore_player_test_{}", std::process::id()));
        let store = PlayerStore::new(&temp_dir);

        let mut player = Player::new(1, 1, "Steve".into(), 0.0, 64.0, 0.0);
        player.x = 123.5;
        player.y = 65.0;
        player.z = -456.75;
        player.yaw = 180.0;
        player.pitch = -15.0;
        player.health = 18;
        player.dimension = -1;
        player.inventory.items[0] = Some(ItemStack::new(276, 1, 10)); // Diamond sword
        player.inventory.armor[0] = Some(ItemStack::new(313, 1, 5)); // Diamond boots

        store.save_player(&player).unwrap();
        assert!(store.player_file("Steve").exists());

        let mut loaded = Player::new(2, 2, "Steve".into(), 0.0, 64.0, 0.0);
        let found = store.load_player(&mut loaded).unwrap();
        assert!(found);
        assert_eq!(loaded.x, 123.5);
        assert_eq!(loaded.y, 65.0);
        assert_eq!(loaded.z, -456.75);
        assert_eq!(loaded.yaw, 180.0);
        assert_eq!(loaded.pitch, -15.0);
        assert_eq!(loaded.health, 18);
        assert_eq!(loaded.dimension, -1);
        assert_eq!(loaded.inventory.items[0], Some(ItemStack::new(276, 1, 10)));
        assert_eq!(loaded.inventory.armor[0], Some(ItemStack::new(313, 1, 5)));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
