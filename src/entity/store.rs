use std::collections::{HashMap, HashSet};

use crate::entity::mob::{Mob, MobType};
use crate::entity::player::Player;
use crate::entity::projectile::{Minecart, Painting, PrimedTnt, Projectile, ProjectileKind};
use crate::gameplay::items::ItemStack;
use crate::net::ConnId;
use crate::util::rng::JavaRandom;
use crate::world::chunk::ChunkPos;
use crate::world::storage::nbt::NbtTag;

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

    /// Java `EntityItem.b(NBTTagCompound)`: the tags of the dropped item.
    pub fn to_nbt(&self, world_uuid: (i64, i64), entity_uuid: (i64, i64)) -> NbtTag {
        let base = crate::entity::save::BaseEntity {
            id: "Item",
            pos: (self.x, self.y, self.z),
            motion: (self.mot_x, self.mot_y, self.mot_z),
            rotation: (0.0, 0.0),
            fall_distance: 0.0,
            fire_ticks: 0,
            air_ticks: 300,
            on_ground: self.on_ground,
        };
        let mut tag = NbtTag::compound();
        base.write(&mut tag, world_uuid, entity_uuid);
        tag.insert("Health", NbtTag::Short(self.health as i8 as i16));
        tag.insert("Age", NbtTag::Short(self.age as i16));
        tag.insert("Item", crate::entity::save::item_to_nbt(&self.item));
        tag
    }

    /// Java `EntityItem.a(NBTTagCompound)`: the reading of the dropped item.
    pub fn from_nbt(entity_id: i32, tag: &NbtTag) -> Option<Self> {
        if tag.get_str("id")? != "Item" {
            return None;
        }
        let base = crate::entity::save::BaseEntity::read(tag, "Item")?;
        let item = crate::entity::save::item_from_nbt(tag.get("Item")?)?;
        Some(Self {
            entity_id,
            item,
            x: base.pos.0,
            y: base.pos.1,
            z: base.pos.2,
            mot_x: base.motion.0,
            mot_y: base.motion.1,
            mot_z: base.motion.2,
            on_ground: base.on_ground,
            age: tag.get_i16("Age").unwrap_or(0) as i32,
            pickup_delay: 0,
            health: tag.get_i16("Health").unwrap_or(5),
            dead: false,
        })
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

    /// Java `EntityBoat.b(NBTTagCompound)`: the boat has no tags of its own.
    pub fn to_nbt(&self, world_uuid: (i64, i64), entity_uuid: (i64, i64)) -> NbtTag {
        let base = crate::entity::save::BaseEntity {
            id: "Boat",
            pos: (self.x, self.y, self.z),
            motion: (self.mot_x, self.mot_y, self.mot_z),
            rotation: (self.yaw, self.pitch),
            fall_distance: 0.0,
            fire_ticks: 0,
            air_ticks: 300,
            on_ground: self.on_ground,
        };
        let mut tag = NbtTag::compound();
        base.write(&mut tag, world_uuid, entity_uuid);
        tag
    }

    /// Java `EntityBoat.a(NBTTagCompound)`: the reading of the boat.
    pub fn from_nbt(entity_id: i32, tag: &NbtTag) -> Option<Self> {
        if tag.get_str("id")? != "Boat" {
            return None;
        }
        let base = crate::entity::save::BaseEntity::read(tag, "Boat")?;
        Some(Self {
            entity_id,
            x: base.pos.0,
            y: base.pos.1,
            z: base.pos.2,
            prev_x: base.pos.0,
            prev_y: base.pos.1,
            prev_z: base.pos.2,
            mot_x: base.motion.0,
            mot_y: base.motion.1,
            mot_z: base.motion.2,
            yaw: base.rotation.0,
            pitch: base.rotation.1,
            damage: 0,
            time_since_hit: 0,
            rock_direction: 1,
            passenger_id: None,
            on_ground: base.on_ground,
            dead: false,
        })
    }
}

