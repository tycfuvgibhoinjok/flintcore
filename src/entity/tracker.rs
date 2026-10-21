use std::collections::{HashMap, HashSet};

use crate::entity::mob::Mob;
use crate::entity::player::Player;
use crate::entity::projectile::{Minecart, Painting, PrimedTnt, Projectile, ProjectileKind};
use crate::entity::store::{Boat, DroppedItem};
use crate::net::packets::{
    AttachEntity, Clientbound, DestroyEntity, EntityEquipment, EntityLook, EntityMetadata,
    EntityPainting, EntityTeleport, EntityVelocity, MobSpawn, NamedEntitySpawn, PickupSpawn,
    RelEntityMove, RelEntityMoveLook, Sleep, VehicleSpawn,
};
use crate::world::world::FallingSand;

#[inline]
pub fn pack_angle(angle: f32) -> u8 {
    let scaled = (angle * 256.0 / 360.0).floor() as i32;
    (scaled as i8) as u8
}

#[derive(Debug)]
pub struct EntityTrackerEntry {
    pub entity_id: i32,
    pub encoded_x: i32,
    pub encoded_y: i32,
    pub encoded_z: i32,
    pub encoded_yaw: u8,
    pub encoded_pitch: u8,
    pub last_mot_x: f64,
    pub last_mot_y: f64,
    pub last_mot_z: f64,
    pub last_meta_flags: u8,
    pub last_meta_bytes: Vec<u8>,
    pub teleport_ticks: u32,
    pub tracked_players: HashSet<i32>,
    pub tracking_range: f64,
    /// Java `EntityTrackerEntry.c`: the interval of the update of the
    /// position of the entity (in ticks).
    pub update_interval: u32,
    /// Java `EntityTrackerEntry.l`: the counter of the updates.
    pub update_counter: u32,
}

impl EntityTrackerEntry {
    pub fn new(player: &Player, tracking_range: f64) -> Self {
        Self {
            entity_id: player.entity_id,
            encoded_x: (player.x * 32.0).floor() as i32,
            encoded_y: (player.y * 32.0).floor() as i32,
            encoded_z: (player.z * 32.0).floor() as i32,
            encoded_yaw: pack_angle(player.yaw),
            encoded_pitch: pack_angle(player.pitch),
            last_mot_x: player.mot_x,
            last_mot_y: player.mot_y,
            last_mot_z: player.mot_z,
            last_meta_flags: player.metadata_flags(),
            last_meta_bytes: Vec::new(),
            teleport_ticks: 0,
            tracked_players: HashSet::new(),
            tracking_range,
            // The player is updated every two ticks.
            update_interval: 2,
            update_counter: 0,
        }
    }

    pub fn new_item(item: &DroppedItem) -> Self {
        Self {
            entity_id: item.entity_id,
            encoded_x: (item.x * 32.0).floor() as i32,
            encoded_y: (item.y * 32.0).floor() as i32,
            encoded_z: (item.z * 32.0).floor() as i32,
            encoded_yaw: 0,
            encoded_pitch: 0,
            last_mot_x: item.mot_x,
            last_mot_y: item.mot_y,
            last_mot_z: item.mot_z,
            last_meta_flags: 0,
            last_meta_bytes: Vec::new(),
            teleport_ticks: 0,
            tracked_players: HashSet::new(),
            tracking_range: 64.0,
            // The item is updated every twenty ticks.
            update_interval: 20,
            update_counter: 0,
        }
    }

    pub fn new_boat(boat: &Boat) -> Self {
        Self {
            entity_id: boat.entity_id,
            encoded_x: (boat.x * 32.0).floor() as i32,
            encoded_y: (boat.y * 32.0).floor() as i32,
            encoded_z: (boat.z * 32.0).floor() as i32,
            encoded_yaw: pack_angle(boat.yaw),
            encoded_pitch: pack_angle(boat.pitch),
            last_mot_x: boat.mot_x,
            last_mot_y: boat.mot_y,
            last_mot_z: boat.mot_z,
            last_meta_flags: 0,
            last_meta_bytes: Vec::new(),
            teleport_ticks: 0,
            tracked_players: HashSet::new(),
            tracking_range: 160.0,
            // The boat is updated every five ticks.
            update_interval: 5,
            update_counter: 0,
        }
    }

    pub fn new_mob(mob: &Mob) -> Self {
        Self {
            entity_id: mob.entity_id,
            encoded_x: (mob.x * 32.0).floor() as i32,
            encoded_y: (mob.y * 32.0).floor() as i32,
            encoded_z: (mob.z * 32.0).floor() as i32,
            encoded_yaw: pack_angle(mob.yaw),
            encoded_pitch: pack_angle(mob.pitch),
            last_mot_x: mob.mot_x,
            last_mot_y: mob.mot_y,
            last_mot_z: mob.mot_z,
            last_meta_flags: 0,
            last_meta_bytes: mob.encode_metadata(),
            teleport_ticks: 0,
            tracked_players: HashSet::new(),
            tracking_range: 160.0,
            // The animals are updated every three ticks.
            update_interval: 3,
            update_counter: 0,
        }
    }

    pub fn new_pos_entity(
        entity_id: i32,
        x: f64,
        y: f64,
        z: f64,
        range: f64,
        update_interval: u32,
    ) -> Self {
        Self {
            entity_id,
            encoded_x: (x * 32.0).floor() as i32,
            encoded_y: (y * 32.0).floor() as i32,
            encoded_z: (z * 32.0).floor() as i32,
            encoded_yaw: 0,
            encoded_pitch: 0,
            last_mot_x: 0.0,
            last_mot_y: 0.0,
            last_mot_z: 0.0,
            last_meta_flags: 0,
            last_meta_bytes: Vec::new(),
            teleport_ticks: 0,
            tracked_players: HashSet::new(),
            tracking_range: range,
            update_interval,
            update_counter: 0,
        }
    }

