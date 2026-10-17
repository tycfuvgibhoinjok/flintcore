use std::collections::{HashMap, HashSet};

use crate::entity::player::Player;
use crate::net::packets::{
    Clientbound, DestroyEntity, EntityEquipment, EntityLook, EntityMetadata, EntityTeleport,
    EntityVelocity, NamedEntitySpawn, RelEntityMove, RelEntityMoveLook,
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
            teleport_ticks: 0,
            tracked_players: HashSet::new(),
            tracking_range,
        }
    }

    pub fn make_spawn_packets(&self, player: &Player) -> Vec<Clientbound> {
        let mut packets = Vec::with_capacity(7);
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

        // Metadata: flags (bit 0 = fire, bit 1 = sneak)
        let mut flags = 0u8;
        if player.fire_ticks > 0 {
            flags |= 0x01;
        }
        if player.sneaking {
            flags |= 0x02;
        }
        packets.push(Clientbound::EntityMetadata(EntityMetadata {
            entity_id: player.entity_id,
            metadata: vec![0, flags, 127],
        }));

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
}

pub struct OutboundTrackingPacket {
    pub recipient_id: i32,
    pub packet: Clientbound,
}

#[derive(Debug, Default)]
pub struct EntityTracker {
    entries: HashMap<i32, EntityTrackerEntry>,
    tracking_range: f64,
}

impl EntityTracker {
    pub fn new(view_distance: i32) -> Self {
        let tracking_range = ((view_distance * 16 - 16).max(32)) as f64;
        Self {
            entries: HashMap::new(),
            tracking_range,
        }
    }

    pub fn track_player(&mut self, player: &Player) {
        let entry = EntityTrackerEntry::new(player, self.tracking_range);
        self.entries.insert(player.entity_id, entry);
    }

    pub fn untrack_player(&mut self, entity_id: i32) -> Vec<OutboundTrackingPacket> {
        let mut packets = Vec::new();
        if let Some(entry) = self.entries.remove(&entity_id) {
            let destroy = Clientbound::DestroyEntity(DestroyEntity { entity_id });
            for &viewer_id in &entry.tracked_players {
                packets.push(OutboundTrackingPacket {
                    recipient_id: viewer_id,
                    packet: destroy.clone(),
                });
            }
        }

        // Also remove this player from everyone else's tracked_players
        for entry in self.entries.values_mut() {
            entry.tracked_players.remove(&entity_id);
        }

        packets
    }

    pub fn tick(&mut self, players: &mut [Player]) -> Vec<OutboundTrackingPacket> {
        let mut outbound = Vec::new();
        let range_sq = self.tracking_range * self.tracking_range;

        // 1. Proximity scan: who sees who
        let player_positions: Vec<(i32, f64, f64, f64)> = players
            .iter()
            .map(|p| (p.entity_id, p.x, p.y, p.z))
            .collect();

        for player in players.iter() {
            let entity_id = player.entity_id;
            if let Some(entry) = self.entries.get_mut(&entity_id) {
                for &(other_id, ox, oy, oz) in &player_positions {
                    if other_id == entity_id {
                        continue;
                    }

                    let dx = ox - player.x;
                    let dy = oy - player.y;
                    let dz = oz - player.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;

                    if dist_sq <= range_sq {
                        if entry.tracked_players.insert(other_id) {
                            // other_id newly observes player: send spawn packets to other_id
                            for packet in entry.make_spawn_packets(player) {
                                outbound.push(OutboundTrackingPacket {
                                    recipient_id: other_id,
                                    packet,
                                });
                            }
                        }
                    } else if entry.tracked_players.remove(&other_id) {
                        // other_id lost sight: destroy entity for other_id
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: other_id,
                            packet: Clientbound::DestroyEntity(DestroyEntity { entity_id }),
                        });
                    }
                }
            }
        }

        // 2. Movement, rotation, velocity, and equipment updates
        for player in players.iter_mut() {
            let entity_id = player.entity_id;

            // Velocity update
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

                // Velocity sent to player themselves
                outbound.push(OutboundTrackingPacket {
                    recipient_id: entity_id,
                    packet: vel_packet.clone(),
                });

                // And to all observers
                if let Some(entry) = self.entries.get(&entity_id) {
                    for &viewer_id in &entry.tracked_players {
                        outbound.push(OutboundTrackingPacket {
                            recipient_id: viewer_id,
                            packet: vel_packet.clone(),
                        });
                    }
                }
            }

            // Equipment updates
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

            // Movement update
            if let Some(entry) = self.entries.get_mut(&entity_id) {
                entry.teleport_ticks += 1;

                let new_x = (player.x * 32.0).floor() as i32;
                let new_y = (player.y * 32.0).floor() as i32;
                let new_z = (player.z * 32.0).floor() as i32;
                let new_yaw = pack_angle(player.yaw);
                let new_pitch = pack_angle(player.pitch);

                let dx = new_x - entry.encoded_x;
                let dy = new_y - entry.encoded_y;
                let dz = new_z - entry.encoded_z;

                let dyaw = (new_yaw.wrapping_sub(entry.encoded_yaw)) as i8;
                let dpitch = (new_pitch.wrapping_sub(entry.encoded_pitch)) as i8;

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
        }

        outbound
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

        // Player 1 and Player 2 should see each other's spawn and equipment packets
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
        let _ = tracker.tick(&mut players); // initial spawn

        // Move Player 2 by 0.5 blocks (16 units in 1/32)
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

    #[test]
    fn pack_angle_negative_and_wrap() {
        assert_eq!(pack_angle(0.0), 0);
        assert_eq!(pack_angle(90.0), 64);
        assert_eq!(pack_angle(180.0), 128);
        assert_eq!(pack_angle(270.0), 192);
        assert_eq!(pack_angle(360.0), 0);
        // Negative angles must wrap correctly rather than saturating to 0
        assert_eq!(pack_angle(-90.0), 192);
        assert_eq!(pack_angle(-180.0), 128);
        assert_eq!(pack_angle(-45.0), 224);
        assert_eq!(pack_angle(-360.0), 0);
    }
}