#[derive(Debug, Default)]
pub struct EntityStore {
    players: HashMap<i32, Player>,
    conn_to_entity: HashMap<ConnId, i32>,
    chunk_index: HashMap<ChunkPos, HashSet<i32>>,
    dropped_items: HashMap<i32, DroppedItem>,
    boats: HashMap<i32, Boat>,
    /// The mobs in the order of their insertion (`World.entityList` of the
    /// reference).
    ///
    /// The reference keeps the entities in a list and walks it in the order of
    /// the insertion. The mobs share one random stream, so the order of the
    /// update decides the order of the draws: it has to be reproducible.
    mobs: Vec<Mob>,
    /// The position of a mob in `mobs` (`World.entityList.indexOf`).
    mob_index: HashMap<i32, usize>,
    projectiles: HashMap<i32, Projectile>,
    primed_tnts: HashMap<i32, PrimedTnt>,
    minecarts: HashMap<i32, Minecart>,
    paintings: HashMap<i32, Painting>,
    next_id: i32,
    /// Dimension of non-player entities (players carry their own field).
    /// Missing entries mean the default overworld dimension 0.
    dimensions: HashMap<i32, i32>,
    /// Dimension assigned to entities inserted right now. The server sets
    /// this while a dimension's world is active.
    pub spawn_dimension: i32,
}

/// The entities of one chunk: the slice of the chunk, the identifier of the
/// entity and the tags of the entity.
type ChunkEntities = Vec<(u8, i32, NbtTag)>;

/// Java `Chunk.a(Entity)`: the slice of the chunk the entity is in (the
/// height of the entity divided by sixteen, eight slices).
fn entity_slice(y: f64) -> u8 {
    let slice = (y / 16.0).floor() as i32;
    slice.clamp(0, 7) as u8
}