    pub fn make_spawn_packets(&self, player: &Player) -> Vec<Clientbound> {
        let mut packets = Vec::with_capacity(9);
        let held_item = player.inventory.item_in_hand().map(|i| i.id).unwrap_or(0);

        packets.push(Clientbound::NamedEntitySpawn(NamedEntitySpawn {
            entity_id: player.entity_id,
            name: player.username.clone(),
            x: self.encoded_x,
            y: self.encoded_y,
            z: self.encoded_z,
            yaw: self.encoded_yaw,
            pitch: self.encoded_pitch,
            current_item: held_item,
        }));

        if player.sleeping {
            if let Some((bx, by, bz)) = player.bed_pos {
                packets.push(Clientbound::Sleep(Sleep {
                    entity_id: player.entity_id,
                    in_bed_code: 0,
                    bed_x: bx,
                    bed_y: by as u8,
                    bed_z: bz,
                }));
            }
        }

        let flags = player.metadata_flags();
        packets.push(Clientbound::EntityMetadata(EntityMetadata {
            entity_id: player.entity_id,
            metadata: vec![0, flags, 127],
        }));

        if let Some(vehicle_id) = player.vehicle_id {
            packets.push(Clientbound::AttachEntity(AttachEntity {
                entity_id: player.entity_id,
                vehicle_id,
            }));
        }

        // Equipment 0..=4
        for slot in 0..5 {
            let stack = player.inventory.get_equipment(slot);
            packets.push(Clientbound::EntityEquipment(EntityEquipment {
                entity_id: player.entity_id,
                slot,
                item_id: stack.map(|s| s.id).unwrap_or(-1),
                damage: stack.map(|s| s.damage).unwrap_or(0),
            }));
        }

        packets
    }

