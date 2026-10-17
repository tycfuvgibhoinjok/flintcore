use std::collections::{HashMap, HashSet};

use crate::entity::player::Player;
use crate::net::ConnId;
use crate::world::chunk::ChunkPos;

#[derive(Debug, Default)]
pub struct EntityStore {
    players: HashMap<i32, Player>,
    conn_to_entity: HashMap<ConnId, i32>,
    chunk_index: HashMap<ChunkPos, HashSet<i32>>,
    next_id: i32,
}

impl EntityStore {
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
            conn_to_entity: HashMap::new(),
            chunk_index: HashMap::new(),
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
