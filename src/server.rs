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
use crate::gameplay::inventory::{InventoryPlayer, OpenContainer};
use crate::gameplay::items::{self, ItemStack};
use crate::gameplay::tile::{TileEntity, TileEntityKind};
use crate::net::connection::ConnectionState;
use crate::net::packets::chat::{is_allowed_char, wrap_text};
use crate::net::packets::{
    ArmAnimation, BlockChange, Chat, Clientbound, Collect, CraftProgressBar, EntityStatus,
    EntityVelocity, Handshake, KeepAlive, KickDisconnect, LevelSound, Login, MultiBlockChange,
    OpenWindow, PlayNoteBlock, PlayerLookMove, PreChunk, Respawn, Serverbound, SetSlot,
    SpawnPosition, Transaction, UpdateHealth, UpdateSign, UpdateTime, WindowItems,
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
    player_store: crate::world::storage::PlayerStore,
    commands: CommandMap,
    plugin_manager: PluginManager,
    current_tick: u64,
}

impl Server {
    pub fn new(
        mut config: Config,
        inbound: mpsc::Receiver<NetEvent>,
        running: Arc<AtomicBool>,
    ) -> Self {
        let mut world = World::new(config.world.clone());
        if config.world.save_enabled {
            world.live_enable();
            if world.is_fresh_world() {
                let rng_seed = std::env::var("FLINTCORE_SPAWN_RNG_SEED")
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or_else(|| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos() as i64)
                            .unwrap_or(config.world.seed)
                    });
                let (sx, sz) = world.live_find_spawn(rng_seed);
                config.world.spawn_x = sx;
                config.world.spawn_y = 64;
                config.world.spawn_z = sz;
            } else {
                let (sx, sy, sz) = world.spawn_point();
                config.world.spawn_x = sx;
                config.world.spawn_y = sy;
                config.world.spawn_z = sz;
            }
            log::info(&format!(
                "Preparing start region for level 0 (Seed: {})",
                world.seed()
            ));
            world.live_prepare_spawn_region();
            // Queue startup-only chunks outside the 128-block keep-spawn radius for unload
            world.queue_unwatched_chunks();
        }
        let player_store = crate::world::storage::PlayerStore::new(world.world_dir());
        let view_distance = config.world.view_distance;
        Self {
            config,
            inbound,
            connections: HashMap::new(),
            running,
            entities: EntityStore::new(),
            tracker: EntityTracker::new(view_distance),
            world,
            player_store,
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

        if self.config.world.save_enabled {
            log::info("saving world and player data before shutdown...");
            for player in self.entities.iter_players() {
                let _ = self.player_store.save_player(player);
            }
            let _ = self.world.save_all();
        }

        log::info("simulation thread stopped");
        Ok(())
    }

    fn tick(&mut self, ticks: u64) {
        self.current_tick = ticks;
        if self.world.live_is_active() {
            self.world.live_tick();
        } else {
            self.world.increment_time();
        }
        self.drain_inbound();

        // Auto-save every 6000 ticks (5 minutes)
        if ticks % 6000 == 0 && ticks > 0 {
            for player in self.entities.iter_players() {
                let _ = self.player_store.save_player(player);
            }
            let _ = self.world.save_all();
        }

        // Tick furnaces and sync open furnace windows
        self.world.tick_furnaces();
        self.sync_open_containers();

        // Tick each player (no_damage_ticks, fire_ticks decrement, deferred digging)
        for player in self.entities.iter_players_mut() {
            player.tick();
        }
        self.tick_deferred_digging();

        // Tick dropped items and player item pickup
        self.tick_dropped_items();

        self.stream_chunks();
        self.flush_dirty_chunks();

        // Tick entity tracking every 2 ticks (mirroring Poseidon EntityTracker)
        if ticks % 2 == 0 {
            let mut players: Vec<_> = self.entities.iter_players().cloned().collect();
            let mut items: Vec<_> = self.entities.iter_dropped_items().cloned().collect();
            let tracking_packets = self.tracker.tick_with_items(&mut players, &mut items);
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
            self.close_player_container_and_drop_transient(id, false);
            if let Some(player) = self.entities.remove_by_conn(id) {
                if self.world.live_is_active() {
                    self.world.live_player_quit(id);
                }
                if self.config.world.save_enabled {
                    let _ = self.player_store.save_player(&player);
                }

                let untrack_packets = self.tracker.untrack_player(player.entity_id);
                for outbound in untrack_packets {
                    if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                        self.send(target.conn_id, outbound.packet);
                    }
                }

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
                self.handle_movement(id, None, None, flying.on_ground);
            }
            Serverbound::PlayerPosition(pos) => {
                self.handle_movement(
                    id,
                    Some((pos.x, pos.y, pos.stance, pos.z)),
                    None,
                    pos.on_ground,
                );
            }
            Serverbound::PlayerLook(look) => {
                self.handle_movement(id, None, Some((look.yaw, look.pitch)), look.on_ground);
            }
            Serverbound::PlayerLookMove(look_move) => {
                self.handle_movement(
                    id,
                    Some((look_move.x, look_move.y, look_move.stance, look_move.z)),
                    Some((look_move.yaw, look_move.pitch)),
                    look_move.on_ground,
                );
            }
            Serverbound::ArmAnimation(anim) => {
                for (&other_id, conn) in &self.connections {
                    if other_id != id && conn.state == ConnectionState::Playing {
                        self.send(other_id, Clientbound::ArmAnimation(anim.clone()));
                    }
                }
            }
            Serverbound::BlockItemSwitch(switch) => {
                if !(0..=8).contains(&switch.slot) {
                    self.kick(id, "Invalid hotbar selection (Hacking?)");
                    return;
                }
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.inventory.held_slot = switch.slot as usize;
                }
            }
            Serverbound::EntityAction(action) => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    match action.action {
                        1 => player.sneaking = true,
                        2 => player.sneaking = false,
                        3 => {
                            player.sleeping = false;
                            player.bed_pos = None;
                        }
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
            Serverbound::CloseWindow(_) => {
                self.close_player_container_and_drop_transient(id, true);
            }
            Serverbound::Transaction(_) => {}
            Serverbound::UpdateSign(sign) => {
                self.handle_update_sign(id, sign);
            }
            Serverbound::UseEntity(use_entity) => {
                self.handle_use_entity(id, use_entity);
            }
            Serverbound::Respawn(_) => {
                self.handle_respawn(id);
            }
            Serverbound::KickDisconnect(kick) => {
                log::info(&format!("player disconnected: {}", kick.reason));
                self.remove_and_handle_disconnect(id);
            }
        }
    }

    fn handle_movement(
        &mut self,
        id: ConnId,
        pos: Option<(f64, f64, f64, f64)>,
        rot: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        let (fall_damage, entity_id, old_chunk, new_chunk) = {
            if let Some(player) = self.entities.get_by_conn_mut(id) {
                if player.dead {
                    return;
                }
                let old_chunk = player.chunk_pos();
                let prev_y = player.y;
                let mut fall_damage = 0i16;

                let (x, y, stance, z) =
                    pos.unwrap_or((player.x, player.y, player.stance, player.z));
                let (yaw, pitch) = rot.unwrap_or((player.yaw, player.pitch));

                if x.is_nan() || y.is_nan() || z.is_nan() || stance.is_nan() {
                    self.kick(id, "Nope!");
                    return;
                }

                if !on_ground {
                    if y < prev_y {
                        player.fall_distance += (prev_y - y) as f32;
                    }
                } else {
                    if player.fall_distance > 3.0 {
                        fall_damage = ((player.fall_distance - 3.0).ceil() as i16).max(0);
                    }
                    player.fall_distance = 0.0;
                }

                player.update_position(x, y, stance, z, yaw, pitch, on_ground);
                let new_chunk = player.chunk_pos();

                (fall_damage, player.entity_id, old_chunk, new_chunk)
            } else {
                return;
            }
        };

        self.entities
            .update_player_chunk(entity_id, old_chunk, new_chunk);

        if fall_damage > 0 {
            self.damage_player_direct(entity_id, fall_damage, "hit the ground too hard");
        }

        if self.world.live_is_active() {
            let (cur_x, cur_z) = match self.entities.get_by_conn(id) {
                Some(p) => (p.x, p.z),
                None => return,
            };
            self.world.live_player_move(id, cur_x, cur_z);
            let (added, removed) = self.world.take_player_chunk_events(id);
            if !added.is_empty() || !removed.is_empty() {
                let cur_cx = (cur_x as i32) >> 4;
                let cur_cz = (cur_z as i32) >> 4;

                let mut pre_false = Vec::new();
                let mut pre_true = Vec::new();
                if let Some(p_mut) = self.entities.get_by_conn_mut(id) {
                    for p in removed {
                        p_mut.sent_chunks.remove(&p);
                        p_mut.chunk_queue.retain(|&q| q != p);
                        if p_mut.pre_chunk_sent.remove(&p) {
                            pre_false.push(p);
                        }
                    }
                    for p in added {
                        if p_mut.pre_chunk_sent.insert(p) {
                            pre_true.push(p);
                        }
                        if !p_mut.sent_chunks.contains(&p) && !p_mut.chunk_queue.contains(&p) {
                            p_mut.chunk_queue.push(p);
                        }
                    }
                    p_mut
                        .chunk_queue
                        .sort_by_key(|p| (p.x - cur_cx).abs().max((p.z - cur_cz).abs()));
                    p_mut.last_eval_x = cur_x;
                    p_mut.last_eval_z = cur_z;
                }

                for p in pre_false {
                    self.send(
                        id,
                        Clientbound::PreChunk(PreChunk {
                            x: p.x,
                            z: p.z,
                            mode: false,
                        }),
                    );
                }
                for p in pre_true {
                    self.send(
                        id,
                        Clientbound::PreChunk(PreChunk {
                            x: p.x,
                            z: p.z,
                            mode: true,
                        }),
                    );
                }
            }
            return;
        }

        // Offline / non-live fallback path (`save_enabled == false`)
        let move_eval = if let Some(player) = self.entities.get_by_conn(id) {
            let dx = player.x - player.last_eval_x;
            let dz = player.z - player.last_eval_z;
            if dx * dx + dz * dz >= 64.0 {
                let old_cx = (player.last_eval_x as i32) >> 4;
                let old_cz = (player.last_eval_z as i32) >> 4;
                let new_cx = (player.x as i32) >> 4;
                let new_cz = (player.z as i32) >> 4;
                if old_cx != new_cx || old_cz != new_cz {
                    Some((old_cx, old_cz, new_cx, new_cz, player.x, player.z))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some((old_cx, old_cz, new_cx, new_cz, cur_x, cur_z)) = move_eval {
            let view_dist = self.config.world.view_distance;
            let mut pre_false = Vec::new();
            let mut pre_true = Vec::new();

            if let Some(p_mut) = self.entities.get_by_conn_mut(id) {
                let out_of_view: Vec<ChunkPos> = p_mut
                    .pre_chunk_sent
                    .iter()
                    .chain(p_mut.sent_chunks.iter())
                    .chain(p_mut.chunk_queue.iter())
                    .copied()
                    .filter(|p| {
                        (p.x - new_cx).abs() > view_dist || (p.z - new_cz).abs() > view_dist
                    })
                    .collect();

                for p in out_of_view {
                    p_mut.sent_chunks.remove(&p);
                    p_mut.chunk_queue.retain(|&q| q != p);
                    if p_mut.pre_chunk_sent.remove(&p) {
                        pre_false.push(p);
                    }
                }

                for dz in -view_dist..=view_dist {
                    for dx in -view_dist..=view_dist {
                        let p = ChunkPos::new(new_cx + dx, new_cz + dz);
                        let was_in_old =
                            (p.x - old_cx).abs() <= view_dist && (p.z - old_cz).abs() <= view_dist;
                        if !was_in_old {
                            if p_mut.pre_chunk_sent.insert(p) {
                                pre_true.push(p);
                            }
                            if !p_mut.sent_chunks.contains(&p) && !p_mut.chunk_queue.contains(&p) {
                                p_mut.chunk_queue.push(p);
                            }
                        }
                    }
                }

                p_mut
                    .chunk_queue
                    .sort_by_key(|p| (p.x - new_cx).abs().max((p.z - new_cz).abs()));
                p_mut.last_eval_x = cur_x;
                p_mut.last_eval_z = cur_z;
            }

            for p in pre_false {
                self.send(
                    id,
                    Clientbound::PreChunk(PreChunk {
                        x: p.x,
                        z: p.z,
                        mode: false,
                    }),
                );
            }
            for p in pre_true {
                self.send(
                    id,
                    Clientbound::PreChunk(PreChunk {
                        x: p.x,
                        z: p.z,
                        mode: true,
                    }),
                );
            }

            // Evict chunks far from all players when save_enabled == false
            let player_centers: Vec<(i32, i32)> = self
                .entities
                .iter_players()
                .map(|p| ((p.x as i32) >> 4, (p.z as i32) >> 4))
                .collect();
            let keep_dist = view_dist + 3;
            let loaded: Vec<ChunkPos> = self.world.chunks.keys().copied().collect();
            for pos in loaded {
                let near_any = player_centers.iter().any(|&(cx, cz)| {
                    (pos.x - cx).abs() <= keep_dist && (pos.z - cz).abs() <= keep_dist
                });
                let near_spawn = pos.x.abs() <= 8 && pos.z.abs() <= 8;
                if !near_any && !near_spawn {
                    self.world.chunks.remove(&pos);
                    self.world.packet_cache.borrow_mut().remove(&pos);
                }
            }
        }
    }

    fn handle_use_entity(&mut self, id: ConnId, use_entity: crate::net::packets::UseEntity) {
        let (attacker_id, attacker_x, attacker_z, held_item, attacker_name) = {
            if let Some(attacker) = self.entities.get_by_conn(id) {
                if attacker.dead {
                    return;
                }
                (
                    attacker.entity_id,
                    attacker.x,
                    attacker.z,
                    attacker.inventory.item_in_hand(),
                    attacker.username.clone(),
                )
            } else {
                return;
            }
        };

        self.broadcast(Clientbound::ArmAnimation(ArmAnimation {
            entity_id: attacker_id,
            animate: 1,
        }));

        if use_entity.attack {
            let target_id = use_entity.target;
            let (target_conn_id, target_name, target_health, died, mot_x, mot_y, mot_z, drops) = {
                if let Some(target) = self.entities.get_player_mut(target_id) {
                    if target.dead {
                        return;
                    }
                    let dx = target.x - attacker_x;
                    let dz = target.z - attacker_z;
                    if dx * dx + dz * dz > 36.0 {
                        return;
                    }

                    let raw_damage = held_item.map(|s| s.attack_damage()).unwrap_or(1);
                    let (dealt, died) = target.take_damage(raw_damage);
                    if dealt <= 0 {
                        return;
                    }

                    target.apply_knockback(attacker_x, attacker_z);
                    let mot_x = (target.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16;
                    let mot_y = (target.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16;
                    let mot_z = (target.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16;
                    let drops = if died {
                        let tx = target.x;
                        let ty = target.y;
                        let tz = target.z;
                        target
                            .inventory
                            .drop_all_on_death()
                            .into_iter()
                            .map(|it| (tx, ty, tz, it))
                            .collect()
                    } else {
                        Vec::new()
                    };

                    (
                        target.conn_id,
                        target.username.clone(),
                        target.health,
                        died,
                        mot_x,
                        mot_y,
                        mot_z,
                        drops,
                    )
                } else {
                    return;
                }
            };

            let vel = Clientbound::EntityVelocity(EntityVelocity {
                entity_id: target_id,
                mot_x,
                mot_y,
                mot_z,
            });
            self.send(target_conn_id, vel.clone());
            self.broadcast(vel);

            self.broadcast(Clientbound::EntityStatus(EntityStatus {
                entity_id: target_id,
                status: 2,
            }));

            self.send(
                target_conn_id,
                Clientbound::UpdateHealth(UpdateHealth {
                    health: target_health,
                }),
            );

            // Damage weapon/tool durability on attacker (Sword = 1, Tool = 2)
            if let Some(attacker) = self.entities.get_by_conn_mut(id) {
                if let Some(mut item) = attacker.inventory.item_in_hand() {
                    let dmg_cost = if item.is_weapon() {
                        1
                    } else if item.is_tool() {
                        2
                    } else {
                        0
                    };
                    if dmg_cost > 0 {
                        let broken = item.damage_item(dmg_cost);
                        if broken {
                            attacker.inventory.set_item_in_hand(None);
                        } else {
                            attacker.inventory.set_item_in_hand(Some(item));
                        }
                        let slot_idx = 36 + attacker.inventory.held_slot as i16;
                        let slot_item =
                            InventoryPlayer::to_slot_item(attacker.inventory.item_in_hand());
                        self.send(
                            id,
                            Clientbound::SetSlot(SetSlot {
                                window_id: 0,
                                slot: slot_idx,
                                item: slot_item,
                            }),
                        );
                    }
                }
            }

            if died {
                for (dx, dy, dz, stack) in drops {
                    self.spawn_dropped_item_random(dx, dy, dz, stack, 40);
                }
                let death_msg = format!("\u{00A7}e{target_name} was slain by {attacker_name}");
                self.broadcast_chat(&death_msg);
                self.broadcast(Clientbound::EntityStatus(EntityStatus {
                    entity_id: target_id,
                    status: 3,
                }));
            }
        }
    }

    fn handle_block_dig(&mut self, id: ConnId, dig: crate::net::packets::BlockDig) {
        if dig.status == 4 {
            // Drop item in hand (Q key)
            let drop_action = if let Some(player) = self.entities.get_by_conn_mut(id) {
                if player.dead {
                    return;
                }
                if player.last_drop_tick != self.current_tick {
                    player.drop_count = 0;
                    player.last_drop_tick = self.current_tick;
                } else {
                    player.drop_count += 1;
                    if player.drop_count >= 20 {
                        self.kick(id, "You dropped your items too quickly (Hacking?)");
                        return;
                    }
                }
                let dropped = player.inventory.split_held_item(1);
                let slot_idx = 36 + player.inventory.held_slot as i16;
                let slot_item = InventoryPlayer::to_slot_item(player.inventory.item_in_hand());
                Some((
                    dropped,
                    slot_idx,
                    slot_item,
                    player.x,
                    player.y,
                    player.z,
                    player.yaw,
                    player.pitch,
                ))
            } else {
                None
            };

            if let Some((dropped, slot_idx, slot_item, px, py, pz, yaw, pitch)) = drop_action {
                self.send(
                    id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: 0,
                        slot: slot_idx,
                        item: slot_item,
                    }),
                );
                if let Some(stack) = dropped {
                    self.spawn_thrown_item(px, py, pz, yaw, pitch, stack);
                }
            }
            return;
        }

        let (x, y, z, entity_id, held_item, on_ground, prev_digging) = {
            if let Some(player) = self.entities.get_by_conn(id) {
                if player.dead {
                    return;
                }
                let dx = player.x - (dig.x as f64 + 0.5);
                let dy = player.y - (dig.y as f64 + 0.5);
                let dz = player.z - (dig.z as f64 + 0.5);
                if (dig.status == 0 || dig.status == 2) && (dx * dx + dy * dy + dz * dz > 36.0) {
                    return;
                }
                (
                    dig.x,
                    dig.y as i32,
                    dig.z,
                    player.entity_id,
                    player.inventory.item_in_hand(),
                    player.on_ground,
                    player.digging.clone(),
                )
            } else {
                return;
            }
        };

        if dig.status == 3 {
            let block = self.world.get_block(x, y, z);
            let meta = self.world.get_data(x, y, z);
            self.send(
                id,
                Clientbound::BlockChange(BlockChange {
                    x,
                    y: y as u8,
                    z,
                    block_type: block,
                    metadata: meta,
                }),
            );
            return;
        }

        // Poseidon: Spawn protection
        let spawn_x = self.config.world.spawn_x;
        let spawn_z = self.config.world.spawn_z;
        let radius = self.config.world.spawn_radius;
        let chebyshev = (x - spawn_x).abs().max((z - spawn_z).abs());
        if chebyshev <= radius {
            let block = self.world.get_block(x, y, z);
            let meta = self.world.get_data(x, y, z);
            self.send(
                id,
                Clientbound::BlockChange(BlockChange {
                    x,
                    y: y as u8,
                    z,
                    block_type: block,
                    metadata: meta,
                }),
            );
            return;
        }

        if dig.status == 0 {
            let block_id = self.world.get_block(x, y, z);
            if block_id == blocks::AIR {
                return;
            }

            // Douse fire on the clicked face
            self.douse_fire(x, y, z, dig.face);

            // Left-click block interaction (`Block.b`)
            self.on_block_left_click(id, block_id, x, y, z);

            let dmg_per_tick =
                items::calc_block_damage_per_tick(held_item, block_id, false, on_ground);
            if dmg_per_tick >= 1.0 {
                self.finish_block_break(id, entity_id, x, y, z);
            } else if let Some(player) = self.entities.get_player_mut(entity_id) {
                player.digging = Some(DiggingState {
                    x,
                    y,
                    z,
                    face: dig.face,
                    start_tick: self.current_tick,
                });
                player.pending_dig = None;
            }
        } else if dig.status == 2 {
            let block_id = self.world.get_block(x, y, z);
            if block_id == blocks::AIR {
                return;
            }
            let dmg_per_tick =
                items::calc_block_damage_per_tick(held_item, block_id, false, on_ground);

            match prev_digging {
                Some(state) if state.x == x && state.y == y && state.z == z => {
                    let ticks = self.current_tick.saturating_sub(state.start_tick) + 1;
                    let progress = dmg_per_tick * (ticks as f32);
                    if progress >= 0.7 {
                        self.finish_block_break(id, entity_id, x, y, z);
                    } else if let Some(player) = self.entities.get_player_mut(entity_id) {
                        player.pending_dig = Some(state);
                    }
                }
                None => {
                    // Direct status=2 without prior status=0 (used in integration tests or insta-mine)
                    self.finish_block_break(id, entity_id, x, y, z);
                }
                Some(_) => {
                    let meta = self.world.get_data(x, y, z);
                    self.send(
                        id,
                        Clientbound::BlockChange(BlockChange {
                            x,
                            y: y as u8,
                            z,
                            block_type: block_id,
                            metadata: meta,
                        }),
                    );
                }
            }
        }
    }

    fn tick_deferred_digging(&mut self) {
        let mut ready_breaks = Vec::new();
        for player in self.entities.iter_players() {
            if let Some(pending) = &player.pending_dig {
                let block_id = self.world.get_block(pending.x, pending.y, pending.z);
                if block_id != blocks::AIR {
                    let ticks = self.current_tick.saturating_sub(pending.start_tick) + 1;
                    let dmg = items::calc_block_damage_per_tick(
                        player.inventory.item_in_hand(),
                        block_id,
                        false,
                        player.on_ground,
                    );
                    if dmg * (ticks as f32) >= 1.0 {
                        ready_breaks.push((
                            player.conn_id,
                            player.entity_id,
                            pending.x,
                            pending.y,
                            pending.z,
                        ));
                    }
                }
            }
        }

        for (conn_id, entity_id, x, y, z) in ready_breaks {
            self.finish_block_break(conn_id, entity_id, x, y, z);
        }
    }

    fn finish_block_break(&mut self, id: ConnId, entity_id: i32, x: i32, y: i32, z: i32) {
        let old_block = self.world.get_block(x, y, z);
        let old_meta = self.world.get_data(x, y, z);
        if old_block == blocks::AIR || blocks::hardness(old_block) < 0.0 {
            return;
        }

        let held_item = self
            .entities
            .get_player(entity_id)
            .and_then(|p| p.inventory.item_in_hand());

        // Level sound 2001: block break effect (`block_id + (meta << 8)`)
        let sound_data = (old_block as i32) + ((old_meta as i32) << 8);
        self.broadcast(Clientbound::LevelSound(LevelSound {
            effect_id: 2001,
            x,
            y: y as u8,
            z,
            data: sound_data,
        }));

        // Spill container contents before removing tile entity (`BlockChest.remove`, `BlockFurnace.remove`, `BlockDispenser.remove`, `BlockJukeBox.remove`)
        if let Some(te) = self.world.remove_world_tile_entity(x, y, z) {
            match te.kind {
                TileEntityKind::Chest { items } => {
                    for stack in items.iter().flatten() {
                        self.spawn_dropped_item_random(x as f64, y as f64, z as f64, *stack, 10);
                    }
                }
                TileEntityKind::Furnace { items, .. } => {
                    for stack in items.iter().flatten() {
                        self.spawn_dropped_item_random(x as f64, y as f64, z as f64, *stack, 10);
                    }
                }
                TileEntityKind::Dispenser { items } => {
                    for stack in items.iter().flatten() {
                        self.spawn_dropped_item_random(x as f64, y as f64, z as f64, *stack, 10);
                    }
                }
                TileEntityKind::RecordPlayer { record } if record > 0 => {
                    self.spawn_dropped_item_random(
                        x as f64,
                        y as f64,
                        z as f64,
                        ItemStack::new(record as i16, 1, 0),
                        10,
                    );
                }
                _ => {}
            }
        }

        // Remove multi-part blocks (doors, beds)
        if old_block == blocks::WOODEN_DOOR || old_block == blocks::IRON_DOOR {
            if (old_meta & 8) != 0 {
                if self.world.get_block(x, y - 1, z) == old_block {
                    self.world.java_set_type_id(x, y - 1, z, blocks::AIR);
                    self.send_block_change_to_observers(x, y - 1, z);
                }
            } else if self.world.get_block(x, y + 1, z) == old_block {
                self.world.java_set_type_id(x, y + 1, z, blocks::AIR);
                self.send_block_change_to_observers(x, y + 1, z);
            }
        } else if old_block == blocks::BED {
            let dir = (old_meta & 3) as usize;
            let offsets = [(0, 1), (-1, 0), (0, -1), (1, 0)];
            let (dx, dz) = offsets[dir];
            let (ox, oz) = if (old_meta & 8) != 0 {
                (x - dx, z - dz)
            } else {
                (x + dx, z + dz)
            };
            if self.world.get_block(ox, y, oz) == blocks::BED {
                self.world.java_set_type_id(ox, y, oz, blocks::AIR);
                self.send_block_change_to_observers(ox, y, oz);
            }
        }

        // BlockIce.a: breaking ice above a solid or liquid block turns into flowing water
        if old_block == blocks::ICE && y > 0 {
            let below_mat = blocks::material(self.world.get_block(x, y - 1, z));
            if below_mat.is_solid() || below_mat.is_liquid() {
                self.world.java_set_type_id(x, y, z, blocks::WATER);
            } else {
                self.world.java_set_type_id(x, y, z, blocks::AIR);
            }
        } else {
            self.world.java_set_type_id(x, y, z, blocks::AIR);
        }

        self.send_block_change_to_observers(x, y, z);

        // Spawn block drops if harvestable with the player's current tool
        let harvest_meta = if (old_block == blocks::WOODEN_DOOR
            || old_block == blocks::IRON_DOOR
            || old_block == blocks::BED)
            && (old_meta & 8) != 0
        {
            0
        } else {
            old_meta
        };
        if items::can_harvest_block(held_item, old_block) {
            let mut rng = std::mem::replace(
                &mut self.world.world_random,
                crate::util::rng::JavaRandom::new(0),
            );
            let drops = blocks::get_block_drops(old_block, harvest_meta, &mut rng, held_item);
            self.world.world_random = rng;
            for stack in drops {
                self.spawn_dropped_item_random(x as f64, y as f64, z as f64, stack, 10);
            }
        }

        // Check attached neighbor blocks (torches, ladders, levers, buttons, signs, etc.)
        let attached_drops = self.world.check_attached_blocks_around(x, y, z);
        for (ax, ay, az, stack) in attached_drops {
            self.send_block_change_to_observers(ax, ay, az);
            self.spawn_dropped_item_random(ax as f64, ay as f64, az as f64, stack, 10);
        }

        // Damage tool in hand
        if let Some(player) = self.entities.get_player_mut(entity_id) {
            player.digging = None;
            player.pending_dig = None;
            player.inventory.damage_held_on_block_break(old_block);
            let slot_idx = 36 + player.inventory.held_slot as i16;
            let slot_item = InventoryPlayer::to_slot_item(player.inventory.item_in_hand());
            self.send(
                id,
                Clientbound::SetSlot(SetSlot {
                    window_id: 0,
                    slot: slot_idx,
                    item: slot_item,
                }),
            );
        }
    }

    fn douse_fire(&mut self, mut x: i32, mut y: i32, mut z: i32, face: u8) {
        match face {
            0 => y -= 1,
            1 => y += 1,
            2 => z -= 1,
            3 => z += 1,
            4 => x -= 1,
            5 => x += 1,
            _ => {}
        }
        if self.world.get_block(x, y, z) == blocks::FIRE {
            self.world.java_set_type_id(x, y, z, blocks::AIR);
            self.send_block_change_to_observers(x, y, z);
        }
    }

    fn on_block_left_click(&mut self, id: ConnId, block_id: u8, x: i32, y: i32, z: i32) {
        match block_id {
            blocks::NOTE_BLOCK => {
                self.play_note_at(x, y, z);
            }
            blocks::WOODEN_DOOR | blocks::TRAPDOOR | blocks::LEVER | blocks::STONE_BUTTON => {
                let _ = self.interact_with_block(id, block_id, x, y, z);
            }
            blocks::REDSTONE_ORE => {
                self.world
                    .java_set_type_id(x, y, z, blocks::LIT_REDSTONE_ORE);
                self.send_block_change_to_observers(x, y, z);
            }
            _ => {}
        }
    }

    fn play_note_at(&mut self, x: i32, y: i32, z: i32) {
        if self.world.get_block(x, y + 1, z) != blocks::AIR {
            return;
        }
        let pitch = match self.world.get_tile_entity(x, y, z) {
            Some(TileEntity {
                kind: TileEntityKind::Note { note },
                ..
            }) => *note,
            _ => 0,
        };
        let below_mat = blocks::material(self.world.get_block(x, y - 1, z));
        let instrument = match below_mat {
            blocks::Material::Stone => 1,
            blocks::Material::Sand => 2,
            blocks::Material::Shatterable => 3,
            blocks::Material::Wood => 4,
            _ => 0,
        };
        self.broadcast(Clientbound::PlayNoteBlock(PlayNoteBlock {
            x,
            y: y as i16,
            z,
            instrument,
            pitch,
        }));
    }

    fn interact_with_block(&mut self, id: ConnId, block_id: u8, x: i32, y: i32, z: i32) -> bool {
        match block_id {
            blocks::WORKBENCH => {
                self.close_player_container_and_drop_transient(id, false);
                let win_id = match self.entities.get_by_conn_mut(id) {
                    Some(p) => {
                        let w = p.next_window_id();
                        p.open_container = Some(OpenContainer::Workbench {
                            window_id: w,
                            x,
                            y,
                            z,
                            craft_grid: [None; 9],
                            craft_result: None,
                        });
                        w
                    }
                    None => return true,
                };
                self.send(
                    id,
                    Clientbound::OpenWindow(OpenWindow {
                        window_id: win_id,
                        inventory_type: 1,
                        title: "Crafting".to_string(),
                        slots_count: 9,
                    }),
                );
                if let Some(p) = self.entities.get_by_conn(id) {
                    if let Some(oc) = &p.open_container {
                        let items = p.inventory.container_window_items(oc);
                        self.send(
                            id,
                            Clientbound::WindowItems(WindowItems {
                                window_id: win_id,
                                items,
                            }),
                        );
                    }
                }
                true
            }
            blocks::CHEST => {
                if self.world.is_solid_normal_at(x, y + 1, z) {
                    return true;
                }
                let mut positions = vec![(x, y, z)];
                if self.world.get_block(x - 1, y, z) == blocks::CHEST {
                    if self.world.is_solid_normal_at(x - 1, y + 1, z) {
                        return true;
                    }
                    positions = vec![(x - 1, y, z), (x, y, z)];
                } else if self.world.get_block(x + 1, y, z) == blocks::CHEST {
                    if self.world.is_solid_normal_at(x + 1, y + 1, z) {
                        return true;
                    }
                    positions = vec![(x, y, z), (x + 1, y, z)];
                } else if self.world.get_block(x, y, z - 1) == blocks::CHEST {
                    if self.world.is_solid_normal_at(x, y + 1, z - 1) {
                        return true;
                    }
                    positions = vec![(x, y, z - 1), (x, y, z)];
                } else if self.world.get_block(x, y, z + 1) == blocks::CHEST {
                    if self.world.is_solid_normal_at(x, y + 1, z + 1) {
                        return true;
                    }
                    positions = vec![(x, y, z), (x, y, z + 1)];
                }

                let mut slots = Vec::with_capacity(positions.len() * 27);
                for &(cx, cy, cz) in &positions {
                    if self.world.get_tile_entity(cx, cy, cz).is_none() {
                        self.world
                            .set_world_tile_entity(TileEntity::new_chest(cx, cy, cz));
                    }
                    if let Some(TileEntity {
                        kind: TileEntityKind::Chest { items },
                        ..
                    }) = self.world.get_tile_entity(cx, cy, cz)
                    {
                        slots.extend_from_slice(items.as_ref());
                    } else {
                        slots.resize(slots.len() + 27, None);
                    }
                }

                self.close_player_container_and_drop_transient(id, false);
                let slots_count = slots.len() as u8;
                let title = if positions.len() > 1 {
                    "Large chest".to_string()
                } else {
                    "Chest".to_string()
                };
                let win_id = match self.entities.get_by_conn_mut(id) {
                    Some(p) => {
                        let w = p.next_window_id();
                        p.open_container = Some(OpenContainer::Chest {
                            window_id: w,
                            positions,
                            slots,
                        });
                        w
                    }
                    None => return true,
                };
                self.send(
                    id,
                    Clientbound::OpenWindow(OpenWindow {
                        window_id: win_id,
                        inventory_type: 0,
                        title,
                        slots_count,
                    }),
                );
                if let Some(p) = self.entities.get_by_conn(id) {
                    if let Some(oc) = &p.open_container {
                        let items = p.inventory.container_window_items(oc);
                        self.send(
                            id,
                            Clientbound::WindowItems(WindowItems {
                                window_id: win_id,
                                items,
                            }),
                        );
                    }
                }
                true
            }
            blocks::FURNACE | blocks::LIT_FURNACE => {
                if self.world.get_tile_entity(x, y, z).is_none() {
                    self.world
                        .set_world_tile_entity(TileEntity::new_furnace(x, y, z));
                }
                let (slots, cook_time, burn_time, ticks_for_current_fuel) =
                    match self.world.get_tile_entity(x, y, z) {
                        Some(TileEntity {
                            kind:
                                TileEntityKind::Furnace {
                                    burn_time,
                                    ticks_for_current_fuel,
                                    cook_time,
                                    items,
                                },
                            ..
                        }) => (**items, *cook_time, *burn_time, *ticks_for_current_fuel),
                        _ => ([None; 3], 0, 0, 0),
                    };

                self.close_player_container_and_drop_transient(id, false);
                let win_id = match self.entities.get_by_conn_mut(id) {
                    Some(p) => {
                        let w = p.next_window_id();
                        p.open_container = Some(OpenContainer::Furnace {
                            window_id: w,
                            x,
                            y,
                            z,
                            slots,
                            cook_time,
                            burn_time,
                            ticks_for_current_fuel,
                        });
                        w
                    }
                    None => return true,
                };
                self.send(
                    id,
                    Clientbound::OpenWindow(OpenWindow {
                        window_id: win_id,
                        inventory_type: 2,
                        title: "Furnace".to_string(),
                        slots_count: 3,
                    }),
                );
                if let Some(p) = self.entities.get_by_conn(id) {
                    if let Some(oc) = &p.open_container {
                        let items = p.inventory.container_window_items(oc);
                        self.send(
                            id,
                            Clientbound::WindowItems(WindowItems {
                                window_id: win_id,
                                items,
                            }),
                        );
                    }
                }
                self.send(
                    id,
                    Clientbound::CraftProgressBar(CraftProgressBar {
                        window_id: win_id,
                        property_id: 0,
                        value: cook_time,
                    }),
                );
                self.send(
                    id,
                    Clientbound::CraftProgressBar(CraftProgressBar {
                        window_id: win_id,
                        property_id: 1,
                        value: burn_time,
                    }),
                );
                self.send(
                    id,
                    Clientbound::CraftProgressBar(CraftProgressBar {
                        window_id: win_id,
                        property_id: 2,
                        value: ticks_for_current_fuel,
                    }),
                );
                true
            }
            blocks::DISPENSER => {
                if self.world.get_tile_entity(x, y, z).is_none() {
                    self.world
                        .set_world_tile_entity(TileEntity::new_dispenser(x, y, z));
                }
                let slots = match self.world.get_tile_entity(x, y, z) {
                    Some(TileEntity {
                        kind: TileEntityKind::Dispenser { items },
                        ..
                    }) => **items,
                    _ => [None; 9],
                };
                self.close_player_container_and_drop_transient(id, false);
                let win_id = match self.entities.get_by_conn_mut(id) {
                    Some(p) => {
                        let w = p.next_window_id();
                        p.open_container = Some(OpenContainer::Dispenser {
                            window_id: w,
                            x,
                            y,
                            z,
                            slots,
                        });
                        w
                    }
                    None => return true,
                };
                self.send(
                    id,
                    Clientbound::OpenWindow(OpenWindow {
                        window_id: win_id,
                        inventory_type: 3,
                        title: "Trap".to_string(),
                        slots_count: 9,
                    }),
                );
                if let Some(p) = self.entities.get_by_conn(id) {
                    if let Some(oc) = &p.open_container {
                        let items = p.inventory.container_window_items(oc);
                        self.send(
                            id,
                            Clientbound::WindowItems(WindowItems {
                                window_id: win_id,
                                items,
                            }),
                        );
                    }
                }
                true
            }
            blocks::WOODEN_DOOR => {
                let mut by = y;
                let mut data = self.world.get_data(x, y, z);
                if (data & 8) != 0 {
                    if self.world.get_block(x, y - 1, z) == blocks::WOODEN_DOOR {
                        by = y - 1;
                        data = self.world.get_data(x, by, z);
                    } else {
                        return true;
                    }
                }
                let toggled = data ^ 4;
                self.world.java_set_data(x, by, z, toggled);
                self.send_block_change_to_observers(x, by, z);
                if self.world.get_block(x, by + 1, z) == blocks::WOODEN_DOOR {
                    self.world.java_set_data(x, by + 1, z, toggled + 8);
                    self.send_block_change_to_observers(x, by + 1, z);
                }
                self.broadcast(Clientbound::LevelSound(LevelSound {
                    effect_id: 1003,
                    x,
                    y: by as u8,
                    z,
                    data: 0,
                }));
                true
            }
            blocks::IRON_DOOR => true,
            blocks::TRAPDOOR => {
                let data = self.world.get_data(x, y, z);
                self.world.java_set_data(x, y, z, data ^ 4);
                self.send_block_change_to_observers(x, y, z);
                self.broadcast(Clientbound::LevelSound(LevelSound {
                    effect_id: 1003,
                    x,
                    y: y as u8,
                    z,
                    data: 0,
                }));
                true
            }
            blocks::LEVER | blocks::STONE_BUTTON => {
                let data = self.world.get_data(x, y, z);
                let toggled = (data & 7) | (8 - (data & 8));
                self.world.java_set_data(x, y, z, toggled);
                self.send_block_change_to_observers(x, y, z);
                true
            }
            blocks::REPEATER_OFF | blocks::REPEATER_ON => {
                let data = self.world.get_data(x, y, z);
                let delay = (((data & 12) >> 2) + 1) & 3;
                let new_data = (data & 3) | (delay << 2);
                self.world.java_set_data(x, y, z, new_data);
                self.send_block_change_to_observers(x, y, z);
                true
            }
            blocks::NOTE_BLOCK => {
                if self.world.get_tile_entity(x, y, z).is_none() {
                    self.world
                        .set_world_tile_entity(TileEntity::new_note(x, y, z));
                }
                if let Some(TileEntity {
                    kind: TileEntityKind::Note { note },
                    ..
                }) = self.world.get_tile_entity_mut(x, y, z)
                {
                    *note = (*note + 1) % 25;
                }
                self.play_note_at(x, y, z);
                true
            }
            blocks::JUKEBOX => {
                let data = self.world.get_data(x, y, z);
                if data != 0 {
                    // Eject current record
                    let mut ejected = 0;
                    if let Some(TileEntity {
                        kind: TileEntityKind::RecordPlayer { record },
                        ..
                    }) = self.world.get_tile_entity_mut(x, y, z)
                    {
                        ejected = *record;
                        *record = 0;
                    }
                    self.world.java_set_data(x, y, z, 0);
                    self.send_block_change_to_observers(x, y, z);
                    self.broadcast(Clientbound::LevelSound(LevelSound {
                        effect_id: 1005,
                        x,
                        y: y as u8,
                        z,
                        data: 0,
                    }));
                    if ejected > 0 {
                        self.spawn_dropped_item_random(
                            x as f64,
                            y as f64 + 0.6,
                            z as f64,
                            ItemStack::new(ejected as i16, 1, 0),
                            10,
                        );
                    }
                    true
                } else {
                    false
                }
            }
            blocks::CAKE => {
                let healed = if let Some(player) = self.entities.get_by_conn_mut(id) {
                    if player.health < 20 {
                        player.heal(3);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if healed {
                    let slices = self.world.get_data(x, y, z) + 1;
                    if slices >= 6 {
                        self.world.java_set_type_id(x, y, z, blocks::AIR);
                    } else {
                        self.world.java_set_data(x, y, z, slices);
                    }
                    self.send_block_change_to_observers(x, y, z);
                }
                true
            }
            blocks::BED => {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.spawn_bed = Some((x, y, z));
                }
                true
            }
            blocks::REDSTONE_ORE => {
                self.world
                    .java_set_type_id(x, y, z, blocks::LIT_REDSTONE_ORE);
                self.send_block_change_to_observers(x, y, z);
                false
            }
            _ => false,
        }
    }

    fn handle_place(&mut self, id: ConnId, place: crate::net::packets::Place) {
        if place.direction == 255 {
            // Deduplicate Notch's double Packet15Place (block click immediately followed by face=255)
            let is_dup = if let Some(player) = self.entities.get_by_conn_mut(id) {
                if player.dead {
                    return;
                }
                if let Some((last_tick, last_item)) = player.last_place_item {
                    if last_item == place.item_id
                        && self.current_tick.saturating_sub(last_tick) <= 2
                    {
                        player.last_place_item = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                return;
            };
            if is_dup {
                return;
            }

            // Right-click air: food / soup / milk bucket
            let slot_update = if let Some(player) = self.entities.get_by_conn_mut(id) {
                if let Some(mut stack) = player.inventory.item_in_hand() {
                    if stack.is_food() {
                        let heal = stack.food_heal_amount();
                        player.heal(heal);
                        let new_stack = if stack.id == items::MUSHROOM_SOUP {
                            Some(ItemStack::new(items::BOWL, 1, 0))
                        } else {
                            stack.count = stack.count.saturating_sub(1);
                            if stack.count > 0 {
                                Some(stack)
                            } else {
                                None
                            }
                        };
                        player.inventory.set_item_in_hand(new_stack);
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

        let (player_x, player_y, player_z, yaw, pitch, sneaking, held_item) =
            match self.entities.get_by_conn_mut(id) {
                Some(p) => {
                    if p.dead {
                        return;
                    }
                    p.last_place_item = Some((self.current_tick, place.item_id));
                    (
                        p.x,
                        p.y,
                        p.z,
                        p.yaw,
                        p.pitch,
                        p.sneaking,
                        p.inventory.item_in_hand(),
                    )
                }
                None => return,
            };

        let cx = place.x;
        let cy = place.y as i32;
        let cz = place.z;
        let face = place.direction;

        // Eye reach distance check (`<= 36.0`)
        let dx = player_x - (cx as f64);
        let dy = (player_y + 1.62) - (cy as f64);
        let dz = player_z - (cz as f64);
        if dx * dx + dy * dy + dz * dz > 36.0 {
            return;
        }

        let clicked_block = self.world.get_block(cx, cy, cz);
        if clicked_block > 0
            && (!sneaking || held_item.is_none())
            && self.interact_with_block(id, clicked_block, cx, cy, cz)
        {
            return;
        }

        let (mut tx, mut ty, mut tz) = if clicked_block == blocks::SNOW {
            (cx, cy, cz)
        } else {
            match face {
                0 => (cx, cy - 1, cz),
                1 => (cx, cy + 1, cz),
                2 => (cx, cy, cz - 1),
                3 => (cx, cy, cz + 1),
                4 => (cx - 1, cy, cz),
                5 => (cx + 1, cy, cz),
                _ => (cx, cy, cz),
            }
        };

        // Spawn protection check
        let spawn_x = self.config.world.spawn_x;
        let spawn_z = self.config.world.spawn_z;
        let radius = self.config.world.spawn_radius;
        let chebyshev = (tx - spawn_x)
            .abs()
            .max((tz - spawn_z).abs())
            .min((cx - spawn_x).abs().max((cz - spawn_z).abs()));
        if chebyshev <= radius {
            self.send_block_change_to_player(id, cx, cy, cz);
            self.send_block_change_to_player(id, tx, ty, tz);
            return;
        }

        // Determine active item (prefer server inventory, fallback to packet item if inventory is empty)
        let active_item = held_item.or_else(|| {
            if place.item_id > 0 {
                Some(ItemStack::new(
                    place.item_id,
                    place.count.unwrap_or(1),
                    place.damage.unwrap_or(0),
                ))
            } else {
                None
            }
        });
        let Some(stack) = active_item else {
            self.send_block_change_to_player(id, cx, cy, cz);
            self.send_block_change_to_player(id, tx, ty, tz);
            return;
        };

        let mut consumed = false;
        let mut durability_cost = 0i16;
        let mut replacement_item: Option<ItemStack> = None;

        match stack.id {
            // Hoes: till dirt or grass into farmland (60)
            items::WOOD_HOE
            | items::STONE_HOE
            | items::IRON_HOE
            | items::DIAMOND_HOE
            | items::GOLD_HOE => {
                let above = self.world.get_block(cx, cy + 1, cz);
                if (face != 0 && above == blocks::AIR && clicked_block == blocks::GRASS)
                    || clicked_block == blocks::DIRT
                {
                    self.world.java_set_type_id(cx, cy, cz, blocks::FARMLAND);
                    self.send_block_change_to_observers(cx, cy, cz);
                    durability_cost = 1;
                }
            }
            // Seeds: plant crops (59) on top of farmland (60)
            items::SEEDS => {
                if face == 1
                    && clicked_block == blocks::FARMLAND
                    && self.world.get_block(cx, cy + 1, cz) == blocks::AIR
                {
                    self.world.java_set_type_id(cx, cy + 1, cz, blocks::CROPS);
                    self.send_block_change_to_observers(cx, cy + 1, cz);
                    consumed = true;
                }
            }
            // Sugar cane (338 -> block 83), Cake (354 -> block 92), Diode/Repeater (356 -> block 93), Redstone dust (331 -> block 55)
            items::SUGAR_CANE | items::CAKE | items::DIODE | items::REDSTONE => {
                let target_block = match stack.id {
                    items::SUGAR_CANE => blocks::REED,
                    items::CAKE => blocks::CAKE,
                    items::DIODE => blocks::REPEATER_OFF,
                    _ => blocks::REDSTONE_WIRE,
                };
                if self
                    .world
                    .can_place_block_at(target_block, tx, ty, tz, face)
                {
                    self.world.java_set_type_id(tx, ty, tz, target_block);
                    self.world
                        .post_place_block(target_block, tx, ty, tz, face, yaw, pitch);
                    self.send_block_change_to_observers(tx, ty, tz);
                    consumed = true;
                }
            }
            // Sign item (323 -> Sign post 63 or Wall sign 68)
            items::SIGN => {
                if face != 0
                    && blocks::material(clicked_block).is_buildable()
                    && (0..128).contains(&ty)
                {
                    let existing = self.world.get_block(tx, ty, tz);
                    if existing == blocks::AIR || blocks::material(existing).is_replaceable() {
                        if face == 1 {
                            let rot =
                                (((yaw + 180.0) * 16.0 / 360.0 + 0.5).floor() as i32 & 15) as u8;
                            self.world
                                .java_set_type_id_and_data(tx, ty, tz, blocks::SIGN, rot);
                        } else {
                            self.world.java_set_type_id_and_data(
                                tx,
                                ty,
                                tz,
                                blocks::WALL_SIGN,
                                face,
                            );
                        }
                        self.world.set_world_tile_entity(TileEntity::new_sign(
                            tx,
                            ty,
                            tz,
                            [String::new(), String::new(), String::new(), String::new()],
                        ));
                        if let Some(p) = self.entities.get_by_conn_mut(id) {
                            p.editing_sign = Some((tx, ty, tz));
                        }
                        self.send_block_change_to_observers(tx, ty, tz);
                        consumed = true;
                    }
                }
            }
            // Wooden door (324 -> block 64) & Iron door (330 -> block 71)
            items::WOOD_DOOR | items::IRON_DOOR => {
                let door_id = if stack.id == items::WOOD_DOOR {
                    blocks::WOODEN_DOOR
                } else {
                    blocks::IRON_DOOR
                };
                if face == 1
                    && ty < 127
                    && self.world.is_solid_normal_at(tx, ty - 1, tz)
                    && self.world.can_place_block_at(door_id, tx, ty, tz, face)
                    && self.world.can_place_block_at(door_id, tx, ty + 1, tz, face)
                {
                    let dir = (((yaw + 180.0) * 4.0 / 360.0 - 0.5).floor() as i32 & 3) as u8;
                    self.world
                        .java_set_type_id_and_data(tx, ty, tz, door_id, dir);
                    self.world
                        .java_set_type_id_and_data(tx, ty + 1, tz, door_id, dir + 8);
                    self.send_block_change_to_observers(tx, ty, tz);
                    self.send_block_change_to_observers(tx, ty + 1, tz);
                    consumed = true;
                }
            }
            // Bed item (355 -> block 26)
            items::BED => {
                if face == 1 && ty < 127 {
                    let dir = ((yaw * 4.0 / 360.0 + 0.5).floor() as i32 & 3) as usize;
                    let offsets = [(0, 1), (-1, 0), (0, -1), (1, 0)];
                    let (bx, bz) = offsets[dir];
                    let hx = tx + bx;
                    let hz = tz + bz;
                    if self.world.get_block(tx, ty, tz) == blocks::AIR
                        && self.world.get_block(hx, ty, hz) == blocks::AIR
                        && self.world.is_solid_normal_at(tx, ty - 1, tz)
                        && self.world.is_solid_normal_at(hx, ty - 1, hz)
                    {
                        self.world
                            .java_set_type_id_and_data(tx, ty, tz, blocks::BED, dir as u8);
                        self.world.java_set_type_id_and_data(
                            hx,
                            ty,
                            hz,
                            blocks::BED,
                            (dir as u8) + 8,
                        );
                        self.send_block_change_to_observers(tx, ty, tz);
                        self.send_block_change_to_observers(hx, ty, hz);
                        consumed = true;
                    }
                }
            }
            // Flint and steel (259 -> Fire 51)
            items::FLINT_AND_STEEL => {
                if self.world.get_block(tx, ty, tz) == blocks::AIR {
                    self.world.java_set_type_id(tx, ty, tz, blocks::FIRE);
                    self.send_block_change_to_observers(tx, ty, tz);
                }
                durability_cost = 1;
            }
            // Empty bucket (325 -> collect stationary/source water or lava)
            items::BUCKET => {
                let target_b = self.world.get_block(tx, ty, tz);
                let target_m = self.world.get_data(tx, ty, tz);
                let (fx, fy, fz, fb, fm) =
                    if (blocks::is_water(target_b) || blocks::is_lava(target_b)) && target_m == 0 {
                        (tx, ty, tz, target_b, target_m)
                    } else {
                        (cx, cy, cz, clicked_block, self.world.get_data(cx, cy, cz))
                    };
                if blocks::is_water(fb) && fm == 0 {
                    self.world.java_set_type_id(fx, fy, fz, blocks::AIR);
                    self.send_block_change_to_observers(fx, fy, fz);
                    replacement_item = Some(ItemStack::new(items::WATER_BUCKET, 1, 0));
                } else if blocks::is_lava(fb) && fm == 0 {
                    self.world.java_set_type_id(fx, fy, fz, blocks::AIR);
                    self.send_block_change_to_observers(fx, fy, fz);
                    replacement_item = Some(ItemStack::new(items::LAVA_BUCKET, 1, 0));
                }
            }
            // Water bucket (326) & Lava bucket (327)
            items::WATER_BUCKET | items::LAVA_BUCKET => {
                let existing = self.world.get_block(tx, ty, tz);
                if (0..128).contains(&ty)
                    && (existing == blocks::AIR || !blocks::material(existing).is_buildable())
                {
                    let fluid = if stack.id == items::WATER_BUCKET {
                        blocks::WATER
                    } else {
                        blocks::LAVA
                    };
                    self.world.java_set_type_id_and_data(tx, ty, tz, fluid, 0);
                    self.send_block_change_to_observers(tx, ty, tz);
                    replacement_item = Some(ItemStack::new(items::BUCKET, 1, 0));
                }
            }
            // Bone meal (Ink Sack 351 with damage 15)
            items::INK_SACK if stack.damage == 15 => match clicked_block {
                blocks::SAPLING => {
                    let _ = self.world.grow_sapling_tree(cx, cy, cz);
                    consumed = true;
                }
                blocks::CROPS => {
                    self.world.java_set_data(cx, cy, cz, 7);
                    self.send_block_change_to_observers(cx, cy, cz);
                    consumed = true;
                }
                blocks::GRASS => {
                    self.world.apply_bonemeal_to_grass(cx, cy, cz);
                    consumed = true;
                }
                _ => {}
            },
            // Records (2256, 2257) into Jukebox (84)
            items::GOLD_RECORD | items::GREEN_RECORD => {
                if clicked_block == blocks::JUKEBOX && self.world.get_data(cx, cy, cz) == 0 {
                    if self.world.get_tile_entity(cx, cy, cz).is_none() {
                        self.world
                            .set_world_tile_entity(TileEntity::new_record_player(cx, cy, cz));
                    }
                    if let Some(TileEntity {
                        kind: TileEntityKind::RecordPlayer { record },
                        ..
                    }) = self.world.get_tile_entity_mut(cx, cy, cz)
                    {
                        *record = stack.id as i32;
                    }
                    self.world.java_set_data(cx, cy, cz, 1);
                    self.send_block_change_to_observers(cx, cy, cz);
                    self.broadcast(Clientbound::LevelSound(LevelSound {
                        effect_id: 1005,
                        x: cx,
                        y: cy as u8,
                        z: cz,
                        data: stack.id as i32,
                    }));
                    consumed = true;
                }
            }
            // Standard ItemBlock (1..=96)
            1..=96 => {
                let block_id = stack.id as u8;
                // Check if clicking top of an existing bottom slab (44) with matching slab
                if block_id == blocks::STEP
                    && face == 1
                    && clicked_block == blocks::STEP
                    && self.world.get_data(cx, cy, cz)
                        == items::filter_block_data(stack.id, stack.damage)
                {
                    tx = cx;
                    ty = cy;
                    tz = cz;
                    let meta = self.world.get_data(cx, cy, cz);
                    self.world
                        .java_set_type_id_and_data(tx, ty, tz, blocks::DOUBLE_STEP, meta);
                    self.send_block_change_to_observers(tx, ty, tz);
                    consumed = true;
                } else if !(ty == 127 && blocks::material(block_id).is_buildable())
                    && self.world.can_place_block_at(block_id, tx, ty, tz, face)
                    && !self.block_intersects_any_player(block_id, tx, ty, tz)
                {
                    let meta = items::filter_block_data(stack.id, stack.damage);
                    self.world
                        .java_set_type_id_and_data(tx, ty, tz, block_id, meta);
                    self.world
                        .post_place_block(block_id, tx, ty, tz, face, yaw, pitch);
                    self.send_block_change_to_observers(tx, ty, tz);
                    if ty > 0 {
                        self.send_block_change_to_observers(tx, ty - 1, tz);
                    }
                    consumed = true;
                }
            }
            _ => {}
        }

        // Always confirm clicked and adjacent blocks to the placing client (`NetServerHandler.java:668, 693`)
        self.send_block_change_to_player(id, cx, cy, cz);
        self.send_block_change_to_player(id, tx, ty, tz);

        // Update player's held item if consumed, damaged, or replaced
        if consumed || durability_cost > 0 || replacement_item.is_some() {
            let slot_update = if let Some(player) = self.entities.get_by_conn_mut(id) {
                let slot_idx = 36 + player.inventory.held_slot as i16;
                if let Some(rep) = replacement_item {
                    player.inventory.set_item_in_hand(Some(rep));
                } else if durability_cost > 0 {
                    player.inventory.damage_held_item(durability_cost);
                } else if consumed {
                    if let Some(mut cur) = player.inventory.item_in_hand() {
                        cur.count = cur.count.saturating_sub(1);
                        player.inventory.set_item_in_hand(if cur.count > 0 {
                            Some(cur)
                        } else {
                            None
                        });
                    }
                }
                let item = InventoryPlayer::to_slot_item(player.inventory.item_in_hand());
                Some((slot_idx, item))
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

    fn block_intersects_any_player(&self, block_id: u8, x: i32, y: i32, z: i32) -> bool {
        if !blocks::is_solid(block_id) {
            return false;
        }
        let bx0 = x as f64;
        let by0 = y as f64;
        let bz0 = z as f64;
        let bx1 = bx0 + 1.0;
        let by1 = if block_id == blocks::STEP {
            by0 + 0.5
        } else {
            by0 + 1.0
        };
        let bz1 = bz0 + 1.0;

        for p in self.entities.iter_players() {
            if p.dead {
                continue;
            }
            let px0 = p.x - 0.3;
            let px1 = p.x + 0.3;
            let py0 = p.y;
            let py1 = p.y + 1.8;
            let pz0 = p.z - 0.3;
            let pz1 = p.z + 0.3;
            if px1 > bx0 && px0 < bx1 && py1 > by0 && py0 < by1 && pz1 > bz0 && pz0 < bz1 {
                return true;
            }
        }
        false
    }

    fn send_block_change_to_player(&self, id: ConnId, x: i32, y: i32, z: i32) {
        if !(0..128).contains(&y) {
            return;
        }
        let block = self.world.get_block(x, y, z);
        let meta = self.world.get_data(x, y, z);
        self.send(
            id,
            Clientbound::BlockChange(BlockChange {
                x,
                y: y as u8,
                z,
                block_type: block,
                metadata: meta,
            }),
        );
    }

    fn send_block_change_to_observers(&self, x: i32, y: i32, z: i32) {
        if !(0..128).contains(&y) {
            return;
        }
        let block = self.world.get_block(x, y, z);
        let meta = self.world.get_data(x, y, z);
        let change = Clientbound::BlockChange(BlockChange {
            x,
            y: y as u8,
            z,
            block_type: block,
            metadata: meta,
        });
        let cpos = ChunkPos::new(x >> 4, z >> 4);
        for p in self.entities.iter_players() {
            if p.sent_chunks.contains(&cpos) {
                self.send(p.conn_id, change.clone());
            }
        }
    }

    fn handle_window_click(&mut self, id: ConnId, click: crate::net::packets::WindowClick) {
        let mut dropped_items = Vec::new();
        let player_pos;
        let mut window_items_sync = None;
        let mut cursor_item = None;
        let mut craft_slot_0 = None;
        let mut clicked_slot_item = None;
        let mut wb_writeback = None;

        if let Some(player) = self.entities.get_by_conn_mut(id) {
            if player.dead {
                return;
            }
            player_pos = (player.x, player.y, player.z, player.yaw, player.pitch);

            if click.window_id == 0 {
                let outcome =
                    player
                        .inventory
                        .handle_window_click(click.slot, click.button, click.shift);
                dropped_items = outcome.dropped;
                if click.slot >= 0 {
                    clicked_slot_item =
                        InventoryPlayer::to_slot_item(player.inventory.get_window_slot(click.slot));
                }
                craft_slot_0 = InventoryPlayer::to_slot_item(player.inventory.craft_result);
                cursor_item = InventoryPlayer::to_slot_item(player.inventory.cursor);
                if click.shift || click.slot == 0 {
                    window_items_sync = Some((0u8, player.inventory.to_window_items()));
                }
            } else if let Some(mut container) = player.open_container.take() {
                if container.window_id() == click.window_id {
                    let outcome = player.inventory.handle_container_click(
                        &mut container,
                        click.slot,
                        click.button,
                        click.shift,
                    );
                    dropped_items = outcome.dropped;
                    if click.slot >= 0 {
                        clicked_slot_item = InventoryPlayer::to_slot_item(
                            player
                                .inventory
                                .get_open_container_slot(&container, click.slot),
                        );
                    }
                    if let OpenContainer::Workbench { craft_result, .. } = &container {
                        craft_slot_0 = InventoryPlayer::to_slot_item(*craft_result);
                    }
                    cursor_item = InventoryPlayer::to_slot_item(player.inventory.cursor);
                    window_items_sync = Some((
                        click.window_id,
                        player.inventory.container_window_items(&container),
                    ));
                    wb_writeback = Some(container.clone());
                }
                player.open_container = Some(container);
            }
        } else {
            return;
        }

        // Write modified container slots back to the world's TileEntity if applicable
        if let Some(container) = wb_writeback {
            self.write_back_open_container(&container);
        }

        for stack in dropped_items {
            self.spawn_thrown_item(
                player_pos.0,
                player_pos.1,
                player_pos.2,
                player_pos.3,
                player_pos.4,
                stack,
            );
        }

        // Confirm transaction
        self.send(
            id,
            Clientbound::Transaction(Transaction {
                window_id: click.window_id,
                action_number: click.action_number,
                accepted: true,
            }),
        );

        if let Some((win_id, items)) = window_items_sync {
            self.send(
                id,
                Clientbound::WindowItems(WindowItems {
                    window_id: win_id,
                    items,
                }),
            );
        } else {
            if click.slot >= 0 {
                self.send(
                    id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: click.window_id as i8,
                        slot: click.slot,
                        item: clicked_slot_item,
                    }),
                );
            }
            if (1..=4).contains(&click.slot) && click.window_id == 0 {
                self.send(
                    id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: 0,
                        slot: 0,
                        item: craft_slot_0,
                    }),
                );
            }
        }

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

    fn write_back_open_container(&mut self, container: &OpenContainer) {
        match container {
            OpenContainer::Chest {
                positions, slots, ..
            } => {
                for (idx, &(cx, cy, cz)) in positions.iter().enumerate() {
                    if let Some(TileEntity {
                        kind: TileEntityKind::Chest { items },
                        ..
                    }) = self.world.get_tile_entity_mut(cx, cy, cz)
                    {
                        let start = idx * 27;
                        for s in 0..27 {
                            if start + s < slots.len() {
                                items[s] = slots[start + s];
                            }
                        }
                    }
                }
            }
            OpenContainer::Furnace { x, y, z, slots, .. } => {
                if let Some(TileEntity {
                    kind: TileEntityKind::Furnace { items, .. },
                    ..
                }) = self.world.get_tile_entity_mut(*x, *y, *z)
                {
                    **items = *slots;
                }
            }
            OpenContainer::Dispenser { x, y, z, slots, .. } => {
                if let Some(TileEntity {
                    kind: TileEntityKind::Dispenser { items },
                    ..
                }) = self.world.get_tile_entity_mut(*x, *y, *z)
                {
                    **items = *slots;
                }
            }
            OpenContainer::Workbench { .. } => {}
        }
    }

    fn sync_open_containers(&mut self) {
        let mut updates = Vec::new();
        for player in self.entities.iter_players_mut() {
            if let Some(OpenContainer::Furnace {
                window_id,
                x,
                y,
                z,
                slots,
                cook_time,
                burn_time,
                ticks_for_current_fuel,
            }) = &mut player.open_container
            {
                if let Some(TileEntity {
                    kind:
                        TileEntityKind::Furnace {
                            burn_time: te_burn,
                            ticks_for_current_fuel: te_fuel,
                            cook_time: te_cook,
                            items: te_items,
                        },
                    ..
                }) = self.world.get_tile_entity(*x, *y, *z)
                {
                    let mut bars = Vec::new();
                    if *cook_time != *te_cook {
                        *cook_time = *te_cook;
                        bars.push((0i16, *te_cook));
                    }
                    if *burn_time != *te_burn {
                        *burn_time = *te_burn;
                        bars.push((1i16, *te_burn));
                    }
                    if *ticks_for_current_fuel != *te_fuel {
                        *ticks_for_current_fuel = *te_fuel;
                        bars.push((2i16, *te_fuel));
                    }
                    let mut slot_changes = Vec::new();
                    for i in 0..3 {
                        if slots[i] != te_items[i] {
                            slots[i] = te_items[i];
                            slot_changes
                                .push((i as i16, InventoryPlayer::to_slot_item(te_items[i])));
                        }
                    }
                    if !bars.is_empty() || !slot_changes.is_empty() {
                        updates.push((player.conn_id, *window_id, bars, slot_changes));
                    }
                }
            }
        }

        for (conn_id, win_id, bars, slot_changes) in updates {
            for (prop, val) in bars {
                self.send(
                    conn_id,
                    Clientbound::CraftProgressBar(CraftProgressBar {
                        window_id: win_id,
                        property_id: prop,
                        value: val,
                    }),
                );
            }
            for (slot, item) in slot_changes {
                self.send(
                    conn_id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: win_id as i8,
                        slot,
                        item,
                    }),
                );
            }
        }
    }

    fn close_player_container_and_drop_transient(&mut self, id: ConnId, sync_client: bool) {
        let mut to_drop = Vec::new();
        let mut pos = (0.0, 64.0, 0.0, 0.0, 0.0);
        let mut window_items = None;

        if let Some(player) = self.entities.get_by_conn_mut(id) {
            pos = (player.x, player.y, player.z, player.yaw, player.pitch);
            if let Some(OpenContainer::Workbench { craft_grid, .. }) = player.open_container.take()
            {
                for item in craft_grid.into_iter().flatten() {
                    to_drop.push(item);
                }
            }
            for item in player.inventory.drain_transient_items() {
                to_drop.push(item);
            }
            if sync_client && !to_drop.is_empty() {
                window_items = Some(player.inventory.to_window_items());
            }
        }

        for item in to_drop {
            self.spawn_thrown_item(pos.0, pos.1, pos.2, pos.3, pos.4, item);
        }

        if let Some(items) = window_items {
            self.send(
                id,
                Clientbound::WindowItems(WindowItems {
                    window_id: 0,
                    items,
                }),
            );
            self.send(
                id,
                Clientbound::SetSlot(SetSlot {
                    window_id: -1,
                    slot: -1,
                    item: None,
                }),
            );
        }
    }

    fn handle_update_sign(&mut self, id: ConnId, mut sign: UpdateSign) {
        let x = sign.x;
        let y = sign.y as i32;
        let z = sign.z;
        if !self.world.is_chunk_loaded(ChunkPos::new(x >> 4, z >> 4)) {
            return;
        }

        for line in &mut sign.lines {
            if line.chars().count() > 15 || !line.chars().all(is_allowed_char) {
                *line = "!?".to_string();
            }
        }

        if let Some(TileEntity {
            kind: TileEntityKind::Sign { lines },
            ..
        }) = self.world.get_tile_entity_mut(x, y, z)
        {
            *lines = sign.lines.clone();
        }
        if let Some(p) = self.entities.get_by_conn_mut(id) {
            p.editing_sign = None;
        }

        let cpos = ChunkPos::new(x >> 4, z >> 4);
        let pkt = Clientbound::UpdateSign(sign);
        for p in self.entities.iter_players() {
            if p.sent_chunks.contains(&cpos) {
                self.send(p.conn_id, pkt.clone());
            }
        }
    }

    fn spawn_dropped_item_random(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        item: ItemStack,
        pickup_delay: i32,
    ) {
        if item.count == 0 || item.id <= 0 {
            return;
        }
        let mut rng = std::mem::replace(
            &mut self.world.world_random,
            crate::util::rng::JavaRandom::new(0),
        );
        let f = 0.7f32;
        let d0 = (rng.next_f32() * f) as f64 + ((1.0 - f) as f64) * 0.5;
        let d1 = (rng.next_f32() * f) as f64 + ((1.0 - f) as f64) * 0.5;
        let d2 = (rng.next_f32() * f) as f64 + ((1.0 - f) as f64) * 0.5;
        let mot_x = (rng.next_f32() * 0.2 - 0.1) as f64;
        let mot_y = 0.2f64;
        let mot_z = (rng.next_f32() * 0.2 - 0.1) as f64;
        self.world.world_random = rng;

        let item_id = self.entities.spawn_dropped_item(
            item,
            x + d0,
            y + d1,
            z + d2,
            mot_x,
            mot_y,
            mot_z,
            pickup_delay,
        );
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(dropped) = self.entities.get_dropped_item(item_id) {
            let packets = self.tracker.track_item_and_scan(dropped, &players);
            for outbound in packets {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }
    }

    fn spawn_thrown_item(&mut self, x: f64, y: f64, z: f64, yaw: f32, pitch: f32, item: ItemStack) {
        if item.count == 0 || item.id <= 0 {
            return;
        }
        let spawn_y = y - 0.3 + 1.62;
        let yaw_rad = (yaw / 180.0) * std::f32::consts::PI;
        let pitch_rad = (pitch / 180.0) * std::f32::consts::PI;
        let f = 0.3f32;
        let mot_x = (-yaw_rad.sin() * pitch_rad.cos() * f) as f64;
        let mot_z = (yaw_rad.cos() * pitch_rad.cos() * f) as f64;
        let mot_y = (-pitch_rad.sin() * f + 0.1) as f64;

        let item_id = self
            .entities
            .spawn_dropped_item(item, x, spawn_y, z, mot_x, mot_y, mot_z, 40);
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(dropped) = self.entities.get_dropped_item(item_id) {
            let packets = self.tracker.track_item_and_scan(dropped, &players);
            for outbound in packets {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }
    }

    fn tick_dropped_items(&mut self) {
        let item_ids: Vec<i32> = self
            .entities
            .iter_dropped_items()
            .map(|i| i.entity_id)
            .collect();
        let mut to_remove = Vec::new();

        for item_id in item_ids {
            let (ix, iy, iz, on_ground, dead) = {
                let Some(item) = self.entities.get_dropped_item_mut(item_id) else {
                    continue;
                };
                if item.pickup_delay > 0 {
                    item.pickup_delay -= 1;
                }
                item.mot_y -= 0.03999999910593033;

                let bx = item.x.floor() as i32;
                let by = item.y.floor() as i32;
                let bz = item.z.floor() as i32;

                if blocks::is_lava(self.world.get_block(bx, by, bz)) {
                    item.mot_y = 0.2;
                    item.health -= 4;
                }

                let next_x = item.x + item.mot_x;
                let next_y = item.y + item.mot_y;
                let next_z = item.z + item.mot_z;

                if !blocks::is_solid(self.world.get_block(
                    next_x.floor() as i32,
                    item.y.floor() as i32,
                    item.z.floor() as i32,
                )) {
                    item.x = next_x;
                } else {
                    item.mot_x = 0.0;
                }

                if !blocks::is_solid(self.world.get_block(
                    item.x.floor() as i32,
                    item.y.floor() as i32,
                    next_z.floor() as i32,
                )) {
                    item.z = next_z;
                } else {
                    item.mot_z = 0.0;
                }

                let below_y = next_y.floor() as i32;
                let below_id =
                    self.world
                        .get_block(item.x.floor() as i32, below_y, item.z.floor() as i32);
                if item.mot_y < 0.0 && blocks::is_solid(below_id) {
                    item.y = (below_y + 1) as f64;
                    item.mot_y = 0.0;
                    item.on_ground = true;
                } else {
                    item.y = next_y;
                    item.on_ground = false;
                }

                let mut friction = 0.98f64;
                if item.on_ground {
                    let ground_id = self.world.get_block(
                        item.x.floor() as i32,
                        item.y.floor() as i32 - 1,
                        item.z.floor() as i32,
                    );
                    friction = (blocks::friction_factor(ground_id) * 0.98) as f64;
                }
                item.mot_x *= friction;
                item.mot_y *= 0.9800000190734863;
                item.mot_z *= friction;

                item.age += 1;
                if item.age >= 6000 || item.health <= 0 || item.y < -64.0 {
                    item.dead = true;
                }

                (item.x, item.y, item.z, item.on_ground, item.dead)
            };
            let _ = on_ground;

            if dead {
                to_remove.push(item_id);
                continue;
            }

            // Check if any living player is in pickup range (`EntityHuman.v()` + `EntityItem.b()`)
            let can_pickup = self
                .entities
                .get_dropped_item(item_id)
                .is_some_and(|i| i.pickup_delay == 0 && !i.dead);
            if !can_pickup {
                continue;
            }

            let collector = self.entities.iter_players().find_map(|p| {
                if p.dead {
                    return None;
                }
                if (ix - p.x).abs() <= 1.3
                    && iy >= p.y - 0.5
                    && iy <= p.y + 2.0
                    && (iz - p.z).abs() <= 1.3
                {
                    Some((p.entity_id, p.conn_id))
                } else {
                    None
                }
            });

            if let Some((player_entity_id, player_conn_id)) = collector {
                let mut stack = match self.entities.get_dropped_item(item_id) {
                    Some(i) => i.item,
                    None => continue,
                };
                let mut picked_up = false;
                let mut sync_items = None;
                if let Some(player) = self.entities.get_player_mut(player_entity_id) {
                    if player.inventory.pickup(&mut stack) {
                        picked_up = true;
                        sync_items = Some(player.inventory.to_window_items());
                    }
                }
                if picked_up {
                    if let Some(item_mut) = self.entities.get_dropped_item_mut(item_id) {
                        item_mut.item = stack;
                        if stack.count == 0 {
                            item_mut.dead = true;
                            to_remove.push(item_id);
                        }
                    }
                    self.broadcast(Clientbound::Collect(Collect {
                        collected_id: item_id,
                        collector_id: player_entity_id,
                    }));
                    if let Some(items) = sync_items {
                        self.send(
                            player_conn_id,
                            Clientbound::WindowItems(WindowItems {
                                window_id: 0,
                                items,
                            }),
                        );
                    }
                }
            }
        }

        for item_id in to_remove {
            let _ = self.entities.remove_dropped_item(item_id);
            let untrack = self.tracker.untrack_item(item_id);
            for outbound in untrack {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }
    }

    fn flush_dirty_chunks(&mut self) {
        while self.world.java_do_lighting() {}
        let dirty_list = self.world.take_dirty_chunks();
        if dirty_list.is_empty() {
            return;
        }

        for (pos, dirty, watchers) in dirty_list {
            if dirty.count == 1 {
                let raw = dirty.blocks[0];
                let lx = ((raw >> 12) & 15) as i32;
                let lz = ((raw >> 8) & 15) as i32;
                let y = (raw & 255) as i32;
                let wx = pos.x * 16 + lx;
                let wz = pos.z * 16 + lz;
                let pkt = Clientbound::BlockChange(BlockChange {
                    x: wx,
                    y: y as u8,
                    z: wz,
                    block_type: self.world.get_block(wx, y, wz),
                    metadata: self.world.get_data(wx, y, wz),
                });
                for conn_id in watchers {
                    if self
                        .entities
                        .get_by_conn(conn_id)
                        .is_some_and(|p| p.sent_chunks.contains(&pos))
                    {
                        self.send(conn_id, pkt.clone());
                    }
                }
            } else if dirty.count < 10 {
                let mut coordinates = Vec::with_capacity(dirty.count);
                let mut block_types = Vec::with_capacity(dirty.count);
                let mut metadata = Vec::with_capacity(dirty.count);
                for &raw in &dirty.blocks[..dirty.count] {
                    let lx = ((raw >> 12) & 15) as i32;
                    let lz = ((raw >> 8) & 15) as i32;
                    let y = (raw & 255) as i32;
                    let wx = pos.x * 16 + lx;
                    let wz = pos.z * 16 + lz;
                    coordinates.push(raw as i16);
                    block_types.push(self.world.get_block(wx, y, wz));
                    metadata.push(self.world.get_data(wx, y, wz));
                }
                let pkt = Clientbound::MultiBlockChange(MultiBlockChange {
                    chunk_x: pos.x,
                    chunk_z: pos.z,
                    coordinates,
                    block_types,
                    metadata,
                });
                for conn_id in watchers {
                    if self
                        .entities
                        .get_by_conn(conn_id)
                        .is_some_and(|p| p.sent_chunks.contains(&pos))
                    {
                        self.send(conn_id, pkt.clone());
                    }
                }
            } else if let Ok((_pre, map)) = self.world.prepare_chunk_packets(pos) {
                let pkt = Clientbound::MapChunk(map);
                for conn_id in watchers {
                    if self
                        .entities
                        .get_by_conn(conn_id)
                        .is_some_and(|p| p.sent_chunks.contains(&pos))
                    {
                        self.send(conn_id, pkt.clone());
                    }
                }
            }
        }
    }

    fn handle_respawn(&mut self, id: ConnId) {
        if self.world.live_is_active() {
            self.world.live_player_quit(id);
        }
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();

        // Poseidon double-respawn client hack: send opposite dimension first, then dimension 0
        self.send(id, Clientbound::Respawn(Respawn { dimension: -1 }));
        self.send(id, Clientbound::Respawn(Respawn { dimension: 0 }));

        if let Some(player) = self.entities.get_by_conn_mut(id) {
            let old_chunk = player.chunk_pos();
            player.respawn(spawn_x as f64 + 0.5, spawn_y as f64, spawn_z as f64 + 0.5);
            let new_chunk = player.chunk_pos();
            let entity_id = player.entity_id;
            self.entities
                .update_player_chunk(entity_id, old_chunk, new_chunk);
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

        self.send_queued_pre_chunks(id);
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

        if self.config.world.save_enabled {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let _ = self.player_store.load_player(player);
                player.last_eval_x = player.x;
                player.last_eval_z = player.z;
            }
        }

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
        let world_time = self.world.world_time();
        self.send(id, Clientbound::UpdateTime(UpdateTime { time: world_time }));

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
        let health = self
            .entities
            .get_player(entity_id)
            .map(|p| p.health)
            .unwrap_or(20);
        self.send(id, Clientbound::UpdateHealth(UpdateHealth { health }));

        // Allocate remaining view-square chunks on the client with PreChunk(true) (PlayerInstance.a)
        self.send_queued_pre_chunks(id);

        // Poseidon: PlayerJoinEvent -> broadcast join message to other players
        let join_msg = self
            .config
            .server
            .join_message
            .replacen("%player%", &login.username, 1);
        self.broadcast_chat_except(id, &join_msg);
    }

    fn stream_initial_chunks(&mut self, id: ConnId) {
        let (player_x, player_y, player_z, yaw, pitch) = {
            let p = self.entities.get_by_conn(id).unwrap();
            (p.x, p.y, p.z, p.yaw, p.pitch)
        };
        if self.world.live_is_active() {
            self.world.live_player_join(id, player_x, player_z);
            let _ = self.world.take_player_chunk_events(id);
            while self.world.java_do_lighting() {}
        }
        let center = ChunkPos::new((player_x as i32) >> 4, (player_z as i32) >> 4);
        let view_distance = self.config.world.view_distance;

        // Spiral ordering of chunks around player
        let mut spiral = Vec::new();
        spiral.push(center);
        let directions = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let mut dir_idx = 0;
        let mut cx = 0;
        let mut cz = 0;
        for k1 in 1..=view_distance * 2 {
            for _ in 0..2 {
                let (dx, dz) = directions[dir_idx % 4];
                dir_idx += 1;
                for _ in 0..k1 {
                    cx += dx;
                    cz += dz;
                    let p = ChunkPos::new(center.x + cx, center.z + cz);
                    if (p.x - center.x).abs() <= view_distance
                        && (p.z - center.z).abs() <= view_distance
                    {
                        spiral.push(p);
                    }
                }
            }
        }
        dir_idx %= 4;
        for _ in 0..view_distance * 2 {
            cx += directions[dir_idx].0;
            cz += directions[dir_idx].1;
            let p = ChunkPos::new(center.x + cx, center.z + cz);
            if (p.x - center.x).abs() <= view_distance && (p.z - center.z).abs() <= view_distance {
                spiral.push(p);
            }
        }

        // Send 3x3 immediate area synchronously so player spawns on solid terrain
        let immediate_radius = 1;
        for dz in -immediate_radius..=immediate_radius {
            for dx in -immediate_radius..=immediate_radius {
                let pos = ChunkPos::new(center.x + dx, center.z + dz);
                if self.world.live_is_active() {
                    self.world.server_get_chunk_at(pos);
                    while self.world.java_do_lighting() {}
                } else {
                    self.world.populate_chunk(pos);
                }
                if let Ok((pre, map)) = self.world.prepare_chunk_packets(pos) {
                    self.send(id, Clientbound::PreChunk(pre));
                    self.send(id, Clientbound::MapChunk(map));
                    if let Some(player) = self.entities.get_by_conn_mut(id) {
                        player.pre_chunk_sent.insert(pos);
                        player.sent_chunks.insert(pos);
                    }
                }
            }
        }

        self.send(
            id,
            Clientbound::PlayerLookMove(PlayerLookMove {
                x: player_x,
                y: player_y + 1.62,
                z: player_z,
                stance: player_y,
                yaw,
                pitch,
                on_ground: false,
            }),
        );

        // Queue remaining chunks in spiral order for tick streaming
        if let Some(player) = self.entities.get_by_conn_mut(id) {
            for pos in spiral {
                if !player.sent_chunks.contains(&pos) {
                    player.chunk_queue.push(pos);
                }
            }
        }
    }

    fn send_queued_pre_chunks(&mut self, id: ConnId) {
        if !self.world.live_is_active() {
            return;
        }
        let to_pre_allocate = if let Some(player) = self.entities.get_by_conn_mut(id) {
            let mut list = Vec::with_capacity(player.chunk_queue.len());
            for &pos in &player.chunk_queue {
                if player.pre_chunk_sent.insert(pos) {
                    list.push(pos);
                }
            }
            list
        } else {
            return;
        };

        for pos in to_pre_allocate {
            self.send(
                id,
                Clientbound::PreChunk(PreChunk {
                    x: pos.x,
                    z: pos.z,
                    mode: true,
                }),
            );
        }
    }

    fn stream_chunks(&mut self) {
        let max_per_tick = if self.world.live_is_active() { 5 } else { 2 };
        let chunks_per_tick = self.config.world.chunks_per_tick.clamp(1, max_per_tick);
        while self.world.java_do_lighting() {}

        let player_ids: Vec<ConnId> = self.entities.iter_players().map(|p| p.conn_id).collect();
        let mut populated_this_tick = 0;

        for id in player_ids {
            let chunks_to_send = if let Some(player) = self.entities.get_by_conn_mut(id) {
                let count = player.chunk_queue.len().min(chunks_per_tick);
                player.chunk_queue.drain(0..count).collect::<Vec<_>>()
            } else {
                Vec::new()
            };

            for (i, pos) in chunks_to_send.iter().enumerate() {
                if self.world.live_is_active() {
                    if !self.world.is_chunk_loaded(*pos) {
                        continue;
                    }
                } else {
                    let is_pop = self
                        .world
                        .get_chunk(*pos)
                        .is_some_and(|c| c.state == ChunkState::Populated);
                    if !is_pop {
                        if populated_this_tick >= 1 {
                            if let Some(player) = self.entities.get_by_conn_mut(id) {
                                for remaining in chunks_to_send[i..].iter().rev() {
                                    player.chunk_queue.insert(0, *remaining);
                                }
                            }
                            break;
                        }
                        self.world.populate_chunk(*pos);
                        populated_this_tick += 1;
                    }
                }

                if let Ok((pre, map)) = self.world.prepare_chunk_packets(*pos) {
                    let need_pre = self
                        .entities
                        .get_by_conn_mut(id)
                        .is_some_and(|p| p.pre_chunk_sent.insert(*pos));
                    if need_pre {
                        self.send(id, Clientbound::PreChunk(pre));
                    }
                    self.send(id, Clientbound::MapChunk(map));
                    if let Some(player) = self.entities.get_by_conn_mut(id) {
                        player.sent_chunks.insert(*pos);
                    }

                    // Send tile entities in this chunk
                    if let Some(chunk) = self.world.get_chunk(*pos) {
                        for te in &chunk.tile_entities {
                            if let TileEntityKind::Sign { lines } = &te.kind {
                                self.send(
                                    id,
                                    Clientbound::UpdateSign(UpdateSign {
                                        x: te.x,
                                        y: te.y as i16,
                                        z: te.z,
                                        lines: lines.clone(),
                                    }),
                                );
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

        let raw_message = chat.message.trim();

        // Poseidon: length > 100 -> disconnect "Chat message too long"
        if raw_message.len() > 100 {
            self.kick(id, "Chat message too long");
            return;
        }

        // Poseidon: illegal characters -> disconnect "Illegal characters in chat"
        if !raw_message.chars().all(is_allowed_char) {
            self.kick(id, "Illegal characters in chat");
            return;
        }

        let message = crate::net::packets::chat::translate_alternate_color_codes('&', raw_message);

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

        let cmd_body = &command[1..];
        let mut parts = cmd_body.split_whitespace();
        let label = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        // Handle live gameplay effects for `/give`, `/time`, `/kill`, `/save-all`
        match label {
            "give" if args.len() >= 2 => {
                let target_name = args[0];
                if let Ok(item_id) = args[1].parse::<i16>() {
                    let amount = args
                        .get(2)
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(1)
                        .clamp(1, 64);
                    let damage = args.get(3).and_then(|s| s.parse::<i16>().ok()).unwrap_or(0);
                    let target_info = self
                        .entities
                        .iter_players()
                        .find(|p| p.username.eq_ignore_ascii_case(target_name))
                        .map(|p| (p.x, p.y, p.z));
                    if let Some((tx, ty, tz)) = target_info {
                        self.spawn_dropped_item_random(
                            tx,
                            ty,
                            tz,
                            ItemStack::new(item_id, amount, damage),
                            0,
                        );
                    }
                }
            }
            "time" if !args.is_empty() => {
                let new_time = if args[0] == "set" && args.len() >= 2 {
                    match args[1] {
                        "day" => Some(0i64),
                        "night" => Some(12500i64),
                        other => other.parse::<i64>().ok(),
                    }
                } else {
                    match args[0] {
                        "day" => Some(0i64),
                        "night" => Some(12500i64),
                        other => other.parse::<i64>().ok(),
                    }
                };
                if let Some(t) = new_time {
                    self.world.live_time = t;
                    self.world.level_data.borrow_mut().time = t;
                    self.broadcast(Clientbound::UpdateTime(UpdateTime { time: t }));
                }
            }
            "kill" => {
                if let Some(player) = self.entities.get_by_conn(id) {
                    let eid = player.entity_id;
                    self.damage_player_direct(eid, 1000, "died");
                }
            }
            "save-all" => {
                for player in self.entities.iter_players() {
                    let _ = self.player_store.save_player(player);
                }
                let _ = self.world.save_all();
            }
            _ => {}
        }

        let result = self.commands.dispatch(&username, cmd_body);

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

    fn damage_player_direct(&mut self, entity_id: i32, damage: i16, reason: &str) {
        let (conn_id, username, health, died, drops) = {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let (dealt, died) = player.take_damage_direct(damage);
                if dealt > 0 {
                    let drops = if died {
                        let px = player.x;
                        let py = player.y;
                        let pz = player.z;
                        player
                            .inventory
                            .drop_all_on_death()
                            .into_iter()
                            .map(|it| (px, py, pz, it))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (
                        player.conn_id,
                        player.username.clone(),
                        player.health,
                        died,
                        drops,
                    )
                } else {
                    return;
                }
            } else {
                return;
            }
        };

        self.broadcast(Clientbound::EntityStatus(EntityStatus {
            entity_id,
            status: 2,
        }));

        self.send(conn_id, Clientbound::UpdateHealth(UpdateHealth { health }));

        if died {
            for (dx, dy, dz, stack) in drops {
                self.spawn_dropped_item_random(dx, dy, dz, stack, 40);
            }
            let death_msg = format!("\u{00A7}e{username} {reason}");
            self.broadcast_chat(&death_msg);
            self.broadcast(Clientbound::EntityStatus(EntityStatus {
                entity_id,
                status: 3,
            }));
        }
    }

    #[allow(dead_code)]
    fn damage_player(&mut self, entity_id: i32, damage: i16, reason: &str) {
        let (conn_id, username, health, died, drops) = {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let (dealt, died) = player.take_damage(damage);
                if dealt > 0 {
                    let drops = if died {
                        let px = player.x;
                        let py = player.y;
                        let pz = player.z;
                        player
                            .inventory
                            .drop_all_on_death()
                            .into_iter()
                            .map(|it| (px, py, pz, it))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (
                        player.conn_id,
                        player.username.clone(),
                        player.health,
                        died,
                        drops,
                    )
                } else {
                    return;
                }
            } else {
                return;
            }
        };

        self.broadcast(Clientbound::EntityStatus(EntityStatus {
            entity_id,
            status: 2,
        }));

        self.send(conn_id, Clientbound::UpdateHealth(UpdateHealth { health }));

        if died {
            for (dx, dy, dz, stack) in drops {
                self.spawn_dropped_item_random(dx, dy, dz, stack, 40);
            }
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

    #[allow(dead_code)]
    fn try_send(&self, id: ConnId, packet: Clientbound) -> bool {
        if let Some(connection) = self.connections.get(&id) {
            connection.sender.try_send(Outbound::Packet(packet)).is_ok()
        } else {
            false
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