    pub fn make_item_spawn_packets(&self, item: &DroppedItem) -> Vec<Clientbound> {
        let mot_x = (item.mot_x.clamp(-0.99, 0.99) * 128.0) as i8;
        let mot_y = (item.mot_y.clamp(-0.99, 0.99) * 128.0) as i8;
        let mot_z = (item.mot_z.clamp(-0.99, 0.99) * 128.0) as i8;
        let v_x = (item.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16;
        let v_y = (item.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16;
        let v_z = (item.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16;

        vec![
            Clientbound::PickupSpawn(PickupSpawn {
                entity_id: item.entity_id,
                item_id: item.item.id,
                count: item.item.count as i8,
                damage: item.item.damage,
                x: self.encoded_x,
                y: self.encoded_y,
                z: self.encoded_z,
                mot_x,
                mot_y,
                mot_z,
            }),
            Clientbound::EntityVelocity(EntityVelocity {
                entity_id: item.entity_id,
                mot_x: v_x,
                mot_y: v_y,
                mot_z: v_z,
            }),
        ]
    }

    pub fn make_boat_spawn_packets(&self, boat: &Boat) -> Vec<Clientbound> {
        let mut packets = Vec::with_capacity(3);
        packets.push(Clientbound::VehicleSpawn(VehicleSpawn {
            entity_id: boat.entity_id,
            vehicle_type: 1,
            x: self.encoded_x,
            y: self.encoded_y,
            z: self.encoded_z,
            thrower_id: 0,
            velocity: None,
        }));
        let v_x = (boat.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16;
        let v_y = (boat.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16;
        let v_z = (boat.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16;
        packets.push(Clientbound::EntityVelocity(EntityVelocity {
            entity_id: boat.entity_id,
            mot_x: v_x,
            mot_y: v_y,
            mot_z: v_z,
        }));
        if let Some(passenger_id) = boat.passenger_id {
            packets.push(Clientbound::AttachEntity(AttachEntity {
                entity_id: passenger_id,
                vehicle_id: boat.entity_id,
            }));
        }
        packets
    }

    pub fn make_mob_spawn_packets(&self, mob: &Mob) -> Vec<Clientbound> {
        let mut packets = Vec::with_capacity(4);
        packets.push(Clientbound::MobSpawn(MobSpawn {
            entity_id: mob.entity_id,
            mob_type: mob.mob_type as u8,
            x: self.encoded_x,
            y: self.encoded_y,
            z: self.encoded_z,
            yaw: self.encoded_yaw,
            pitch: self.encoded_pitch,
            metadata: mob.encode_metadata(),
        }));
        if let Some(held) = mob.held_item() {
            packets.push(Clientbound::EntityEquipment(EntityEquipment {
                entity_id: mob.entity_id,
                slot: 0,
                item_id: held.id,
                damage: held.damage,
            }));
        }
        if let Some(vehicle_id) = mob.vehicle_id {
            packets.push(Clientbound::AttachEntity(AttachEntity {
                entity_id: mob.entity_id,
                vehicle_id,
            }));
        }
        if let Some(passenger_id) = mob.passenger_id {
            packets.push(Clientbound::AttachEntity(AttachEntity {
                entity_id: passenger_id,
                vehicle_id: mob.entity_id,
            }));
        }
        packets
    }

    pub fn make_projectile_spawn_packets(&self, proj: &Projectile) -> Vec<Clientbound> {
        let (vx, vy, vz, thrower_id) = if proj.kind == ProjectileKind::Fireball {
            (
                (proj.accel_x.clamp(-3.9, 3.9) * 8000.0) as i16,
                (proj.accel_y.clamp(-3.9, 3.9) * 8000.0) as i16,
                (proj.accel_z.clamp(-3.9, 3.9) * 8000.0) as i16,
                proj.shooter_id.unwrap_or(1),
            )
        } else {
            (
                (proj.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16,
                (proj.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16,
                (proj.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16,
                proj.shooter_id.unwrap_or(proj.entity_id),
            )
        };
        vec![Clientbound::VehicleSpawn(VehicleSpawn {
            entity_id: proj.entity_id,
            vehicle_type: proj.kind.vehicle_type(),
            x: self.encoded_x,
            y: self.encoded_y,
            z: self.encoded_z,
            thrower_id,
            velocity: Some((vx, vy, vz)),
        })]
    }

    pub fn make_tnt_spawn_packets(&self, tnt: &PrimedTnt) -> Vec<Clientbound> {
        let vx = (tnt.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16;
        let vy = (tnt.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16;
        let vz = (tnt.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16;
        vec![
            Clientbound::VehicleSpawn(VehicleSpawn {
                entity_id: tnt.entity_id,
                vehicle_type: 50,
                x: self.encoded_x,
                y: self.encoded_y,
                z: self.encoded_z,
                thrower_id: 0,
                velocity: None,
            }),
            Clientbound::EntityVelocity(EntityVelocity {
                entity_id: tnt.entity_id,
                mot_x: vx,
                mot_y: vy,
                mot_z: vz,
            }),
        ]
    }

    pub fn make_minecart_spawn_packets(&self, cart: &Minecart) -> Vec<Clientbound> {
        let mut packets = Vec::with_capacity(3);
        packets.push(Clientbound::VehicleSpawn(VehicleSpawn {
            entity_id: cart.entity_id,
            vehicle_type: cart.vehicle_type(),
            x: self.encoded_x,
            y: self.encoded_y,
            z: self.encoded_z,
            thrower_id: 0,
            velocity: None,
        }));
        packets.push(Clientbound::EntityVelocity(EntityVelocity {
            entity_id: cart.entity_id,
            mot_x: (cart.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16,
            mot_y: (cart.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16,
            mot_z: (cart.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16,
        }));
        if let Some(pid) = cart.passenger_id {
            packets.push(Clientbound::AttachEntity(AttachEntity {
                entity_id: pid,
                vehicle_id: cart.entity_id,
            }));
        }
        packets
    }

    /// Java `EntityTrackerEntry.java:377-385`: the falling sand is spawned on
    /// the client as a vehicle with the type 70 (the sand ✓) or 71 (the gravel
    /// ✓).
    pub fn make_falling_sand_spawn_packets(&self, sand: &FallingSand) -> Vec<Clientbound> {
        let vehicle_type = if sand.id == crate::gameplay::blocks::SAND {
            70
        } else {
            71
        };
        vec![Clientbound::VehicleSpawn(VehicleSpawn {
            entity_id: sand.entity_id,
            vehicle_type,
            x: self.encoded_x,
            y: self.encoded_y,
            z: self.encoded_z,
            thrower_id: 0,
            velocity: None,
        })]
    }

    pub fn make_painting_spawn_packets(&self, p: &Painting) -> Vec<Clientbound> {
        vec![Clientbound::EntityPainting(EntityPainting {
            entity_id: p.entity_id,
            title: p.title.clone(),
            x: p.x,
            y: p.y,
            z: p.z,
            direction: p.direction,
        })]
    }
}

pub struct OutboundTrackingPacket {
    pub recipient_id: i32,
    pub packet: Clientbound,
}

#[derive(Debug, Default)]
pub struct EntityTracker {
    entries: HashMap<i32, EntityTrackerEntry>,
    item_entries: HashMap<i32, EntityTrackerEntry>,
    boat_entries: HashMap<i32, EntityTrackerEntry>,
    mob_entries: HashMap<i32, EntityTrackerEntry>,
    projectile_entries: HashMap<i32, EntityTrackerEntry>,
    tnt_entries: HashMap<i32, EntityTrackerEntry>,
    minecart_entries: HashMap<i32, EntityTrackerEntry>,
    painting_entries: HashMap<i32, EntityTrackerEntry>,
    falling_sand_entries: HashMap<i32, EntityTrackerEntry>,
    tracking_range: f64,
}

/// Java `EntityTrackerEntry.b(EntityPlayer)`: the radius of the tracking is
/// a square in XZ, the height of the entity is not taken into account.
fn in_tracking_range(dx: f64, dz: f64, range: f64) -> bool {
    dx.abs() <= range && dz.abs() <= range
}

impl EntityTracker {
    pub fn new(view_distance: i32) -> Self {
        // Java `ServerConfigurationManager.a()`: the radius of the tracking
        // is limited by the distance of the view.
        let tracking_range = (view_distance * 16 - 16).max(0) as f64;
        Self {
            entries: HashMap::new(),
            item_entries: HashMap::new(),
            boat_entries: HashMap::new(),
            mob_entries: HashMap::new(),
            projectile_entries: HashMap::new(),
            tnt_entries: HashMap::new(),
            minecart_entries: HashMap::new(),
            painting_entries: HashMap::new(),
            falling_sand_entries: HashMap::new(),
            tracking_range,
        }
    }

    pub fn track_player(&mut self, player: &Player) {
        let entry = EntityTrackerEntry::new(player, self.tracking_range);
        self.entries.insert(player.entity_id, entry);
    }

    pub fn track_item(&mut self, item: &DroppedItem) {
        let entry = EntityTrackerEntry::new_item(item);
        self.item_entries.insert(item.entity_id, entry);
    }

    pub fn track_item_and_scan(
        &mut self,
        item: &DroppedItem,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let mut entry = EntityTrackerEntry::new_item(item);
        // The radius of the entity is limited by the distance of the view.
        let item_range = 64.0f64.min(self.tracking_range);
        let target_chunk = item.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - item.x;
            let dz = p.z - item.z;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, item_range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_item_spawn_packets(item) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.item_entries.insert(item.entity_id, entry);
        outbound
    }

    pub fn untrack_item(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.item_entries, entity_id)
    }

    pub fn track_boat_and_scan(
        &mut self,
        boat: &Boat,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let mut entry = EntityTrackerEntry::new_boat(boat);
        // The radius of the entity is limited by the distance of the view.
        let boat_range = 160.0f64.min(self.tracking_range);
        let target_chunk = boat.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - boat.x;
            let dz = p.z - boat.z;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, boat_range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_boat_spawn_packets(boat) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.boat_entries.insert(boat.entity_id, entry);
        outbound
    }

    pub fn untrack_boat(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.boat_entries, entity_id)
    }

    pub fn track_mob_and_scan(
        &mut self,
        mob: &Mob,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let mut entry = EntityTrackerEntry::new_mob(mob);
        // The radius of the entity is limited by the distance of the view.
        let range = 160.0f64.min(self.tracking_range);
        let target_chunk = mob.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - mob.x;
            let dz = p.z - mob.z;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_mob_spawn_packets(mob) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.mob_entries.insert(mob.entity_id, entry);
        outbound
    }

    pub fn untrack_mob(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.mob_entries, entity_id)
    }

    pub fn track_projectile_and_scan(
        &mut self,
        proj: &Projectile,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        // Java `EntityTracker.track`: the arrow is updated every twenty
        // ticks, the fireball, the snowball and the egg every ten.
        let interval = match proj.kind {
            crate::entity::projectile::ProjectileKind::Arrow
            | crate::entity::projectile::ProjectileKind::FishingHook => 20,
            _ => 10,
        };
        let mut entry = EntityTrackerEntry::new_pos_entity(
            proj.entity_id,
            proj.x,
            proj.y,
            proj.z,
            64.0,
            interval,
        );
        // The radius of the entity is limited by the distance of the view.
        let range = 64.0f64.min(self.tracking_range);
        let target_chunk = proj.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - proj.x;
            let dz = p.z - proj.z;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_projectile_spawn_packets(proj) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.projectile_entries.insert(proj.entity_id, entry);
        outbound
    }

    pub fn untrack_projectile(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.projectile_entries, entity_id)
    }

    pub fn track_tnt_and_scan(
        &mut self,
        tnt: &PrimedTnt,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let mut entry =
            // The primed TNT is updated every ten ticks.
            EntityTrackerEntry::new_pos_entity(tnt.entity_id, tnt.x, tnt.y, tnt.z, 160.0, 10);
        // The radius of the entity is limited by the distance of the view.
        let range = 160.0f64.min(self.tracking_range);
        let target_chunk = tnt.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - tnt.x;
            let dz = p.z - tnt.z;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_tnt_spawn_packets(tnt) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.tnt_entries.insert(tnt.entity_id, entry);
        outbound
    }

    pub fn untrack_tnt(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.tnt_entries, entity_id)
    }

    pub fn track_minecart_and_scan(
        &mut self,
        cart: &Minecart,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let mut entry =
            // The minecart is updated every five ticks.
            EntityTrackerEntry::new_pos_entity(cart.entity_id, cart.x, cart.y, cart.z, 160.0, 5);
        // The radius of the entity is limited by the distance of the view.
        let range = 160.0f64.min(self.tracking_range);
        let target_chunk = cart.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - cart.x;
            let dz = p.z - cart.z;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_minecart_spawn_packets(cart) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.minecart_entries.insert(cart.entity_id, entry);
        outbound
    }

    pub fn untrack_minecart(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.minecart_entries, entity_id)
    }

    pub fn track_painting_and_scan(
        &mut self,
        painting: &Painting,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        // Java `EntityTracker.track`: the painting is never updated, its
        // interval is `Integer.MAX_VALUE`.
        let mut entry = EntityTrackerEntry::new_pos_entity(
            painting.entity_id,
            painting.x as f64,
            painting.y as f64,
            painting.z as f64,
            160.0,
            u32::MAX,
        );
        // The radius of the entity is limited by the distance of the view.
        let range = 160.0f64.min(self.tracking_range);
        let target_chunk = painting.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - painting.x as f64;
            let dz = p.z - painting.z as f64;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_painting_spawn_packets(painting) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.painting_entries.insert(painting.entity_id, entry);
        outbound
    }

    /// Java `EntityTracker.track`: the falling sand is tracked with the radius
    /// 160 ✓ and the update interval 20 ✓ (`EntityTracker.java:58-59` ✓).
    pub fn track_falling_sand_and_scan(
        &mut self,
        sand: &FallingSand,
        players: &[Player],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let mut entry = EntityTrackerEntry::new_pos_entity(
            sand.entity_id,
            sand.x as f64 + 0.5,
            sand.loc_y,
            sand.z as f64 + 0.5,
            160.0,
            20,
        );
        // The radius of the entity is limited by the distance of the view.
        let range = 160.0f64.min(self.tracking_range);
        let target_chunk = crate::world::chunk::ChunkPos::new(sand.x >> 4, sand.z >> 4);

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - (sand.x as f64 + 0.5);
            let dz = p.z - (sand.z as f64 + 0.5);
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if in_tracking_range(dx, dz, range)
                && chunk_ready
                && entry.tracked_players.insert(p.entity_id)
            {
                for packet in entry.make_falling_sand_spawn_packets(sand) {
                    outbound.push(OutboundTrackingPacket {
                        recipient_id: p.entity_id,
                        packet,
                    });
                }
            }
        }

        self.falling_sand_entries.insert(sand.entity_id, entry);
        outbound
    }

    /// The updates of the falling sand (`EntityTrackerEntry.update()` ✓).
    ///
    /// The falling sand is in the air ✓ — the packet of the position is sent
    /// every tick ✓ (`EntityTrackerEntry.update()` ✓: the entity is not on
    /// the ground ✓).
    pub fn tick_falling_sands(&mut self, sands: &[FallingSand]) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        for sand in sands {
            if sand.entity_id == 0 {
                continue;
            }
            if let Some(entry) = self.falling_sand_entries.get_mut(&sand.entity_id) {
                Self::emit_pos_only_update(
                    entry,
                    sand.entity_id,
                    sand.x as f64 + 0.5,
                    sand.loc_y,
                    sand.z as f64 + 0.5,
                    true,
                    &mut outbound,
                );
            }
        }
        outbound
    }

    /// The numbers of the falling sand that are tracked (`EntityTracker.b` ✓).
    pub fn falling_sand_ids(&self) -> Vec<i32> {
        self.falling_sand_entries.keys().copied().collect()
    }

    pub fn untrack_falling_sand(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.falling_sand_entries, entity_id)
    }

    pub fn untrack_painting(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        Self::untrack_from(&mut self.painting_entries, entity_id)
    }

    fn untrack_from(
        map: &mut HashMap<i32, EntityTrackerEntry>,
        entity_id: i32,
    ) -> Vec<OutboundTrackingPacket> {
        let mut packets = Vec::new();
        if let Some(entry) = map.remove(&entity_id) {
            let destroy = Clientbound::DestroyEntity(DestroyEntity { entity_id });
            for &viewer_id in &entry.tracked_players {
                packets.push(OutboundTrackingPacket {
                    recipient_id: viewer_id,
                    packet: destroy.clone(),
                });
            }
        }
        packets
    }

    pub fn untrack_player(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        let packets = Self::untrack_from(&mut self.entries, entity_id);

        for entry in self.entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.item_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.boat_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.mob_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.projectile_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.tnt_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.minecart_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }
        for entry in self.painting_entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }

        packets
    }

    pub fn observers_of_player(&self, entity_id: i32) -> Vec<i32> {
        self.entries
            .get(&entity_id)
            .map(|e| e.tracked_players.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn observers_of_mob(&self, entity_id: i32) -> Vec<i32> {
        self.mob_entries
            .get(&entity_id)
            .map(|e| e.tracked_players.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn tick(&mut self, players: &mut [Player]) -> Vec<OutboundTrackingPacket> {
        self.tick_world(
            players,
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            &[],
        )
    }

    pub fn tick_with_items(
        &mut self,
        players: &mut [Player],
        items: &mut [DroppedItem],
    ) -> Vec<OutboundTrackingPacket> {
        self.tick_world(
            players,
            items,
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            &[],
        )
    }

    pub fn tick_all(
        &mut self,
        players: &mut [Player],
        items: &mut [DroppedItem],
        boats: &mut [Boat],
    ) -> Vec<OutboundTrackingPacket> {
        self.tick_world(
            players,
            items,
            boats,
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            &[],
        )
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub fn tick_world(
        &mut self,
        players: &mut [Player],
        items: &mut [DroppedItem],
        boats: &mut [Boat],
        mobs: &mut [Mob],
        projectiles: &mut [Projectile],
        tnts: &mut [PrimedTnt],
        minecarts: &mut [Minecart],
        paintings: &[Painting],
    ) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let range = self.tracking_range;

        let observers: Vec<(
            i32,
            f64,
            f64,
            f64,
            bool,
            &HashSet<crate::world::chunk::ChunkPos>,
        )> = players
            .iter()
            .map(|p| (p.entity_id, p.x, p.y, p.z, p.dead, &p.sent_chunks))
            .collect();

        // 1. Proximity scan for players
        for player in players.iter() {
            let entity_id = player.entity_id;
            let target_chunk = player.chunk_pos();
            if let Some(entry) = self.entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    if other_id == entity_id {
                        continue;
                    }
                    let dx = ox - player.x;
                    let dz = oz - player.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !player.dead
                        && !other_dead
                        && in_tracking_range(dx, dz, range)
                        && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_spawn_packets(player) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        // Proximity scan for dropped items
        for item in items.iter() {
            let entity_id = item.entity_id;
            let target_chunk = item.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let item_range = 64.0f64.min(self.tracking_range);
            if let Some(entry) = self.item_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - item.x;
                    let dz = oz - item.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !item.dead
                        && !other_dead
                        && in_tracking_range(dx, dz, item_range)
                        && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_item_spawn_packets(item) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        // Proximity scan for boats
        for boat in boats.iter() {
            let entity_id = boat.entity_id;
            let target_chunk = boat.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let boat_range = 160.0f64.min(self.tracking_range);
            if let Some(entry) = self.boat_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - boat.x;
                    let dz = oz - boat.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !boat.dead
                        && !other_dead
                        && in_tracking_range(dx, dz, boat_range)
                        && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_boat_spawn_packets(boat) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        // Proximity scan for mobs
        for mob in mobs.iter() {
            let entity_id = mob.entity_id;
            let target_chunk = mob.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let mob_range = 160.0f64.min(self.tracking_range);
            if let Some(entry) = self.mob_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - mob.x;
                    let dz = oz - mob.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !mob.dead
                        && !other_dead
                        && in_tracking_range(dx, dz, mob_range)
                        && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_mob_spawn_packets(mob) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        // Proximity scan for projectiles, primed TNTs, minecarts, paintings
        for proj in projectiles.iter() {
            let entity_id = proj.entity_id;
            let target_chunk = proj.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let range = 64.0f64.min(self.tracking_range);
            if let Some(entry) = self.projectile_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - proj.x;
                    let dz = oz - proj.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !proj.dead && !other_dead && in_tracking_range(dx, dz, range) && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_projectile_spawn_packets(proj) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        for tnt in tnts.iter() {
            let entity_id = tnt.entity_id;
            let target_chunk = tnt.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let range = 160.0f64.min(self.tracking_range);
            if let Some(entry) = self.tnt_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - tnt.x;
                    let dz = oz - tnt.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !tnt.dead && !other_dead && in_tracking_range(dx, dz, range) && chunk_ready {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_tnt_spawn_packets(tnt) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        for cart in minecarts.iter() {
            let entity_id = cart.entity_id;
            let target_chunk = cart.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let range = 160.0f64.min(self.tracking_range);
            if let Some(entry) = self.minecart_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - cart.x;
                    let dz = oz - cart.z;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !cart.dead && !other_dead && in_tracking_range(dx, dz, range) && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_minecart_spawn_packets(cart) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        for painting in paintings.iter() {
            let entity_id = painting.entity_id;
            let target_chunk = painting.chunk_pos();
            // The radius of the entity is limited by the distance of the view.
            let range = 160.0f64.min(self.tracking_range);
            if let Some(entry) = self.painting_entries.get_mut(&entity_id) {
                for &(other_id, ox, _oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - painting.x as f64;
                    let dz = oz - painting.z as f64;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !painting.dead
                        && !other_dead
                        && in_tracking_range(dx, dz, range)
                        && chunk_ready
                    {
                        if entry.tracked_players.insert(other_id) {
                            for packet in entry.make_painting_spawn_packets(painting) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        // 2. Movement, rotation, velocity, metadata, and equipment updates for players
        for player in players.iter_mut() {
            if player.dead {
                continue;
            }
            let entity_id = player.entity_id;

            if player.velocity_changed {
                player.velocity_changed = false;
                let mot_x = (player.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16;
                let mot_y = (player.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16;
                let mot_z = (player.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16;
                let vel_packet = Clientbound::EntityVelocity(EntityVelocity {
                    entity_id,
                    mot_x,
                    mot_y,
                    mot_z,
                });

                outbound.push(OutboundTrackingPacket {
                    recipient_id: entity_id,
                    packet: vel_packet.clone(),
                });

                if let Some(entry) = self.entries.get(&entity_id) {
                    for &viewer_id in &entry.tracked_players {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: viewer_id,
                            packet: vel_packet.clone(),
                        });
                    }
                }
            }

            let cur_flags = player.metadata_flags();
            if let Some(entry) = self.entries.get_mut(&entity_id) {
                if cur_flags != entry.last_meta_flags {
                    entry.last_meta_flags = cur_flags;
                    let meta_pkt = Clientbound::EntityMetadata(EntityMetadata {
                        entity_id,
                        metadata: vec![0, cur_flags, 127],
                    });
                    for &viewer_id in &entry.tracked_players {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: viewer_id,
                            packet: meta_pkt.clone(),
                        });
                    }
                }
            }

            let equipment_diff = player.equipment_changed();
            if !equipment_diff.is_empty() {
                player.sync_equipment_cache();
                if let Some(entry) = self.entries.get(&entity_id) {
                    for (slot, stack) in equipment_diff {
                        let eq_packet = Clientbound::EntityEquipment(EntityEquipment {
                            entity_id,
                            slot,
                            item_id: stack.map(|s| s.id).unwrap_or(-1),
                            damage: stack.map(|s| s.damage).unwrap_or(0),
                        });
                        for &viewer_id in &entry.tracked_players {
                            outbound.push(OutboundTrackingPacket {
                                recipient_id: viewer_id,
                                packet: eq_packet.clone(),
                            });
                        }
                    }
                }
            }

            if let Some(entry) = self.entries.get_mut(&entity_id) {
                Self::emit_pos_rot_update(
                    entry,
                    entity_id,
                    player.x,
                    player.y,
                    player.z,
                    player.yaw,
                    player.pitch,
                    player.air_borne,
                    &mut outbound,
                );
            }
        }

        // 3. Movement updates for dropped items
        for item in items.iter_mut() {
            let entity_id = item.entity_id;
            if let Some(entry) = self.item_entries.get_mut(&entity_id) {
                Self::emit_pos_only_update(
                    entry,
                    entity_id,
                    item.x,
                    item.y,
                    item.z,
                    !item.on_ground,
                    &mut outbound,
                );
            }
        }

        // 4. Movement & velocity updates for boats
        for boat in boats.iter_mut() {
            let entity_id = boat.entity_id;
            if let Some(entry) = self.boat_entries.get_mut(&entity_id) {
                Self::emit_vel_and_move_update(
                    entry,
                    entity_id,
                    boat.x,
                    boat.y,
                    boat.z,
                    boat.yaw,
                    boat.pitch,
                    boat.mot_x,
                    boat.mot_y,
                    boat.mot_z,
                    !boat.on_ground,
                    &mut outbound,
                );
            }
        }

        // 5. Movement, velocity, and metadata updates for mobs
        for mob in mobs.iter_mut() {
            let entity_id = mob.entity_id;
            if let Some(entry) = self.mob_entries.get_mut(&entity_id) {
                let meta_bytes = mob.encode_metadata();
                if meta_bytes != entry.last_meta_bytes {
                    entry.last_meta_bytes = meta_bytes.clone();
                    let meta_pkt = Clientbound::EntityMetadata(EntityMetadata {
                        entity_id,
                        metadata: meta_bytes,
                    });
                    for &viewer_id in &entry.tracked_players {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: viewer_id,
                            packet: meta_pkt.clone(),
                        });
                    }
                }

                if mob.velocity_changed {
                    mob.velocity_changed = false;
                    let vp = Clientbound::EntityVelocity(EntityVelocity {
                        entity_id,
                        mot_x: (mob.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16,
                        mot_y: (mob.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16,
                        mot_z: (mob.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16,
                    });
                    for &viewer_id in &entry.tracked_players {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: viewer_id,
                            packet: vp.clone(),
                        });
                    }
                }

                Self::emit_pos_rot_update(
                    entry,
                    entity_id,
                    mob.x,
                    mob.y,
                    mob.z,
                    mob.yaw,
                    mob.pitch,
                    mob.air_borne,
                    &mut outbound,
                );
                // Java `EntityTrackerEntry.update()`: the flag of the air is
                // cleared after the packet of the position.
                mob.air_borne = false;
            }
        }

        // 6. Movement updates for projectiles, primed TNTs, minecarts
        for proj in projectiles.iter_mut() {
            if let Some(entry) = self.projectile_entries.get_mut(&proj.entity_id) {
                Self::emit_pos_only_update(
                    entry,
                    proj.entity_id,
                    proj.x,
                    proj.y,
                    proj.z,
                    !proj.in_ground,
                    &mut outbound,
                );
            }
        }
        for tnt in tnts.iter_mut() {
            if let Some(entry) = self.tnt_entries.get_mut(&tnt.entity_id) {
                Self::emit_pos_only_update(
                    entry,
                    tnt.entity_id,
                    tnt.x,
                    tnt.y,
                    tnt.z,
                    !tnt.on_ground,
                    &mut outbound,
                );
            }
        }
        for cart in minecarts.iter_mut() {
            if let Some(entry) = self.minecart_entries.get_mut(&cart.entity_id) {
                Self::emit_vel_and_move_update(
                    entry,
                    cart.entity_id,
                    cart.x,
                    cart.y,
                    cart.z,
                    cart.yaw,
                    cart.pitch,
                    cart.mot_x,
                    cart.mot_y,
                    cart.mot_z,
                    !cart.on_ground,
                    &mut outbound,
                );
            }
        }

        outbound
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_pos_rot_update(
        entry: &mut EntityTrackerEntry,
        entity_id: i32,
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        airborne: bool,
        outbound: &mut Vec<OutboundTrackingPacket>,
    ) {
        // Java `EntityTrackerEntry.update()`: the packet is sent every
        // `update_interval` ticks and always when the entity is in the air.
        if !Self::update_due(entry, airborne) {
            return;
        }
        entry.teleport_ticks += 1;
        let new_x = (x * 32.0).floor() as i32;
        let new_y = (y * 32.0).floor() as i32;
        let new_z = (z * 32.0).floor() as i32;
        let new_yaw = pack_angle(yaw);
        let new_pitch = pack_angle(pitch);

        let dx = new_x - entry.encoded_x;
        let dy = new_y - entry.encoded_y;
        let dz = new_z - entry.encoded_z;

        // The difference of the angles is not wrapped: the reference keeps
        // the angles as integers and takes the absolute value of their
        // difference (`EntityTrackerEntry.update()`).
        let dyaw = i32::from(new_yaw) - i32::from(entry.encoded_yaw);
        let dpitch = i32::from(new_pitch) - i32::from(entry.encoded_pitch);

        // The thresholds of the reference are one: any change of the
        // position or of the rotation is sent.
        let pos_changed = dx != 0 || dy != 0 || dz != 0;
        let rot_changed = dyaw.abs() >= 1 || dpitch.abs() >= 1;

        if pos_changed || rot_changed {
            let move_packet = if (-128..=127).contains(&dx)
                && (-128..=127).contains(&dy)
                && (-128..=127).contains(&dz)
                && entry.teleport_ticks < 400
            {
                if pos_changed && rot_changed {
                    Clientbound::RelEntityMoveLook(RelEntityMoveLook {
                        entity_id,
                        dx: dx as i8,
                        dy: dy as i8,
                        dz: dz as i8,
                        yaw: new_yaw,
                        pitch: new_pitch,
                    })
                } else if pos_changed {
                    Clientbound::RelEntityMove(RelEntityMove {
                        entity_id,
                        dx: dx as i8,
                        dy: dy as i8,
                        dz: dz as i8,
                    })
                } else {
                    Clientbound::EntityLook(EntityLook {
                        entity_id,
                        yaw: new_yaw,
                        pitch: new_pitch,
                    })
                }
            } else {
                entry.teleport_ticks = 0;
                Clientbound::EntityTeleport(EntityTeleport {
                    entity_id,
                    x: new_x,
                    y: new_y,
                    z: new_z,
                    yaw: new_yaw,
                    pitch: new_pitch,
                })
            };

            entry.encoded_x = new_x;
            entry.encoded_y = new_y;
            entry.encoded_z = new_z;
            entry.encoded_yaw = new_yaw;
            entry.encoded_pitch = new_pitch;

            for &viewer_id in &entry.tracked_players {
                outbound.push(OutboundTrackingPacket {
                    recipient_id: viewer_id,
                    packet: move_packet.clone(),
                });
            }
        }
    }

    /// Java `EntityTrackerEntry.update()`: `++this.l % this.c == 0` — the
    /// packet of the movement is sent every `update_interval` ticks; the
    /// entity in the air is always sent.
    fn update_due(entry: &mut EntityTrackerEntry, airborne: bool) -> bool {
        entry.update_counter = entry.update_counter.wrapping_add(1);
        airborne || entry.update_counter % entry.update_interval == 0
    }

    fn emit_pos_only_update(
        entry: &mut EntityTrackerEntry,
        entity_id: i32,
        x: f64,
        y: f64,
        z: f64,
        airborne: bool,
        outbound: &mut Vec<OutboundTrackingPacket>,
    ) {
        // Java `EntityTrackerEntry.update()`: the packet is sent every
        // `update_interval` ticks and always when the entity is in the air.
        if !Self::update_due(entry, airborne) {
            return;
        }
        entry.teleport_ticks += 1;
        let new_x = (x * 32.0).floor() as i32;
        let new_y = (y * 32.0).floor() as i32;
        let new_z = (z * 32.0).floor() as i32;
        let dx = new_x - entry.encoded_x;
        let dy = new_y - entry.encoded_y;
        let dz = new_z - entry.encoded_z;

        if dx != 0 || dy != 0 || dz != 0 {
            let move_packet = if (-128..=127).contains(&dx)
                && (-128..=127).contains(&dy)
                && (-128..=127).contains(&dz)
                && entry.teleport_ticks < 400
            {
                Clientbound::RelEntityMove(RelEntityMove {
                    entity_id,
                    dx: dx as i8,
                    dy: dy as i8,
                    dz: dz as i8,
                })
            } else {
                entry.teleport_ticks = 0;
                Clientbound::EntityTeleport(EntityTeleport {
                    entity_id,
                    x: new_x,
                    y: new_y,
                    z: new_z,
                    yaw: 0,
                    pitch: 0,
                })
            };
            entry.encoded_x = new_x;
            entry.encoded_y = new_y;
            entry.encoded_z = new_z;
            for &viewer_id in &entry.tracked_players {
                outbound.push(OutboundTrackingPacket {
                    recipient_id: viewer_id,
                    packet: move_packet.clone(),
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn emit_vel_and_move_update(
        entry: &mut EntityTrackerEntry,
        entity_id: i32,
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        mot_x: f64,
        mot_y: f64,
        mot_z: f64,
        airborne: bool,
        outbound: &mut Vec<OutboundTrackingPacket>,
    ) {
        // Java `EntityTrackerEntry.update()`: the packet is sent every
        // `update_interval` ticks and always when the entity is in the air.
        if !Self::update_due(entry, airborne) {
            return;
        }
        entry.teleport_ticks += 1;
        let new_x = (x * 32.0).floor() as i32;
        let new_y = (y * 32.0).floor() as i32;
        let new_z = (z * 32.0).floor() as i32;
        let new_yaw = pack_angle(yaw);
        let new_pitch = pack_angle(pitch);

        let dx = new_x - entry.encoded_x;
        let dy = new_y - entry.encoded_y;
        let dz = new_z - entry.encoded_z;

        // EntityTrackerEntry.java:82-85 — needsPositionUpdate is always true for EntityBoat & EntityMinecart
        let needs_pos = true;
        let needs_rot = new_yaw != entry.encoded_yaw || new_pitch != entry.encoded_pitch;

        if needs_pos {
            entry.encoded_x = new_x;
            entry.encoded_y = new_y;
            entry.encoded_z = new_z;
        }
        if needs_rot {
            entry.encoded_yaw = new_yaw;
            entry.encoded_pitch = new_pitch;
        }

        let move_packet = if (-128..128).contains(&dx)
            && (-128..128).contains(&dy)
            && (-128..128).contains(&dz)
            && entry.teleport_ticks <= 400
        {
            if needs_pos && needs_rot {
                Clientbound::RelEntityMoveLook(RelEntityMoveLook {
                    entity_id,
                    dx: dx as i8,
                    dy: dy as i8,
                    dz: dz as i8,
                    yaw: new_yaw,
                    pitch: new_pitch,
                })
            } else {
                Clientbound::RelEntityMove(RelEntityMove {
                    entity_id,
                    dx: dx as i8,
                    dy: dy as i8,
                    dz: dz as i8,
                })
            }
        } else {
            entry.teleport_ticks = 0;
            Clientbound::EntityTeleport(EntityTeleport {
                entity_id,
                x: new_x,
                y: new_y,
                z: new_z,
                yaw: new_yaw,
                pitch: new_pitch,
            })
        };

        // EntityTrackerEntry.java:126-143 — send Packet28EntityVelocity before move packet
        let dvx = mot_x - entry.last_mot_x;
        let dvy = mot_y - entry.last_mot_y;
        let dvz = mot_z - entry.last_mot_z;
        let vel_delta_sq = dvx * dvx + dvy * dvy + dvz * dvz;
        if vel_delta_sq > 0.0004
            || (vel_delta_sq > 0.0 && mot_x == 0.0 && mot_y == 0.0 && mot_z == 0.0)
        {
            entry.last_mot_x = mot_x;
            entry.last_mot_y = mot_y;
            entry.last_mot_z = mot_z;
            let vp = Clientbound::EntityVelocity(EntityVelocity {
                entity_id,
                mot_x: (mot_x.clamp(-3.9, 3.9) * 8000.0) as i16,
                mot_y: (mot_y.clamp(-3.9, 3.9) * 8000.0) as i16,
                mot_z: (mot_z.clamp(-3.9, 3.9) * 8000.0) as i16,
            });
            for &viewer_id in &entry.tracked_players {
                outbound.push(OutboundTrackingPacket {
                    recipient_id: viewer_id,
                    packet: vp.clone(),
                });
            }
        }

        for &viewer_id in &entry.tracked_players {
            outbound.push(OutboundTrackingPacket {
                recipient_id: viewer_id,
                packet: move_packet.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The radius of the tracking is a square in XZ.
    ///
    /// Java `EntityTrackerEntry.b(EntityPlayer)`: the height of the entity
    /// is not taken into account, only the difference of the coordinates X
    /// and Z.
    #[test]
    fn the_radius_of_the_tracking_is_a_square_in_xz() {
        let mut tracker = EntityTracker::new(10);
        let player = Player::new(1, 1, "Player1".to_string(), 0.0, 64.0, 0.0);
        // A mob far above the player, but not far in XZ.
        let mut rng = crate::util::rng::JavaRandom::new(1);
        let mob = Mob::new(
            2,
            crate::entity::mob::MobType::Pig,
            0.0,
            200.0,
            0.0,
            &mut rng,
        );
        tracker.track_player(&player);
        let players = vec![player];
        let packets = tracker.track_mob_and_scan(&mob, &players);
        assert!(
            !packets.is_empty(),
            "the mob is tracked: the height is not taken into account"
        );
    }

    /// The radius of the tracking is limited by the distance of the view.
    ///
    /// Java `EntityTracker.a(Entity, int, int, boolean)`: the radius of the
    /// entity is limited by `viewDistance * 16 - 16`.
    #[test]
    fn the_radius_of_the_tracking_is_limited_by_the_view_distance() {
        // The view distance of two chunks: the radius is 16 blocks.
        let tracker = EntityTracker::new(2);
        assert_eq!(tracker.tracking_range, 16.0);
        // The view distance of ten chunks: the radius is 144 blocks.
        let tracker = EntityTracker::new(10);
        assert_eq!(tracker.tracking_range, 144.0);
    }

    /// The packet of the position is sent every `update_interval` ticks.
    ///
    /// Java `EntityTrackerEntry.update()`: `++this.l % this.c == 0` — the
    /// animals are updated every three ticks, the players every two, the
    /// items every twenty.
    #[test]
    fn the_packet_of_the_position_is_sent_every_interval() {
        let mut tracker = EntityTracker::new(10);
        let player1 = Player::new(1, 1, "Player1".to_string(), 0.0, 64.0, 0.0);
        let player2 = Player::new(2, 2, "Player2".to_string(), 10.0, 64.0, 0.0);
        tracker.track_player(&player1);
        tracker.track_player(&player2);
        let mut players = vec![player1, player2];
        // The first tick: the players see each other.
        tracker.tick(&mut players);
        // The player that walks is not pushed, the packet of its position is
        // sent every two ticks.
        let mut sent = 0;
        for tick in 0..6 {
            players[0].x += 0.5;
            let packets = tracker.tick(&mut players);
            let updates = packets
                .iter()
                .filter(|p| {
                    p.recipient_id == 2
                        && matches!(
                            p.packet,
                            Clientbound::RelEntityMove(_)
                                | Clientbound::RelEntityMoveLook(_)
                                | Clientbound::EntityTeleport(_)
                        )
                })
                .count();
            if updates > 0 {
                sent += 1;
            }
            // The counter of the tracker was raised once before the loop, so
            // the first packet of the loop is sent at its first tick.
            assert_eq!(
                sent,
                tick / 2 + 1,
                "the packet of the position is sent every two ticks"
            );
        }
    }

    /// The rotation is sent when it changes by one.
    ///
    /// Java `EntityTrackerEntry.update()`: the threshold of the rotation is
    /// one.
    #[test]
    fn the_rotation_is_sent_when_it_changes_by_one() {
        let mut tracker = EntityTracker::new(10);
        let player1 = Player::new(1, 1, "Player1".to_string(), 0.0, 64.0, 0.0);
        let player2 = Player::new(2, 2, "Player2".to_string(), 10.0, 64.0, 0.0);
        tracker.track_player(&player1);
        tracker.track_player(&player2);
        let mut players = vec![player1, player2];
        // The first tick: the players see each other.
        tracker.tick(&mut players);
        // A rotation of one unit (of the 256 of a circle).
        players[0].yaw = 360.0 / 256.0;
        let packets = tracker.tick(&mut players);
        assert!(
            packets.iter().any(|p| {
                p.recipient_id == 2
                    && matches!(
                        p.packet,
                        Clientbound::EntityLook(_) | Clientbound::RelEntityMoveLook(_)
                    )
            }),
            "the rotation of one unit is sent"
        );
    }

    #[test]
    fn tracker_spawns_and_tracks_in_range() {
        let mut tracker = EntityTracker::new(10);
        let p1 = Player::new(1, 1, "Player1".to_string(), 0.0, 64.0, 0.0);
        let p2 = Player::new(2, 2, "Player2".to_string(), 10.0, 64.0, 0.0);

        tracker.track_player(&p1);
        tracker.track_player(&p2);

        let mut players = vec![p1, p2];
        let packets = tracker.tick(&mut players);

        assert!(!packets.is_empty());
        let to_p1: Vec<_> = packets.iter().filter(|p| p.recipient_id == 1).collect();
        let to_p2: Vec<_> = packets.iter().filter(|p| p.recipient_id == 2).collect();
        assert!(!to_p1.is_empty());
        assert!(!to_p2.is_empty());
    }

    #[test]
    fn tracker_emits_rel_move_for_small_delta() {
        let mut tracker = EntityTracker::new(10);
        let p1 = Player::new(1, 1, "Player1".to_string(), 0.0, 64.0, 0.0);
        let mut p2 = Player::new(2, 2, "Player2".to_string(), 10.0, 64.0, 0.0);

        tracker.track_player(&p1);
        tracker.track_player(&p2);

        let mut players = vec![p1.clone(), p2.clone()];
        let _ = tracker.tick(&mut players);

        p2.x += 0.5;
        let mut players2 = vec![p1, p2];
        let move_packets = tracker.tick(&mut players2);

        let p2_moves_to_p1: Vec<_> = move_packets
            .iter()
            .filter(|p| p.recipient_id == 1)
            .collect();
        assert_eq!(p2_moves_to_p1.len(), 1);
        assert!(matches!(
            p2_moves_to_p1[0].packet,
            Clientbound::RelEntityMove(_)
        ));
    }

    /// The difference of the angles of a half turn does not overflow.
    ///
    /// Java `Math.abs` promotes the byte of the difference to an `int`
    /// before taking the absolute value, so the value `-128` has an
    /// absolute value of `128`. The port of the `i8::abs` panicked with
    /// "attempt to negate with overflow" when a mob turned by a half turn.
    #[test]
    fn the_difference_of_the_angles_of_a_half_turn_does_not_overflow() {
        let mut tracker = EntityTracker::new(10);
        let p1 = Player::new(1, 1, "Player1".to_string(), 0.0, 64.0, 0.0);
        let mut p2 = Player::new(2, 2, "Player2".to_string(), 10.0, 64.0, 0.0);

        tracker.track_player(&p1);
        tracker.track_player(&p2);

        let mut players = vec![p1.clone(), p2.clone()];
        let _ = tracker.tick(&mut players);

        // The yaw of the second player turns by a half turn: the encoded
        // angle goes from 0 to 128, the difference of the bytes is -128.
        p2.yaw = 180.0;
        let mut players2 = vec![p1, p2];
        let packets = tracker.tick(&mut players2);
        let to_p1: Vec<_> = packets.iter().filter(|p| p.recipient_id == 1).collect();
        assert_eq!(to_p1.len(), 1);
        assert!(matches!(to_p1[0].packet, Clientbound::EntityLook(_)));
    }

    #[test]
    fn pack_angle_negative_and_wrap() {
        assert_eq!(pack_angle(0.0), 0);
        assert_eq!(pack_angle(90.0), 64);
        assert_eq!(pack_angle(180.0), 128);
        assert_eq!(pack_angle(270.0), 192);
        assert_eq!(pack_angle(360.0), 0);
        assert_eq!(pack_angle(-90.0), 192);
        assert_eq!(pack_angle(-180.0), 128);
        assert_eq!(pack_angle(-45.0), 224);
        assert_eq!(pack_angle(-360.0), 0);
    }
}
