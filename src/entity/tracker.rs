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
        }
    }

    pub fn new_pos_entity(entity_id: i32, x: f64, y: f64, z: f64, range: f64) -> Self {
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
    tracking_range: f64,
}

impl EntityTracker {
    pub fn new(view_distance: i32) -> Self {
        let tracking_range = ((view_distance * 16 - 16).max(32)) as f64;
        Self {
            entries: HashMap::new(),
            item_entries: HashMap::new(),
            boat_entries: HashMap::new(),
            mob_entries: HashMap::new(),
            projectile_entries: HashMap::new(),
            tnt_entries: HashMap::new(),
            minecart_entries: HashMap::new(),
            painting_entries: HashMap::new(),
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
        let item_range_sq = 64.0 * 64.0;
        let target_chunk = item.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - item.x;
            let dy = p.y - item.y;
            let dz = p.z - item.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= item_range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id)
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
        let boat_range_sq = 160.0 * 160.0;
        let target_chunk = boat.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - boat.x;
            let dy = p.y - boat.y;
            let dz = p.z - boat.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= boat_range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id)
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
        let range_sq = 160.0 * 160.0;
        let target_chunk = mob.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - mob.x;
            let dy = p.y - mob.y;
            let dz = p.z - mob.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id) {
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
        let mut entry =
            EntityTrackerEntry::new_pos_entity(proj.entity_id, proj.x, proj.y, proj.z, 64.0);
        let range_sq = 64.0 * 64.0;
        let target_chunk = proj.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - proj.x;
            let dy = p.y - proj.y;
            let dz = p.z - proj.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id) {
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
            EntityTrackerEntry::new_pos_entity(tnt.entity_id, tnt.x, tnt.y, tnt.z, 160.0);
        let range_sq = 160.0 * 160.0;
        let target_chunk = tnt.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - tnt.x;
            let dy = p.y - tnt.y;
            let dz = p.z - tnt.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id) {
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
            EntityTrackerEntry::new_pos_entity(cart.entity_id, cart.x, cart.y, cart.z, 160.0);
        let range_sq = 160.0 * 160.0;
        let target_chunk = cart.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - cart.x;
            let dy = p.y - cart.y;
            let dz = p.z - cart.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id) {
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
        let mut entry = EntityTrackerEntry::new_pos_entity(
            painting.entity_id,
            painting.x as f64,
            painting.y as f64,
            painting.z as f64,
            160.0,
        );
        let range_sq = 160.0 * 160.0;
        let target_chunk = painting.chunk_pos();

        for p in players {
            if p.dead {
                continue;
            }
            let dx = p.x - painting.x as f64;
            let dy = p.y - painting.y as f64;
            let dz = p.z - painting.z as f64;
            let dist_sq = dx * dx + dy * dy + dz * dz;
            let chunk_ready = p.sent_chunks.is_empty() || p.sent_chunks.contains(&target_chunk);
            if dist_sq <= range_sq && chunk_ready && entry.tracked_players.insert(p.entity_id) {
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
        let range_sq = self.tracking_range * self.tracking_range;

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
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    if other_id == entity_id {
                        continue;
                    }
                    let dx = ox - player.x;
                    let dy = oy - player.y;
                    let dz = oz - player.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !player.dead && !other_dead && dist_sq <= range_sq && chunk_ready {
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
            let item_range_sq = 64.0 * 64.0;
            if let Some(entry) = self.item_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - item.x;
                    let dy = oy - item.y;
                    let dz = oz - item.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !item.dead && !other_dead && dist_sq <= item_range_sq && chunk_ready {
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
            let boat_range_sq = 160.0 * 160.0;
            if let Some(entry) = self.boat_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - boat.x;
                    let dy = oy - boat.y;
                    let dz = oz - boat.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !boat.dead && !other_dead && dist_sq <= boat_range_sq && chunk_ready {
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
            let mob_range_sq = 160.0 * 160.0;
            if let Some(entry) = self.mob_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - mob.x;
                    let dy = oy - mob.y;
                    let dz = oz - mob.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);

                    if !mob.dead && !other_dead && dist_sq <= mob_range_sq && chunk_ready {
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
            let r_sq = 64.0 * 64.0;
            if let Some(entry) = self.projectile_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - proj.x;
                    let dy = oy - proj.y;
                    let dz = oz - proj.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !proj.dead && !other_dead && dist_sq <= r_sq && chunk_ready {
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
            let r_sq = 160.0 * 160.0;
            if let Some(entry) = self.tnt_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - tnt.x;
                    let dy = oy - tnt.y;
                    let dz = oz - tnt.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !tnt.dead && !other_dead && dist_sq <= r_sq && chunk_ready {
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
            let r_sq = 160.0 * 160.0;
            if let Some(entry) = self.minecart_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - cart.x;
                    let dy = oy - cart.y;
                    let dz = oz - cart.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !cart.dead && !other_dead && dist_sq <= r_sq && chunk_ready {
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
            let r_sq = 160.0 * 160.0;
            if let Some(entry) = self.painting_entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz, other_dead, other_sent) in &observers {
                    let dx = ox - painting.x as f64;
                    let dy = oy - painting.y as f64;
                    let dz = oz - painting.z as f64;
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    let chunk_ready = other_sent.is_empty() || other_sent.contains(&target_chunk);
                    if !painting.dead && !other_dead && dist_sq <= r_sq && chunk_ready {
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
                    &mut outbound,
                );
            }
        }

        // 3. Movement updates for dropped items
        for item in items.iter_mut() {
            let entity_id = item.entity_id;
            if let Some(entry) = self.item_entries.get_mut(&entity_id) {
                Self::emit_pos_only_update(entry, entity_id, item.x, item.y, item.z, &mut outbound);
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
                    &mut outbound,
                );
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
        outbound: &mut Vec<OutboundTrackingPacket>,
    ) {
        entry.teleport_ticks += 1;
        let new_x = (x * 32.0).floor() as i32;
        let new_y = (y * 32.0).floor() as i32;
        let new_z = (z * 32.0).floor() as i32;
        let new_yaw = pack_angle(yaw);
        let new_pitch = pack_angle(pitch);

        let dx = new_x - entry.encoded_x;
        let dy = new_y - entry.encoded_y;
        let dz = new_z - entry.encoded_z;

        // The difference of the angles is a byte (the packet carries a
        // byte), but the absolute value is taken on an `i32`: Java promotes
        // the byte of the difference to an `int` before `Math.abs`, so the
        // value `-128` has an absolute value of `128` and not an overflow.
        let dyaw = i32::from((new_yaw.wrapping_sub(entry.encoded_yaw)) as i8);
        let dpitch = i32::from((new_pitch.wrapping_sub(entry.encoded_pitch)) as i8);

        let pos_changed = dx != 0 || dy != 0 || dz != 0;
        let rot_changed = if pos_changed {
            new_yaw != entry.encoded_yaw || new_pitch != entry.encoded_pitch
        } else {
            dyaw.abs() >= 4 || dpitch.abs() >= 4
        };

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

    fn emit_pos_only_update(
        entry: &mut EntityTrackerEntry,
        entity_id: i32,
        x: f64,
        y: f64,
        z: f64,
        outbound: &mut Vec<OutboundTrackingPacket>,
    ) {
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
        outbound: &mut Vec<OutboundTrackingPacket>,
    ) {
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
