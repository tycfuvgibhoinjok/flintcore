use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::config::Config;
use crate::entity::player::DiggingState;
use crate::entity::store::EntityStore;
use crate::entity::tracker::EntityTracker;
use crate::gameplay::blocks;
use crate::gameplay::commands::CommandMap;
use crate::gameplay::inventory::InventoryPlayer;
use crate::net::connection::ConnectionState;
use crate::net::packets::chat::{is_allowed_char, wrap_text};
use crate::net::packets::{
    BlockChange, Chat, Clientbound, EntityStatus, Handshake, KeepAlive, KickDisconnect, LevelSound,
    Login, PlayerLookMove, PreChunk, Respawn, Serverbound, SetSlot, SpawnPosition, Transaction,
    UpdateHealth, UpdateTime, WindowItems,
};
use crate::net::protocol::PROTOCOL_VERSION;
use crate::net::{ConnId, NetEvent, Outbound};
use crate::plugin::events::{PlayerChatEvent, PluginManager};
use crate::util::{log, Result};
use crate::world::chunk::{ChunkPos, ChunkState};
use crate::world::world::World;

const TICK: Duration = Duration::from_millis(50);
const MAX_CATCHUP: u32 = 4;
const SLEEP_SLICE: Duration = Duration::from_millis(5);

struct Connection {
    sender: mpsc::Sender<Outbound>,
    state: ConnectionState,
    username: Option<String>,
    entity_id: i32,
}

pub struct Server {
    config: Config,
    inbound: mpsc::Receiver<NetEvent>,
    connections: HashMap<ConnId, Connection>,
    running: Arc<AtomicBool>,
    entities: EntityStore,
    tracker: EntityTracker,
    world: World,
    commands: CommandMap,
    plugin_manager: PluginManager,
    current_tick: u64,
}

impl Server {
    pub fn new(
        config: Config,
        inbound: mpsc::Receiver<NetEvent>,
        running: Arc<AtomicBool>,
    ) -> Self {
        let world = World::new(config.world.clone());
        let view_distance = config.world.view_distance;
        Self {
            config,
            inbound,
            connections: HashMap::new(),
            running,
            entities: EntityStore::new(),
            tracker: EntityTracker::new(view_distance),
            world,
            commands: CommandMap::new(),
            plugin_manager: PluginManager::new(),
            current_tick: 0,
        }
    }

    pub fn run(mut self) -> Result<()> {
        let mut last = Instant::now();
        let mut accumulator = Duration::ZERO;
        let mut ticks: u64 = 0;

        while self.running.load(Ordering::Relaxed) {
            let now = Instant::now();
            accumulator += now - last;
            last = now;

            let maximum = TICK * MAX_CATCHUP;
            if accumulator > maximum {
                log::warn("tick backlog exceeded, clamping catch-up");
                accumulator = maximum;
            }

            while accumulator >= TICK {
                self.tick(ticks);
                ticks += 1;
                accumulator -= TICK;
            }

            let mut remaining = TICK - accumulator;
            while remaining > Duration::ZERO && self.running.load(Ordering::Relaxed) {
                let slice = remaining.min(SLEEP_SLICE);
                std::thread::sleep(slice);
                remaining -= slice;
            }
        }

        log::info("simulation thread stopped");
        Ok(())
    }

