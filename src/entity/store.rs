use std::collections::{HashMap, HashSet};

use crate::entity::mob::{Mob, MobType};
use crate::entity::player::Player;
use crate::entity::projectile::{Minecart, Painting, PrimedTnt, Projectile, ProjectileKind};
use crate::gameplay::items::ItemStack;
use crate::net::ConnId;
use crate::world::chunk::ChunkPos;

#[derive(Debug, Clone)]
pub struct DroppedItem {
    pub entity_id: i32,
    pub item: ItemStack,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub on_ground: bool,
    pub age: i32,
    pub pickup_delay: i32,
    pub health: i16,
    pub dead: bool,
}

impl DroppedItem {
    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x as i32) >> 4, (self.z as i32) >> 4)
    }
}

#[derive(Debug, Clone)]
pub struct Boat {
    pub entity_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub prev_x: f64,
    pub prev_y: f64,
    pub prev_z: f64,
    pub mot_x: f64,
    pub mot_y: f64,
    pub mot_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub damage: i32,
    pub time_since_hit: i32,
    pub rock_direction: i32,
    pub passenger_id: Option<i32>,
    pub on_ground: bool,
    pub dead: bool,
}

impl Boat {
    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::new((self.x as i32) >> 4, (self.z as i32) >> 4)
    }

    /// Java `EntityBoat.m()`: passenger vertical offset `-0.3`.
    pub fn mounted_y_offset(&self) -> f64 {
        -0.30000001192092896
    }
}

#[derive(Debug, Default)]
pub struct EntityStore {
    players: HashMap<i32, Player>,
    conn_to_entity: HashMap<ConnId, i32>,
    chunk_index: HashMap<ChunkPos, HashSet<i32>>,
    dropped_items: HashMap<i32, DroppedItem>,
    boats: HashMap<i32, Boat>,
    mobs: HashMap<i32, Mob>,
    projectiles: HashMap<i32, Projectile>,
    primed_tnts: HashMap<i32, PrimedTnt>,
    minecarts: HashMap<i32, Minecart>,
    paintings: HashMap<i32, Painting>,
    next_id: i32,
}

impl EntityStore {
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
            conn_to_entity: HashMap::new(),
            chunk_index: HashMap::new(),
            dropped_items: HashMap::new(),
            boats: HashMap::new(),
            mobs: HashMap::new(),
            projectiles: HashMap::new(),
            primed_tnts: HashMap::new(),
            minecarts: HashMap::new(),
            paintings: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn allocate_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    pub fn insert_player(
        &mut self,
        conn_id: ConnId,
        username: String,
        x: f64,
        y: f64,
        z: f64,
    ) -> i32 {
        let entity_id = self.allocate_id();
        let player = Player::new(entity_id, conn_id, username, x, y, z);
        let chunk = player.chunk_pos();

        self.players.insert(entity_id, player);
        self.conn_to_entity.insert(conn_id, entity_id);
        self.chunk_index.entry(chunk).or_default().insert(entity_id);

        entity_id
    }

    pub fn remove_player(&mut self, entity_id: i32) -> Option<Player> {
        if let Some(player) = self.players.remove(&entity_id) {
            self.conn_to_entity.remove(&player.conn_id);
            let chunk = player.chunk_pos();
            if let Some(set) = self.chunk_index.get_mut(&chunk) {
                set.remove(&entity_id);
                if set.is_empty() {
                    self.chunk_index.remove(&chunk);
                }
            }
            Some(player)
        } else {
            None
        }
    }

    pub fn remove_by_conn(&mut self, conn_id: ConnId) -> Option<Player> {
        if let Some(&entity_id) = self.conn_to_entity.get(&conn_id) {
            self.remove_player(entity_id)
        } else {
            None
        }
    }

    pub fn get_player(&self, entity_id: i32) -> Option<&Player> {
        self.players.get(&entity_id)
    }

    pub fn get_player_mut(&mut self, entity_id: i32) -> Option<&mut Player> {
        self.players.get_mut(&entity_id)
    }

