use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::config::Config;
use crate::gameplay::blocks;
use crate::net::connection::ConnectionState;
use crate::net::packets::{
    BlockChange, Chat, Clientbound, Handshake, KeepAlive, KickDisconnect, Login, PlayerLookMove,
    PreChunk, Respawn, Serverbound, SpawnPosition, UpdateTime,
};
use crate::net::protocol::PROTOCOL_VERSION;
use crate::net::{ConnId, NetEvent, Outbound};
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
    x: f64,
    y: f64,
    z: f64,
    yaw: f32,
    pitch: f32,
    on_ground: bool,
    held_slot: i16,
    sneaking: bool,
    sent_chunks: HashSet<ChunkPos>,
}

pub struct Server {
    config: Config,
    inbound: mpsc::Receiver<NetEvent>,
    connections: HashMap<ConnId, Connection>,
    running: Arc<AtomicBool>,
    next_entity_id: i32,
    world: World,
}

impl Server {
    pub fn new(
        config: Config,
        inbound: mpsc::Receiver<NetEvent>,
        running: Arc<AtomicBool>,
    ) -> Self {
        let world = World::new(config.world.clone());
        Self {
            config,
            inbound,
            connections: HashMap::new(),
            running,
            next_entity_id: 1,
            world,
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
        self.drain_inbound();
        self.stream_chunks();
        if ticks % 20 == 0 {
            self.broadcast_keep_alive();
        }
        if ticks % 600 == 0 {
            log::info(&format!(
                "tick {ticks}, {} connections, {} chunks loaded",
                self.connections.len(),
                self.world.chunk_count()
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
                        x: 0.0,
                        y: 64.0,
                        z: 0.0,
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground: false,
                        held_slot: 0,
                        sneaking: false,
                        sent_chunks: HashSet::new(),
                    },
                );
            }
            NetEvent::Packet { id, packet } => self.handle_packet(id, packet),
            NetEvent::Disconnected { id } => {
                self.connections.remove(&id);
                log::info(&format!("connection {id} disconnected"));
            }
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
                if let Some(conn) = self.connections.get_mut(&id) {
                    conn.on_ground = flying.on_ground;
                }
            }
            Serverbound::PlayerPosition(pos) => {
                if let Some(conn) = self.connections.get_mut(&id) {
                    conn.x = pos.x;
                    conn.y = pos.y;
                    conn.z = pos.z;
                    conn.on_ground = pos.on_ground;
                }
            }
            Serverbound::PlayerLook(look) => {
                if let Some(conn) = self.connections.get_mut(&id) {
                    conn.yaw = look.yaw;
                    conn.pitch = look.pitch;
                    conn.on_ground = look.on_ground;
                }
            }
            Serverbound::PlayerLookMove(look_move) => {
                if let Some(conn) = self.connections.get_mut(&id) {
                    conn.x = look_move.x;
                    conn.y = look_move.y;
                    conn.z = look_move.z;
                    conn.yaw = look_move.yaw;
                    conn.pitch = look_move.pitch;
                    conn.on_ground = look_move.on_ground;
                }
            }
            Serverbound::ArmAnimation(anim) => {
                for (&other_id, conn) in &self.connections {
                    if other_id != id && conn.state == ConnectionState::Playing {
                        self.send(other_id, Clientbound::ArmAnimation(anim.clone()));
                    }
                }
            }
            Serverbound::BlockItemSwitch(switch) => {
                if let Some(conn) = self.connections.get_mut(&id) {
                    conn.held_slot = switch.slot;
                }
            }
            Serverbound::EntityAction(action) => {
                if let Some(conn) = self.connections.get_mut(&id) {
                    match action.action {
                        1 => conn.sneaking = true,
                        2 => conn.sneaking = false,
                        _ => {}
                    }
                }
            }
            Serverbound::BlockDig(dig) => {
                if dig.status == 2 {
                    self.world
                        .set_block(dig.x, dig.y as i32, dig.z, blocks::AIR);
                    let change = Clientbound::BlockChange(BlockChange {
                        x: dig.x,
                        y: dig.y,
                        z: dig.z,
                        block_type: blocks::AIR,
                        metadata: 0,
                    });
                    for (&other_id, conn) in &self.connections {
                        if conn.state == ConnectionState::Playing {
                            self.send(other_id, change.clone());
                        }
                    }
                }
            }
            Serverbound::Place(place) => {
                let (tx, ty, tz) = match place.direction {
                    0 => (place.x, place.y.saturating_sub(1), place.z),
                    1 => (place.x, place.y.saturating_add(1), place.z),
                    2 => (place.x, place.y, place.z - 1),
                    3 => (place.x, place.y, place.z + 1),
                    4 => (place.x - 1, place.y, place.z),
                    5 => (place.x + 1, place.y, place.z),
                    _ => (place.x, place.y, place.z),
                };
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
                    for (&other_id, conn) in &self.connections {
                        if conn.state == ConnectionState::Playing {
                            self.send(other_id, change.clone());
                        }
                    }
                }
            }
            Serverbound::UseEntity(_)
            | Serverbound::CloseWindow(_)
            | Serverbound::WindowClick(_)
            | Serverbound::Transaction(_)
            | Serverbound::UpdateSign(_) => {}
            Serverbound::Respawn(_) => {
                let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();
                self.send(id, Clientbound::Respawn(Respawn { dimension: 0 }));
                self.stream_initial_chunks(id);
                self.send(
                    id,
                    Clientbound::PlayerLookMove(PlayerLookMove {
                        x: spawn_x as f64 + 0.5,
                        y: spawn_y as f64 + 1.62,
                        z: spawn_z as f64 + 0.5,
                        stance: spawn_y as f64,
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground: false,
                    }),
                );
            }
            Serverbound::KickDisconnect(kick) => {
                log::info(&format!("player disconnected: {}", kick.reason));
                self.connections.remove(&id);
            }
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