    fn tick(&mut self, ticks: u64) {
        self.current_tick = ticks;
        self.drain_inbound();
        self.stream_chunks();

        // Tick entity tracking every 2 ticks (mirroring Poseidon EntityTracker)
        if ticks % 2 == 0 {
            let mut players: Vec<_> = self.entities.iter_players().cloned().collect();
            let tracking_packets = self.tracker.tick(&mut players);
            for p in players {
                if let Some(live) = self.entities.get_player_mut(p.entity_id) {
                    live.mot_x = p.mot_x;
                    live.mot_y = p.mot_y;
                    live.mot_z = p.mot_z;
                    live.velocity_changed = p.velocity_changed;
                    live.equipment_cache = p.equipment_cache;
                }
            }

            for outbound in tracking_packets {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }

        // Health synchronization
        let health_updates: Vec<(ConnId, i16)> = self
            .entities
            .iter_players_mut()
            .filter_map(|p| {
                if p.health != p.last_health {
                    p.last_health = p.health;
                    Some((p.conn_id, p.health))
                } else {
                    None
                }
            })
            .collect();

        for (conn_id, health) in health_updates {
            self.send(conn_id, Clientbound::UpdateHealth(UpdateHealth { health }));
        }

        if ticks % 20 == 0 {
            self.broadcast_keep_alive();
        }
        if ticks % 600 == 0 {
            log::info(&format!(
                "tick {ticks}, {} connections, {} chunks loaded, {} players",
                self.connections.len(),
                self.world.chunk_count(),
                self.entities.player_count()
            ));
        }
    }

    fn broadcast_keep_alive(&self) {
        for (&id, conn) in &self.connections {
            if conn.state == ConnectionState::Playing {
                self.send(id, Clientbound::KeepAlive(KeepAlive));
            }
        }
    }

    fn drain_inbound(&mut self) {
        for _ in 0..self.config.network.max_packets_per_tick {
            match self.inbound.try_recv() {
                Ok(event) => self.handle_event(event),
                Err(_) => break,
            }
        }
    }

    fn handle_event(&mut self, event: NetEvent) {
        match event {
            NetEvent::Connected {
                id,
                address,
                sender,
            } => {
                log::info(&format!("connection {id} from {address}"));
                self.connections.insert(
                    id,
                    Connection {
                        sender,
                        state: ConnectionState::Handshaking,
                        username: None,
                        entity_id: 0,
                    },
                );
            }
            NetEvent::Packet { id, packet } => self.handle_packet(id, packet),
            NetEvent::Disconnected { id } => {
                self.remove_and_handle_disconnect(id);
            }
        }
    }

    fn remove_and_handle_disconnect(&mut self, id: ConnId) {
        if let Some(conn) = self.connections.remove(&id) {
            if let Some(player) = self.entities.remove_by_conn(id) {
                // Untrack player and send DestroyEntity to observers
                let untrack_packets = self.tracker.untrack_player(player.entity_id);
                for outbound in untrack_packets {
                    if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                        self.send(target.conn_id, outbound.packet);
                    }
                }

                // Broadcast leave message to all remaining players
                let msg =
                    self.config
                        .server
                        .leave_message
                        .replacen("%player%", &player.username, 1);
                self.broadcast_chat(&msg);
            }
            log::info(&format!(
                "connection {id} ({}) disconnected",
                conn.username.as_deref().unwrap_or("unknown")
            ));
        }
    }