    pub fn get_by_conn(&self, conn_id: ConnId) -> Option<&Player> {
        let entity_id = *self.conn_to_entity.get(&conn_id)?;
        self.get_player(entity_id)
    }

    pub fn get_by_conn_mut(&mut self, conn_id: ConnId) -> Option<&mut Player> {
        let entity_id = *self.conn_to_entity.get(&conn_id)?;
        self.get_player_mut(entity_id)
    }

    pub fn update_player_chunk(
        &mut self,
        entity_id: i32,
        old_chunk: ChunkPos,
        new_chunk: ChunkPos,
    ) {
        if old_chunk != new_chunk {
            if let Some(set) = self.chunk_index.get_mut(&old_chunk) {
                set.remove(&entity_id);
                if set.is_empty() {
                    self.chunk_index.remove(&old_chunk);
                }
            }
            self.chunk_index
                .entry(new_chunk)
                .or_default()
                .insert(entity_id);
        }
    }

    pub fn iter_players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    pub fn iter_players_mut(&mut self) -> impl Iterator<Item = &mut Player> {
        self.players.values_mut()
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    pub fn players_in_range(&self, center_x: f64, center_z: f64, radius: f64) -> Vec<i32> {
        let mut result = Vec::new();
        let r_sq = radius * radius;
        for player in self.players.values() {
            let dx = player.x - center_x;
            let dz = player.z - center_z;
            if dx * dx + dz * dz <= r_sq {
                result.push(player.entity_id);
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_dropped_item(
        &mut self,
        item: ItemStack,
        x: f64,
        y: f64,
        z: f64,
        mot_x: f64,
        mot_y: f64,
        mot_z: f64,
        pickup_delay: i32,
    ) -> i32 {
        let entity_id = self.allocate_id();
        let dropped = DroppedItem {
            entity_id,
            item,
            x,
            y,
            z,
            mot_x,
            mot_y,
            mot_z,
            on_ground: false,
            age: 0,
            pickup_delay,
            health: 5,
            dead: false,
        };
        self.dropped_items.insert(entity_id, dropped);
        entity_id
    }

    pub fn get_dropped_item(&self, entity_id: i32) -> Option<&DroppedItem> {
        self.dropped_items.get(&entity_id)
    }

    pub fn get_dropped_item_mut(&mut self, entity_id: i32) -> Option<&mut DroppedItem> {
        self.dropped_items.get_mut(&entity_id)
    }

    pub fn remove_dropped_item(&mut self, entity_id: i32) -> Option<DroppedItem> {
        self.dropped_items.remove(&entity_id)
    }

    pub fn iter_dropped_items(&self) -> impl Iterator<Item = &DroppedItem> {
        self.dropped_items.values()
    }

    pub fn iter_dropped_items_mut(&mut self) -> impl Iterator<Item = &mut DroppedItem> {
        self.dropped_items.values_mut()
    }

    pub fn remove_dropped_items_in_chunk(&mut self, pos: ChunkPos) -> Vec<i32> {
        let mut removed = Vec::new();
        self.dropped_items.retain(|&id, item| {
            if item.chunk_pos() == pos {
                removed.push(id);
                false
            } else {
                true
            }
        });
        removed
    }

    /// Spawn a new `EntityBoat` at `(x, y, z)`.
    pub fn spawn_boat(&mut self, x: f64, y: f64, z: f64) -> i32 {
        let entity_id = self.allocate_id();
        let boat = Boat {
            entity_id,
            x,
            y,
            z,
            prev_x: x,
            prev_y: y,
            prev_z: z,
            mot_x: 0.0,
            mot_y: 0.0,
            mot_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            damage: 0,
            time_since_hit: 0,
            rock_direction: 1,
            passenger_id: None,
            on_ground: false,
            dead: false,
        };
        self.boats.insert(entity_id, boat);
        entity_id
    }

    pub fn get_boat(&self, entity_id: i32) -> Option<&Boat> {
        self.boats.get(&entity_id)
    }

    pub fn get_boat_mut(&mut self, entity_id: i32) -> Option<&mut Boat> {
        self.boats.get_mut(&entity_id)
    }

    pub fn remove_boat(&mut self, entity_id: i32) -> Option<Boat> {
        self.boats.remove(&entity_id)
    }

    pub fn iter_boats(&self) -> impl Iterator<Item = &Boat> {
        self.boats.values()
    }

    pub fn iter_boats_mut(&mut self) -> impl Iterator<Item = &mut Boat> {
        self.boats.values_mut()
    }

    // ---------------------------------------------------------------------
    // Mobs
    // ---------------------------------------------------------------------

    pub fn spawn_mob(&mut self, mob_type: MobType, x: f64, y: f64, z: f64, seed: i64) -> i32 {
        let entity_id = self.allocate_id();
        let mob = Mob::new(entity_id, mob_type, x, y, z, seed);
        self.mobs.insert(entity_id, mob);
        entity_id
    }

    pub fn insert_mob(&mut self, mob: Mob) -> i32 {
        let id = mob.entity_id;
        self.mobs.insert(id, mob);
        id
    }

    pub fn get_mob(&self, entity_id: i32) -> Option<&Mob> {
        self.mobs.get(&entity_id)
    }

    pub fn get_mob_mut(&mut self, entity_id: i32) -> Option<&mut Mob> {
        self.mobs.get_mut(&entity_id)
    }

    pub fn remove_mob(&mut self, entity_id: i32) -> Option<Mob> {
        self.mobs.remove(&entity_id)
    }

    pub fn iter_mobs(&self) -> impl Iterator<Item = &Mob> {
        self.mobs.values()
    }

    pub fn iter_mobs_mut(&mut self) -> impl Iterator<Item = &mut Mob> {
        self.mobs.values_mut()
    }

    pub fn count_mob_categories(&self) -> (usize, usize, usize) {
        let mut monsters = 0;
        let mut animals = 0;
        let mut water = 0;
        for m in self.mobs.values() {
            if m.dead {
                continue;
            }
            if m.mob_type.is_monster() {
                monsters += 1;
            } else if m.mob_type.is_animal() {
                animals += 1;
            } else if m.mob_type.is_water_animal() {
                water += 1;
            }
        }
        (monsters, animals, water)
    }

    // ---------------------------------------------------------------------
    // Projectiles, PrimedTnt, Minecarts, Paintings
    // ---------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_projectile(
        &mut self,
        kind: ProjectileKind,
        shooter_id: Option<i32>,
        shooter_is_skeleton: bool,
        from_player: bool,
        x: f64,
        y: f64,
        z: f64,
        mot_x: f64,
        mot_y: f64,
        mot_z: f64,
    ) -> i32 {
        let entity_id = self.allocate_id();
        let proj = Projectile {
            entity_id,
            kind,
            shooter_id,
            shooter_is_skeleton,
            x,
            y,
            z,
            mot_x,
            mot_y,
            mot_z,
            accel_x: mot_x * 0.1,
            accel_y: mot_y * 0.1,
            accel_z: mot_z * 0.1,
            yaw: 0.0,
            pitch: 0.0,
            in_ground: false,
            ticks_in_ground: 0,
            ticks_in_air: 0,
            catch_ticks: 0,
            hooked_entity: None,
            from_player,
            dead: false,
        };
        self.projectiles.insert(entity_id, proj);
        entity_id
    }

    pub fn get_projectile(&self, entity_id: i32) -> Option<&Projectile> {
        self.projectiles.get(&entity_id)
    }

    pub fn get_projectile_mut(&mut self, entity_id: i32) -> Option<&mut Projectile> {
        self.projectiles.get_mut(&entity_id)
    }

    pub fn remove_projectile(&mut self, entity_id: i32) -> Option<Projectile> {
        self.projectiles.remove(&entity_id)
    }

    pub fn iter_projectiles(&self) -> impl Iterator<Item = &Projectile> {
        self.projectiles.values()
    }

    pub fn iter_projectiles_mut(&mut self) -> impl Iterator<Item = &mut Projectile> {
        self.projectiles.values_mut()
    }

    pub fn spawn_primed_tnt(&mut self, x: f64, y: f64, z: f64, fuse: i32) -> i32 {
        let entity_id = self.allocate_id();
        let angle = (entity_id as f64) * 1.1;
        let tnt = PrimedTnt {
            entity_id,
            x,
            y,
            z,
            mot_x: -angle.sin() * 0.02,
            mot_y: 0.2,
            mot_z: -angle.cos() * 0.02,
            fuse,
            on_ground: false,
            dead: false,
        };
        self.primed_tnts.insert(entity_id, tnt);
        entity_id
    }

    pub fn get_primed_tnt(&self, entity_id: i32) -> Option<&PrimedTnt> {
        self.primed_tnts.get(&entity_id)
    }

    pub fn get_primed_tnt_mut(&mut self, entity_id: i32) -> Option<&mut PrimedTnt> {
        self.primed_tnts.get_mut(&entity_id)
    }

    pub fn remove_primed_tnt(&mut self, entity_id: i32) -> Option<PrimedTnt> {
        self.primed_tnts.remove(&entity_id)
    }

    pub fn iter_primed_tnts(&self) -> impl Iterator<Item = &PrimedTnt> {
        self.primed_tnts.values()
    }

    pub fn iter_primed_tnts_mut(&mut self) -> impl Iterator<Item = &mut PrimedTnt> {
        self.primed_tnts.values_mut()
    }

    pub fn spawn_minecart(&mut self, cart_type: u8, x: f64, y: f64, z: f64) -> i32 {
        let entity_id = self.allocate_id();
        let cart = Minecart::new(entity_id, cart_type, x, y, z);
        self.minecarts.insert(entity_id, cart);
        entity_id
    }

    pub fn insert_minecart(&mut self, cart: Minecart) -> i32 {
        let id = cart.entity_id;
        self.minecarts.insert(id, cart);
        id
    }

    pub fn get_minecart(&self, entity_id: i32) -> Option<&Minecart> {
        self.minecarts.get(&entity_id)
    }

    pub fn get_minecart_mut(&mut self, entity_id: i32) -> Option<&mut Minecart> {
        self.minecarts.get_mut(&entity_id)
    }

    pub fn remove_minecart(&mut self, entity_id: i32) -> Option<Minecart> {
        self.minecarts.remove(&entity_id)
    }

    pub fn iter_minecarts(&self) -> impl Iterator<Item = &Minecart> {
        self.minecarts.values()
    }

    pub fn iter_minecarts_mut(&mut self) -> impl Iterator<Item = &mut Minecart> {
        self.minecarts.values_mut()
    }

    pub fn spawn_painting(&mut self, title: String, x: i32, y: i32, z: i32, direction: i32) -> i32 {
        let entity_id = self.allocate_id();
        let p = Painting {
            entity_id,
            title,
            x,
            y,
            z,
            direction,
            dead: false,
        };
        self.paintings.insert(entity_id, p);
        entity_id
    }

    pub fn get_painting(&self, entity_id: i32) -> Option<&Painting> {
        self.paintings.get(&entity_id)
    }

    pub fn remove_painting(&mut self, entity_id: i32) -> Option<Painting> {
        self.paintings.remove(&entity_id)
    }

    pub fn iter_paintings(&self) -> impl Iterator<Item = &Painting> {
        self.paintings.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_insert_and_retrieve() {
        let mut store = EntityStore::new();
        let id = store.insert_player(1, "Hero".to_string(), 10.0, 64.0, 20.0);
        assert_eq!(store.player_count(), 1);
        let p = store.get_player(id).unwrap();
        assert_eq!(p.username, "Hero");
        assert_eq!(store.get_by_conn(1).unwrap().entity_id, id);
    }

    #[test]
    fn store_remove() {
        let mut store = EntityStore::new();
        let id = store.insert_player(1, "Hero".to_string(), 10.0, 64.0, 20.0);
        let removed = store.remove_by_conn(1);
        assert!(removed.is_some());
        assert_eq!(store.player_count(), 0);
        assert!(store.get_player(id).is_none());
    }
}