        let entity_id = self.next_entity_id;
        self.next_entity_id = self.next_entity_id.wrapping_add(1).max(1);

        let seed = self.world.seed();
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();
        let player_x = spawn_x as f64 + 0.5;
        let player_y = spawn_y as f64;
        let player_z = spawn_z as f64 + 0.5;

        if let Some(connection) = self.connections.get_mut(&id) {
            connection.state = ConnectionState::Playing;
            connection.username = Some(login.username.clone());
            connection.entity_id = entity_id;
            connection.x = player_x;
            connection.y = player_y;
            connection.z = player_z;
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
                    if let Some(conn) = self.connections.get_mut(&id) {
                        conn.sent_chunks.insert(pos);
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
            .connections
            .iter()
            .filter(|(_, conn)| conn.state == ConnectionState::Playing)
            .map(|(&id, conn)| {
                (
                    id,
                    ChunkPos::new((conn.x as i32) >> 4, (conn.z as i32) >> 4),
                )
            })
            .collect();

        for (id, center) in players {
            let mut sent_this_tick = 0;
            let mut populated_this_tick = 0;

            // Unload out of range chunks
            let unload_radius = view_distance + 1;
            let to_unload: Vec<ChunkPos> = if let Some(conn) = self.connections.get(&id) {
                conn.sent_chunks
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
                if let Some(conn) = self.connections.get_mut(&id) {
                    conn.sent_chunks.remove(&pos);
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
                            .connections
                            .get(&id)
                            .is_some_and(|c| c.sent_chunks.contains(&pos));

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
                                if let Some(conn) = self.connections.get_mut(&id) {
                                    conn.sent_chunks.insert(pos);
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
        for connection in self.connections.values() {
            let _ = connection
                .sender
                .try_send(Outbound::Packet(Clientbound::Chat(Chat {
                    message: format!("<{username}> {}", chat.message),
                })));
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