    fn handle_packet(&mut self, id: ConnId, packet: Serverbound) {
        match packet {
            Serverbound::Handshake(handshake) => {
                let server_id = if self.config.server.online_mode {
                    handshake.username.clone()
                } else {
                    "-".to_string()
                };
                self.send(
                    id,
                    Clientbound::Handshake(Handshake {
                        username: server_id,
                    }),
                );
                if let Some(connection) = self.connections.get_mut(&id) {
                    connection.username = Some(handshake.username);
                }
            }
            Serverbound::Login(login) => self.handle_login(id, login),
            Serverbound::KeepAlive(_) => {}
            Serverbound::Chat(chat) => self.handle_chat(id, chat),
            Serverbound::Flying(flying) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.on_ground = flying.on_ground;
                }
            }
            Serverbound::PlayerPosition(pos) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.update_position(
                        pos.x,
                        pos.y,
                        pos.stance,
                        pos.z,
                        player.yaw,
                        player.pitch,
                        pos.on_ground,
                    );
                }
            }
            Serverbound::PlayerLook(look) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.update_position(
                        player.x,
                        player.y,
                        player.stance,
                        player.z,
                        look.yaw,
                        look.pitch,
                        look.on_ground,
                    );
                }
            }
            Serverbound::PlayerLookMove(look_move) => {
                self.handle_player_movement(id, look_move);
            }
            Serverbound::ArmAnimation(anim) => {
                // Broadcast arm animation to all other players in Playing state
                for (&other_id, conn) in &self.connections {
                    if other_id != id && conn.state == ConnectionState::Playing {
                        self.send(other_id, Clientbound::ArmAnimation(anim.clone()));
                    }
                }
            }
            Serverbound::BlockItemSwitch(switch) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.inventory.held_slot = (switch.slot as usize).min(8);
                }
            }
            Serverbound::EntityAction(action) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    match action.action {
                        1 => player.sneaking = true,
                        2 => player.sneaking = false,
                        _ => {}
                    }
                }
            }
            Serverbound::BlockDig(dig) => {
                self.handle_block_dig(id, dig);
            }
            Serverbound::Place(place) => {
                self.handle_place(id, place);
            }
            Serverbound::WindowClick(click) => {
                self.handle_window_click(id, click);
            }
            Serverbound::CloseWindow(close) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    if let Some(cursor) = player.inventory.cursor.take() {
                        player.inventory.add_item(cursor);
                        let window_items = player.inventory.to_window_items();
                        self.send(
                            id,
                            Clientbound::WindowItems(WindowItems {
                                window_id: 0,
                                items: window_items,
                            }),
                        );
                    }
                }
                self.send(id, Clientbound::CloseWindow(close));
            }
            Serverbound::Transaction(_)
            | Serverbound::UseEntity(_)
            | Serverbound::UpdateSign(_) => {}
            Serverbound::Respawn(_) => {
                self.handle_respawn(id);
            }
            Serverbound::KickDisconnect(kick) => {
                log::info(&format!("player disconnected: {}", kick.reason));
                self.remove_and_handle_disconnect(id);
            }
        }
    }

    fn handle_player_movement(
        &mut self,
        id: ConnId,
        look_move: crate::net::packets::PlayerLookMove,
    ) {
        let (fall_damage, entity_id) = {
            if let Some(player) = self.entities.get_by_conn_mut(id) {
                let prev_y = player.y;
                let mut fall_damage = 0i16;

                if !look_move.on_ground {
                    if look_move.y < prev_y {
                        player.fall_distance += (prev_y - look_move.y) as f32;
                    }
                } else {
                    if player.fall_distance > 3.0 {
                        fall_damage = ((player.fall_distance - 3.0).ceil() as i16).max(0);
                    }
                    player.fall_distance = 0.0;
                }

                player.update_position(
                    look_move.x,
                    look_move.y,
                    look_move.stance,
                    look_move.z,
                    look_move.yaw,
                    look_move.pitch,
                    look_move.on_ground,
                );

                (fall_damage, player.entity_id)
            } else {
                return;
            }
        };

        if fall_damage > 0 {
            self.damage_player(entity_id, fall_damage, "fell from a high place");
        }
    }

    fn handle_block_dig(&mut self, id: ConnId, dig: crate::net::packets::BlockDig) {
        let (x, y, z, entity_id) = {
            if let Some(player) = self.entities.get_by_conn(id) {
                // Reach distance check: dist^2 <= 36.0 (6 blocks)
                let dx = player.x - (dig.x as f64 + 0.5);
                let dy = (player.y + 1.62) - (dig.y as f64 + 0.5);
                let dz = player.z - (dig.z as f64 + 0.5);
                if dx * dx + dy * dy + dz * dz > 36.0 {
                    return;
                }
                (dig.x, dig.y as i32, dig.z, player.entity_id)
            } else {
                return;
            }
        };

        // Poseidon: Spawn protection
        let spawn_x = self.config.world.spawn_x;
        let spawn_z = self.config.world.spawn_z;
        let radius = self.config.world.spawn_radius;
        let chebyshev = (x - spawn_x).abs().max((z - spawn_z).abs());
        if chebyshev <= radius {
            let block = self.world.get_block(x, y, z);
            self.send(
                id,
                Clientbound::BlockChange(BlockChange {
                    x,
                    y: y as u8,
                    z,
                    block_type: block,
                    metadata: 0,
                }),
            );
            return;
        }

        if dig.status == 0 {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                player.digging = Some(DiggingState {
                    x,
                    y,
                    z,
                    face: dig.face,
                    start_tick: self.current_tick,
                });
            }
        } else if dig.status == 2 {
            let old_block = self.world.get_block(x, y, z);
            self.world.set_block(x, y, z, blocks::AIR);

            // Level sound 2001: block break effect
            self.broadcast(Clientbound::LevelSound(LevelSound {
                effect_id: 2001,
                x,
                y: y as u8,
                z,
                data: old_block as i32,
            }));

            // BlockChange to all observers
            let change = Clientbound::BlockChange(BlockChange {
                x,
                y: y as u8,
                z,
                block_type: blocks::AIR,
                metadata: 0,
            });
            self.broadcast(change);

            // Damage tool in hand
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                player.digging = None;
                if player.inventory.damage_held_item(1) {
                    // Tool broke: sync slot
                    let slot_idx = 36 + player.inventory.held_slot as i16;
                    self.send(
                        id,
                        Clientbound::SetSlot(SetSlot {
                            window_id: 0,
                            slot: slot_idx,
                            item: None,
                        }),
                    );
                }
            }
        }
    }

    fn handle_place(&mut self, id: ConnId, place: crate::net::packets::Place) {
        if place.direction == 255 {
            // Right-click air: check if food
            let slot_update = if let Some(player) = self.entities.get_by_conn_mut(id) {
                if let Some(mut stack) = player.inventory.item_in_hand() {
                    if stack.is_food() {
                        let heal = stack.food_heal_amount();
                        player.heal(heal);
                        stack.count = stack.count.saturating_sub(1);
                        player.inventory.set_item_in_hand(if stack.count > 0 {
                            Some(stack)
                        } else {
                            None
                        });
                        let slot_idx = 36 + player.inventory.held_slot as i16;
                        let item = InventoryPlayer::to_slot_item(player.inventory.item_in_hand());
                        Some((slot_idx, item))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((slot, item)) = slot_update {
                self.send(
                    id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: 0,
                        slot,
                        item,
                    }),
                );
            }
            return;
        }

        let (tx, ty, tz) = match place.direction {
            0 => (place.x, place.y.saturating_sub(1), place.z),
            1 => (place.x, place.y.saturating_add(1), place.z),
            2 => (place.x, place.y, place.z - 1),
            3 => (place.x, place.y, place.z + 1),
            4 => (place.x - 1, place.y, place.z),
            5 => (place.x + 1, place.y, place.z),
            _ => (place.x, place.y, place.z),
        };

        // Reach distance check
        if let Some(player) = self.entities.get_by_conn(id) {
            let dx = player.x - (tx as f64 + 0.5);
            let dy = (player.y + 1.62) - (ty as f64 + 0.5);
            let dz = player.z - (tz as f64 + 0.5);
            if dx * dx + dy * dy + dz * dz > 36.0 {
                return;
            }
        }

        // Spawn protection
        let spawn_x = self.config.world.spawn_x;
        let spawn_z = self.config.world.spawn_z;
        let radius = self.config.world.spawn_radius;
        let chebyshev = (tx - spawn_x).abs().max((tz - spawn_z).abs());
        if chebyshev <= radius {
            return;
        }

        if place.item_id > 0 && place.item_id < 256 {
            let block_id = place.item_id as u8;
            let meta = place.damage.unwrap_or(0) as u8;
            self.world
                .set_block_and_data(tx, ty as i32, tz, block_id, meta);

            let change = Clientbound::BlockChange(BlockChange {
                x: tx,
                y: ty,
                z: tz,
                block_type: block_id,
                metadata: meta,
            });
            self.broadcast(change);

            // Decrement item in hand
            let slot_update = if let Some(player) = self.entities.get_by_conn_mut(id) {
                if let Some(mut stack) = player.inventory.item_in_hand() {
                    stack.count = stack.count.saturating_sub(1);
                    player.inventory.set_item_in_hand(if stack.count > 0 {
                        Some(stack)
                    } else {
                        None
                    });
                    let slot_idx = 36 + player.inventory.held_slot as i16;
                    let item = InventoryPlayer::to_slot_item(player.inventory.item_in_hand());
                    Some((slot_idx, item))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((slot, item)) = slot_update {
                self.send(
                    id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: 0,
                        slot,
                        item,
                    }),
                );
            }
        }
    }

    fn handle_window_click(&mut self, id: ConnId, click: crate::net::packets::WindowClick) {
        let (clicked_item, cursor_item) = if let Some(player) = self.entities.get_by_conn_mut(id) {
            player
                .inventory
                .handle_window_click(click.slot, click.button, click.shift);
            let item = InventoryPlayer::to_slot_item(player.inventory.get_window_slot(click.slot));
            let cursor = InventoryPlayer::to_slot_item(player.inventory.cursor);
            (item, cursor)
        } else {
            return;
        };

        // Confirm transaction
        self.send(
            id,
            Clientbound::Transaction(Transaction {
                window_id: click.window_id,
                action_number: click.action_number,
                accepted: true,
            }),
        );

        // Sync clicked slot
        self.send(
            id,
            Clientbound::SetSlot(SetSlot {
                window_id: click.window_id as i8,
                slot: click.slot,
                item: clicked_item,
            }),
        );

        // Sync cursor slot
        self.send(
            id,
            Clientbound::SetSlot(SetSlot {
                window_id: -1,
                slot: -1,
                item: cursor_item,
            }),
        );
    }

    fn handle_respawn(&mut self, id: ConnId) {
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();

        // Poseidon double-respawn client hack: send opposite dimension first, then dimension 0
        self.send(id, Clientbound::Respawn(Respawn { dimension: -1 }));
        self.send(id, Clientbound::Respawn(Respawn { dimension: 0 }));

        if let Some(player) = self.entities.get_by_conn_mut(id) {
            player.respawn(spawn_x as f64 + 0.5, spawn_y as f64, spawn_z as f64 + 0.5);
        }

        self.stream_initial_chunks(id);

        self.send(
            id,
            Clientbound::SpawnPosition(SpawnPosition {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
            }),
        );
        self.send(id, Clientbound::UpdateTime(UpdateTime { time: 0 }));
        self.send(id, Clientbound::UpdateHealth(UpdateHealth { health: 20 }));

        if let Some(player) = self.entities.get_by_conn(id) {
            let window_items = player.inventory.to_window_items();
            self.send(
                id,
                Clientbound::WindowItems(WindowItems {
                    window_id: 0,
                    items: window_items,
                }),
            );
        }
    }

    fn handle_login(&mut self, id: ConnId, login: Login) {
        if login.protocol != PROTOCOL_VERSION {
            let reason = if login.protocol > PROTOCOL_VERSION {
                "Outdated server! I'm still on Beta 1.7.3"
            } else {
                "Outdated client! Please use Beta 1.7.3"
            };
            self.kick(id, reason);
            return;
        }

        let seed = self.world.seed();
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();
        let player_x = spawn_x as f64 + 0.5;
        let player_y = spawn_y as f64;
        let player_z = spawn_z as f64 + 0.5;

        let entity_id =
            self.entities
                .insert_player(id, login.username.clone(), player_x, player_y, player_z);

        if let Some(connection) = self.connections.get_mut(&id) {
            connection.state = ConnectionState::Playing;
            connection.username = Some(login.username.clone());
            connection.entity_id = entity_id;
        }

        if let Some(player) = self.entities.get_player(entity_id) {
            self.tracker.track_player(player);
        }

        log::info(&format!(
            "{} logged in with entity id {entity_id}",
            login.username
        ));

        self.send(
            id,
            Clientbound::Login(Login {
                protocol: entity_id,
                username: String::new(),
                map_seed: seed,
                dimension: 0,
            }),
        );
        self.send(
            id,
            Clientbound::SpawnPosition(SpawnPosition {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
            }),
        );
        self.send(id, Clientbound::UpdateTime(UpdateTime { time: 0 }));

        // Stream initial spawn area chunks and send spawn teleport
        self.stream_initial_chunks(id);

        // Sync initial inventory to client
        if let Some(player) = self.entities.get_player(entity_id) {
            let window_items = player.inventory.to_window_items();
            self.send(
                id,
                Clientbound::WindowItems(WindowItems {
                    window_id: 0,
                    items: window_items,
                }),
            );
        }

        // Send UpdateHealth to the new player
        self.send(id, Clientbound::UpdateHealth(UpdateHealth { health: 20 }));

        // Poseidon: PlayerJoinEvent -> broadcast join message to other players
        let join_msg = self
            .config
            .server
            .join_message
            .replacen("%player%", &login.username, 1);
        self.broadcast_chat_except(id, &join_msg);
    }

    fn stream_initial_chunks(&mut self, id: ConnId) {
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();
        let center = ChunkPos::new(spawn_x >> 4, spawn_z >> 4);
        let radius = 1; // 3x3 immediate area for spawn handshake

        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let pos = ChunkPos::new(center.x + dx, center.z + dz);
                self.world.populate_chunk(pos);
                if let Ok((pre, map)) = self.world.prepare_chunk_packets(pos) {
                    self.send(id, Clientbound::PreChunk(pre));
                    self.send(id, Clientbound::MapChunk(map));
                    if let Some(player) = self.entities.get_by_conn_mut(id) {
                        player.sent_chunks.insert(pos);
                    }
                }
            }
        }

        let player_x = spawn_x as f64 + 0.5;
        let player_y = spawn_y as f64;
        let player_z = spawn_z as f64 + 0.5;

        self.send(
            id,
            Clientbound::PlayerLookMove(PlayerLookMove {
                x: player_x,
                y: player_y + 1.62,
                z: player_z,
                stance: player_y,
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            }),
        );
    }

    fn stream_chunks(&mut self) {
        let view_distance = self.config.world.view_distance;
        let chunks_per_tick = self.config.world.chunks_per_tick;

        let players: Vec<(ConnId, ChunkPos)> = self
            .entities
            .iter_players()
            .map(|p| (p.conn_id, p.chunk_pos()))
            .collect();

        for (id, center) in players {
            let mut sent_this_tick = 0;
            let mut populated_this_tick = 0;

            // Unload out of range chunks
            let unload_radius = view_distance + 1;
            let to_unload: Vec<ChunkPos> = if let Some(player) = self.entities.get_by_conn(id) {
                player
                    .sent_chunks
                    .iter()
                    .copied()
                    .filter(|pos| {
                        (pos.x - center.x).abs() > unload_radius
                            || (pos.z - center.z).abs() > unload_radius
                    })
                    .collect()
            } else {
                Vec::new()
            };

            for pos in to_unload {
                self.send(
                    id,
                    Clientbound::PreChunk(PreChunk {
                        x: pos.x,
                        z: pos.z,
                        mode: false,
                    }),
                );
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.sent_chunks.remove(&pos);
                }
            }

            // Stream new chunks in view distance ring by ring outwards from player
            for r in 0..=view_distance {
                if sent_this_tick >= chunks_per_tick {
                    break;
                }
                for dz in -r..=r {
                    if sent_this_tick >= chunks_per_tick {
                        break;
                    }
                    for dx in -r..=r {
                        if dx.abs() != r && dz.abs() != r {
                            continue;
                        }
                        let pos = ChunkPos::new(center.x + dx, center.z + dz);
                        let already_sent = self
                            .entities
                            .get_by_conn(id)
                            .is_some_and(|p| p.sent_chunks.contains(&pos));

                        if !already_sent {
                            let is_populated = self
                                .world
                                .get_chunk(pos)
                                .is_some_and(|c| c.state == ChunkState::Populated);

                            if !is_populated {
                                if populated_this_tick >= 1 {
                                    continue;
                                }
                                self.world.populate_chunk(pos);
                                populated_this_tick += 1;
                            }

                            if let Ok((pre, map)) = self.world.prepare_chunk_packets(pos) {
                                self.send(id, Clientbound::PreChunk(pre));
                                self.send(id, Clientbound::MapChunk(map));
                                if let Some(player) = self.entities.get_by_conn_mut(id) {
                                    player.sent_chunks.insert(pos);
                                }
                                sent_this_tick += 1;
                                if sent_this_tick >= chunks_per_tick {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_chat(&mut self, id: ConnId, chat: Chat) {
        let username = self
            .connections
            .get(&id)
            .and_then(|connection| connection.username.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let message = chat.message.trim().to_string();

        // Poseidon: length > 100 -> disconnect "Chat message too long"
        if message.len() > 100 {
            self.kick(id, "Chat message too long");
            return;
        }

        // Poseidon: illegal characters -> disconnect "Illegal characters in chat"
        if !message.chars().all(is_allowed_char) {
            self.kick(id, "Illegal characters in chat");
            return;
        }

        if message.is_empty() {
            return;
        }

        if message.starts_with('/') {
            self.handle_command(id, &message);
            return;
        }

        let all_players: Vec<String> = self
            .entities
            .iter_players()
            .map(|p| p.username.clone())
            .collect();
        let mut event = PlayerChatEvent::new(username.clone(), message.clone(), &all_players);
        self.plugin_manager.call_chat_event(&mut event);
        if event.is_cancelled() {
            return;
        }

        let formatted =
            event
                .get_format()
                .replacen("%1$s", &username, 1)
                .replacen("%2$s", &event.message, 1);

        log::info(&formatted);
        self.broadcast_chat(&formatted);
    }

    fn handle_command(&mut self, id: ConnId, command: &str) {
        let username = self
            .connections
            .get(&id)
            .and_then(|connection| connection.username.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let result = self.commands.dispatch(&username, &command[1..]);

        if !result.handled {
            self.send(
                id,
                Clientbound::Chat(Chat {
                    message: "Unknown command. Type \"help\" for help.".to_string(),
                }),
            );
            return;
        }

        log::info(&format!("{username} issued server command: {command}"));

        for msg in &result.sender_messages {
            self.send(
                id,
                Clientbound::Chat(Chat {
                    message: msg.clone(),
                }),
            );
        }

        for msg in &result.broadcast_messages {
            self.broadcast_chat(msg);
        }
    }

    fn damage_player(&mut self, entity_id: i32, damage: i16, reason: &str) {
        let (conn_id, username, health, died) = {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let (dealt, died) = player.take_damage(damage);
                if dealt > 0 {
                    (player.conn_id, player.username.clone(), player.health, died)
                } else {
                    return;
                }
            } else {
                return;
            }
        };

        // Hurt status animation (status 2) to observers and player
        self.broadcast(Clientbound::EntityStatus(EntityStatus {
            entity_id,
            status: 2,
        }));

        self.send(conn_id, Clientbound::UpdateHealth(UpdateHealth { health }));

        if died {
            let death_msg = format!("\u{00A7}e{username} {reason}");
            self.broadcast_chat(&death_msg);
            self.broadcast(Clientbound::EntityStatus(EntityStatus {
                entity_id,
                status: 3,
            }));
        }
    }

    fn broadcast(&self, packet: Clientbound) {
        for connection in self.connections.values() {
            if connection.state == ConnectionState::Playing {
                let _ = connection.sender.try_send(Outbound::Packet(packet.clone()));
            }
        }
    }

    fn broadcast_chat(&self, message: &str) {
        for connection in self.connections.values() {
            if connection.state != ConnectionState::Playing {
                continue;
            }
            for line in wrap_text(message) {
                let _ = connection
                    .sender
                    .try_send(Outbound::Packet(Clientbound::Chat(Chat { message: line })));
            }
        }
    }

    fn broadcast_chat_except(&self, except_id: ConnId, message: &str) {
        for (&conn_id, connection) in &self.connections {
            if conn_id == except_id || connection.state != ConnectionState::Playing {
                continue;
            }
            for line in wrap_text(message) {
                let _ = connection
                    .sender
                    .try_send(Outbound::Packet(Clientbound::Chat(Chat { message: line })));
            }
        }
    }

    fn send(&self, id: ConnId, packet: Clientbound) {
        if let Some(connection) = self.connections.get(&id) {
            let _ = connection.sender.try_send(Outbound::Packet(packet));
        }
    }

    fn kick(&self, id: ConnId, reason: &str) {
        if let Some(connection) = self.connections.get(&id) {
            let _ = connection
                .sender
                .try_send(Outbound::Packet(Clientbound::KickDisconnect(
                    KickDisconnect {
                        reason: reason.to_string(),
                    },
                )));
            let _ = connection.sender.try_send(Outbound::Close);
        }
    }
}