impl EntityStore {
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
            conn_to_entity: HashMap::new(),
            chunk_index: HashMap::new(),
            dropped_items: HashMap::new(),
            boats: HashMap::new(),
            mobs: Vec::new(),
            mob_index: HashMap::new(),
            projectiles: HashMap::new(),
            primed_tnts: HashMap::new(),
            minecarts: HashMap::new(),
            paintings: HashMap::new(),
            next_id: 1,
            dimensions: HashMap::new(),
            spawn_dimension: 0,
        }
    }

    /// Java `ChunkLoader.a(Chunk, World, NBTTagCompound)`: the entities of the
    /// chunk are written into the list `Entities`.
    ///
    /// The reference walks the slices of the chunk from the bottom to the top
    /// and the entities of a slice in the order of their insertion; the
    /// identifier of an entity grows with the order of the insertion, so the
    /// order of the identifiers reproduces that order.
    pub fn all_entities_nbt(
        &self,
        world_uuid: (i64, i64),
        seed: i64,
    ) -> HashMap<(i32, ChunkPos), Vec<NbtTag>> {
        let mut map: HashMap<(i32, ChunkPos), ChunkEntities> = HashMap::new();

        for mob in &self.mobs {
            if mob.dead {
                continue;
            }
            let uuid = crate::util::hash::entity_uuid(seed, mob.entity_id);
            let tag = mob.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(mob.entity_id), mob.chunk_pos()))
                .or_default()
                .push((entity_slice(mob.y), mob.entity_id, tag));
        }

        for item in self.dropped_items.values() {
            if item.dead {
                continue;
            }
            let uuid = crate::util::hash::entity_uuid(seed, item.entity_id);
            let tag = item.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(item.entity_id), item.chunk_pos()))
                .or_default()
                .push((entity_slice(item.y), item.entity_id, tag));
        }

        for boat in self.boats.values() {
            if boat.dead {
                continue;
            }
            let uuid = crate::util::hash::entity_uuid(seed, boat.entity_id);
            let tag = boat.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(boat.entity_id), boat.chunk_pos()))
                .or_default()
                .push((entity_slice(boat.y), boat.entity_id, tag));
        }

        for tnt in self.primed_tnts.values() {
            if tnt.dead {
                continue;
            }
            let uuid = crate::util::hash::entity_uuid(seed, tnt.entity_id);
            let tag = tnt.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(tnt.entity_id), tnt.chunk_pos()))
                .or_default()
                .push((entity_slice(tnt.y), tnt.entity_id, tag));
        }

        for cart in self.minecarts.values() {
            let uuid = crate::util::hash::entity_uuid(seed, cart.entity_id);
            let tag = cart.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(cart.entity_id), cart.chunk_pos()))
                .or_default()
                .push((entity_slice(cart.y), cart.entity_id, tag));
        }

        for proj in self.projectiles.values() {
            if proj.dead {
                continue;
            }
            // The egg and the fireball are not in the table `EntityTypes`:
            // the reference does not save them.
            if !matches!(proj.kind, ProjectileKind::Arrow | ProjectileKind::Snowball) {
                continue;
            }
            let uuid = crate::util::hash::entity_uuid(seed, proj.entity_id);
            let tag = proj.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(proj.entity_id), proj.chunk_pos()))
                .or_default()
                .push((entity_slice(proj.y), proj.entity_id, tag));
        }

        for painting in self.paintings.values() {
            if painting.dead {
                continue;
            }
            let uuid = crate::util::hash::entity_uuid(seed, painting.entity_id);
            let tag = painting.to_nbt(world_uuid, uuid);
            map.entry((self.dimension_of(painting.entity_id), painting.chunk_pos()))
                .or_default()
                .push((entity_slice(painting.y as f64), painting.entity_id, tag));
        }

        // The order of the slices and of the insertion.
        for entities in map.values_mut() {
            entities.sort_by_key(|(slice, id, _)| (*slice, *id));
        }
        map.into_iter()
            .map(|(key, entities)| (key, entities.into_iter().map(|(_, _, tag)| tag).collect()))
            .collect()
    }

    /// Java `ChunkLoader.a(Chunk, World, NBTTagCompound)`: the entities of the
    /// chunk of the dimension.
    pub fn chunk_entities_nbt(
        &self,
        pos: ChunkPos,
        dimension: i32,
        world_uuid: (i64, i64),
        seed: i64,
    ) -> Vec<NbtTag> {
        self.all_entities_nbt(world_uuid, seed)
            .remove(&(dimension, pos))
            .unwrap_or_default()
    }

    /// Java `ChunkLoader.a(World, NBTTagCompound)`: the entities of the chunk
    /// are restored.
    pub fn restore_entities(&mut self, tags: &[NbtTag], rng: &mut JavaRandom) -> Vec<i32> {
        let mut restored = Vec::new();
        for tag in tags {
            let Some(name) = tag.get_str("id") else {
                continue;
            };
            match name {
                "Item" => {
                    let id = self.allocate_id();
                    if let Some(item) = DroppedItem::from_nbt(id, tag) {
                        self.dropped_items.insert(id, item);
                        self.tag_spawned(id);
                        restored.push(id);
                    }
                }
                "Boat" => {
                    let id = self.allocate_id();
                    if let Some(boat) = Boat::from_nbt(id, tag) {
                        self.boats.insert(id, boat);
                        self.tag_spawned(id);
                        restored.push(id);
                    }
                }
                "PrimedTnt" => {
                    let id = self.allocate_id();
                    if let Some(tnt) = PrimedTnt::from_nbt(id, tag) {
                        self.primed_tnts.insert(id, tnt);
                        self.tag_spawned(id);
                        restored.push(id);
                    }
                }
                "Minecart" => {
                    let id = self.allocate_id();
                    if let Some(cart) = Minecart::from_nbt(id, tag) {
                        self.minecarts.insert(id, cart);
                        self.tag_spawned(id);
                        restored.push(id);
                    }
                }
                "Arrow" | "Snowball" => {
                    let kind = if name == "Arrow" {
                        ProjectileKind::Arrow
                    } else {
                        ProjectileKind::Snowball
                    };
                    let id = self.allocate_id();
                    if let Some(proj) = Projectile::from_nbt(id, kind, tag) {
                        self.projectiles.insert(id, proj);
                        self.tag_spawned(id);
                        restored.push(id);
                    }
                }
                "Painting" => {
                    let id = self.allocate_id();
                    if let Some(painting) = Painting::from_nbt(id, tag) {
                        self.paintings.insert(id, painting);
                        self.tag_spawned(id);
                        restored.push(id);
                    }
                }
                _ => {
                    // The mobs: the name of the type is the name of the mob.
                    let id = self.allocate_id();
                    if let Some(mob) = Mob::from_nbt(id, tag, rng) {
                        self.insert_mob(mob);
                        restored.push(id);
                    }
                }
            }
        }
        restored
    }

    pub fn allocate_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn tag_spawned(&mut self, entity_id: i32) {
        if self.spawn_dimension != 0 {
            self.dimensions.insert(entity_id, self.spawn_dimension);
        } else {
            self.dimensions.remove(&entity_id);
        }
    }

    pub fn dimension_of(&self, entity_id: i32) -> i32 {
        self.dimensions.get(&entity_id).copied().unwrap_or(0)
    }

    pub fn remove_dimension(&mut self, entity_id: i32) {
        self.dimensions.remove(&entity_id);
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
        self.tag_spawned(entity_id);
        entity_id
    }

    pub fn get_dropped_item(&self, entity_id: i32) -> Option<&DroppedItem> {
        self.dropped_items.get(&entity_id)
    }

    pub fn get_dropped_item_mut(&mut self, entity_id: i32) -> Option<&mut DroppedItem> {
        self.dropped_items.get_mut(&entity_id)
    }

    pub fn remove_dropped_item(&mut self, entity_id: i32) -> Option<DroppedItem> {
        self.remove_dimension(entity_id);
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
        for id in &removed {
            self.remove_dimension(*id);
        }
        removed
    }

    /// Spawn a new `EntityBoat` at `(x, y, z)`.
    ///
    /// Java `EntityBoat(World, double, double, double)`: the boat is placed
    /// at `y + 0.3` (`height = width / 2`).
    pub fn spawn_boat(&mut self, x: f64, y: f64, z: f64) -> i32 {
        let entity_id = self.allocate_id();
        let y = y + 0.30000001192092896;
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
        self.tag_spawned(entity_id);
        entity_id
    }

    pub fn get_boat(&self, entity_id: i32) -> Option<&Boat> {
        self.boats.get(&entity_id)
    }

    pub fn get_boat_mut(&mut self, entity_id: i32) -> Option<&mut Boat> {
        self.boats.get_mut(&entity_id)
    }

    pub fn remove_boat(&mut self, entity_id: i32) -> Option<Boat> {
        self.remove_dimension(entity_id);
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

    pub fn spawn_mob(
        &mut self,
        mob_type: MobType,
        x: f64,
        y: f64,
        z: f64,
        rng: &mut JavaRandom,
    ) -> i32 {
        let entity_id = self.allocate_id();
        let mob = Mob::new(entity_id, mob_type, x, y, z, rng);
        self.insert_mob(mob);
        entity_id
    }

    pub fn insert_mob(&mut self, mob: Mob) -> i32 {
        let id = mob.entity_id;
        match self.mob_index.get(&id).copied() {
            Some(index) => self.mobs[index] = mob,
            None => {
                self.mob_index.insert(id, self.mobs.len());
                self.mobs.push(mob);
            }
        }
        self.tag_spawned(id);
        id
    }

    pub fn get_mob(&self, entity_id: i32) -> Option<&Mob> {
        self.mob_index
            .get(&entity_id)
            .and_then(|index| self.mobs.get(*index))
    }

    pub fn get_mob_mut(&mut self, entity_id: i32) -> Option<&mut Mob> {
        let index = *self.mob_index.get(&entity_id)?;
        self.mobs.get_mut(index)
    }

    pub fn remove_mob(&mut self, entity_id: i32) -> Option<Mob> {
        self.remove_dimension(entity_id);
        let index = self.mob_index.remove(&entity_id)?;
        let mob = self.mobs.remove(index);
        // The positions of the mobs after the removed one are shifted.
        for position in self.mob_index.values_mut() {
            if *position > index {
                *position -= 1;
            }
        }
        Some(mob)
    }

    /// The mobs in the order of their insertion (`World.entityList`).
    pub fn iter_mobs(&self) -> impl Iterator<Item = &Mob> {
        self.mobs.iter()
    }

    /// The mobs in the order of their insertion (`World.entityList`).
    pub fn iter_mobs_mut(&mut self) -> impl Iterator<Item = &mut Mob> {
        self.mobs.iter_mut()
    }

    pub fn count_mob_categories(&self, dimension: i32) -> (usize, usize, usize) {
        let mut monsters = 0;
        let mut animals = 0;
        let mut water = 0;
        for m in self.iter_mobs() {
            if m.dead || self.dimension_of(m.entity_id) != dimension {
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
            shake: 0,
            x_tile: 0,
            y_tile: 0,
            z_tile: 0,
            in_tile: 0,
            ticks_in_ground: 0,
            ticks_in_air: 0,
            catch_ticks: 0,
            hooked_entity: None,
            from_player,
            dead: false,
        };
        self.projectiles.insert(entity_id, proj);
        self.tag_spawned(entity_id);
        entity_id
    }

    pub fn get_projectile(&self, entity_id: i32) -> Option<&Projectile> {
        self.projectiles.get(&entity_id)
    }

    pub fn get_projectile_mut(&mut self, entity_id: i32) -> Option<&mut Projectile> {
        self.projectiles.get_mut(&entity_id)
    }

    pub fn remove_projectile(&mut self, entity_id: i32) -> Option<Projectile> {
        self.remove_dimension(entity_id);
        self.projectiles.remove(&entity_id)
    }

    pub fn iter_projectiles(&self) -> impl Iterator<Item = &Projectile> {
        self.projectiles.values()
    }

    pub fn iter_projectiles_mut(&mut self) -> impl Iterator<Item = &mut Projectile> {
        self.projectiles.values_mut()
    }

    /// Spawns a new `EntityTNTPrimed`.
    ///
    /// Java `EntityTNTPrimed(World, double, double, double)`: the angle of
    /// the hit comes from `Math.random()`, not from the shared stream of
    /// the entities, so the caller passes it in.
    pub fn spawn_primed_tnt(&mut self, x: f64, y: f64, z: f64, fuse: i32, angle: f64) -> i32 {
        let entity_id = self.allocate_id();
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
        self.tag_spawned(entity_id);
        entity_id
    }

    pub fn get_primed_tnt(&self, entity_id: i32) -> Option<&PrimedTnt> {
        self.primed_tnts.get(&entity_id)
    }

    pub fn get_primed_tnt_mut(&mut self, entity_id: i32) -> Option<&mut PrimedTnt> {
        self.primed_tnts.get_mut(&entity_id)
    }

    pub fn remove_primed_tnt(&mut self, entity_id: i32) -> Option<PrimedTnt> {
        self.remove_dimension(entity_id);
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
        self.tag_spawned(entity_id);
        entity_id
    }

    pub fn insert_minecart(&mut self, cart: Minecart) -> i32 {
        let id = cart.entity_id;
        self.minecarts.insert(id, cart);
        self.tag_spawned(id);
        id
    }

    pub fn get_minecart(&self, entity_id: i32) -> Option<&Minecart> {
        self.minecarts.get(&entity_id)
    }

    pub fn get_minecart_mut(&mut self, entity_id: i32) -> Option<&mut Minecart> {
        self.minecarts.get_mut(&entity_id)
    }

    pub fn remove_minecart(&mut self, entity_id: i32) -> Option<Minecart> {
        self.remove_dimension(entity_id);
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
            tick_count: 0,
            dead: false,
        };
        self.paintings.insert(entity_id, p);
        self.tag_spawned(entity_id);
        entity_id
    }

    pub fn get_painting(&self, entity_id: i32) -> Option<&Painting> {
        self.paintings.get(&entity_id)
    }

    pub fn get_painting_mut(&mut self, entity_id: i32) -> Option<&mut Painting> {
        self.paintings.get_mut(&entity_id)
    }

    pub fn remove_painting(&mut self, entity_id: i32) -> Option<Painting> {
        self.remove_dimension(entity_id);
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
