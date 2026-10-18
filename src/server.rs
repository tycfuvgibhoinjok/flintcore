use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::config::Config;
use crate::entity::mob::{
    chunk_light_at, evaluate_natural_spawns, find_path, world_sky_subtracted, MobType,
};
use crate::entity::player::DiggingState;
use crate::entity::projectile::{ProjectileKind, PAINTING_TITLES};
use crate::entity::store::EntityStore;
use crate::entity::tracker::EntityTracker;
use crate::gameplay::blocks;
use crate::gameplay::commands::{CommandAction, CommandContext, CommandMap};
use crate::gameplay::inventory::{InventoryPlayer, OpenContainer};
use crate::gameplay::items::{self, ItemStack};
use crate::gameplay::tile::{TileEntity, TileEntityKind};
use crate::net::connection::ConnectionState;
use crate::net::packets::chat::{is_allowed_char, wrap_text};
use crate::net::packets::{
    ArmAnimation, AttachEntity, Bed, BlockChange, Chat, Clientbound, Collect, CraftProgressBar,
    EntityMetadata, EntityStatus, EntityVelocity, Explosion, Handshake, KeepAlive, KickDisconnect,
    LevelSound, Login, MultiBlockChange, OpenWindow, PlayNoteBlock, PlayerLookMove, PreChunk,
    Respawn, Serverbound, SetSlot, Sleep, SpawnPosition, Transaction, UpdateHealth, UpdateSign,
    UpdateTime, WindowItems,
};
use crate::net::protocol::PROTOCOL_VERSION;
use crate::net::{ConnId, NetEvent, Outbound};
use crate::plugin::events::{PlayerChatEvent, PluginManager};
use crate::util::rng::JavaRandom;
use crate::util::{log, Result};
use crate::world::chunk::{ChunkPos, ChunkState};
use crate::world::redstone::WorldEvent;
use crate::world::world::World;

const TICK: Duration = Duration::from_millis(50);
const MAX_CATCHUP: u32 = 4;
const SLEEP_SLICE: Duration = Duration::from_millis(5);

struct Connection {
    sender: mpsc::UnboundedSender<Outbound>,
    address: std::net::SocketAddr,
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
    ops: HashSet<String>,
    banned_players: HashSet<String>,
    banned_ips: HashSet<String>,
    whitelist: HashSet<String>,
    whitelist_enabled: bool,
    tps_records: VecDeque<f64>,
    last_tps_sample: Instant,
    ticks_since_sample: u32,
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
            // Queue and immediately flush startup-only chunks outside the 128-block keep-spawn radius
            world.queue_unwatched_chunks();
            while !world.live.unload_queue.is_empty() {
                world.live_unload_chunks();
            }
            world.compact_all_chunks();
        }
        let player_store = crate::world::storage::PlayerStore::new(world.world_dir());
        let view_distance = config.world.view_distance;
        let ops = Self::load_list_file("ops.txt");
        let banned_players = Self::load_list_file("banned-players.txt");
        let banned_ips = Self::load_list_file("banned-ips.txt");
        let whitelist = Self::load_list_file("white-list.txt");

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
            ops,
            banned_players,
            banned_ips,
            whitelist,
            whitelist_enabled: false,
            tps_records: VecDeque::with_capacity(900),
            last_tps_sample: Instant::now(),
            ticks_since_sample: 0,
        }
    }

    fn load_list_file(filename: &str) -> HashSet<String> {
        std::fs::read_to_string(filename)
            .ok()
            .map(|text| {
                text.lines()
                    .map(|l| l.trim().to_lowercase())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn save_list_file(filename: &str, set: &HashSet<String>) {
        let mut lines: Vec<&str> = set.iter().map(|s| s.as_str()).collect();
        lines.sort_unstable();
        let content = lines.join("\n");
        let _ = std::fs::write(filename, content);
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
        self.ticks_since_sample += 1;
        let elapsed_sample = self.last_tps_sample.elapsed().as_secs_f64();
        if elapsed_sample >= 1.0 {
            let tps = ((self.ticks_since_sample as f64) / elapsed_sample).min(20.0);
            if self.tps_records.len() >= 900 {
                self.tps_records.pop_back();
            }
            self.tps_records.push_front(tps);
            self.ticks_since_sample = 0;
            self.last_tps_sample = Instant::now();
        }

        if self.world.live_is_active() {
            self.world.live_tick();
        } else {
            self.world.increment_time();
        }
        self.drain_inbound();

        // Incremental auto-save (`ChunkProviderServer.saveChunks(false, null)` — up to 24 dirty chunks)
        if ticks > 0 && self.config.world.save_enabled {
            if ticks % 100 == 0 {
                let _ = self.world.save_chunks_incremental(24);
            }
            if ticks % 1200 == 0 {
                for player in self.entities.iter_players() {
                    let _ = self.player_store.save_player(player);
                }
            }
        }

        // Tick furnaces and sync open furnace windows
        self.world.tick_furnaces();
        self.sync_open_containers();

        // Random block ticks (grass, crops, saplings, reeds, leaves, ice/snow) when players online
        let player_xz: Vec<(f64, f64)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead)
            .map(|p| (p.x, p.z))
            .collect();
        if !player_xz.is_empty() {
            self.world.tick_random_blocks(&player_xz);
        }

        // Tick each player (no_damage_ticks, fire_ticks decrement, deferred digging)
        for player in self.entities.iter_players_mut() {
            player.tick();
        }
        self.tick_player_environmental_damage();
        self.tick_deferred_digging();
        self.tick_pressure_plates_and_beds();

        // Natural mob spawning every 20 ticks (`SpawnerCreature.spawnEntities`)
        if ticks % 20 == 0 && self.world.live_is_active() {
            self.tick_mob_spawner();
        }
        self.tick_mob_spawners();

        // Tick boats, mobs, projectiles, primed TNTs, minecarts, and dropped items
        self.tick_boats();
        self.tick_mobs();
        self.tick_projectiles();
        self.tick_primed_tnts();
        self.tick_minecarts();
        self.tick_dropped_items();
        self.process_world_events();
        self.world.flush_pending_piston_syncs();

        while self.world.java_do_lighting() {}
        if ticks % 200 == 0 {
            self.world.compact_dirty_light_chunks();
        }

        self.flush_dirty_chunks();
        self.stream_chunks();

        // Tick entity tracking every 2 ticks (mirroring Poseidon EntityTracker)
        if ticks % 2 == 0 {
            self.run_tracker_tick();
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

    fn run_tracker_tick(&mut self) {
        let mut players: Vec<_> = self.entities.iter_players().cloned().collect();
        let mut items: Vec<_> = self.entities.iter_dropped_items().cloned().collect();
        let mut boats: Vec<_> = self.entities.iter_boats().cloned().collect();
        let mut mobs: Vec<_> = self.entities.iter_mobs().cloned().collect();
        let mut projectiles: Vec<_> = self.entities.iter_projectiles().cloned().collect();
        let mut tnts: Vec<_> = self.entities.iter_primed_tnts().cloned().collect();
        let mut minecarts: Vec<_> = self.entities.iter_minecarts().cloned().collect();
        let paintings: Vec<_> = self.entities.iter_paintings().cloned().collect();

        let tracking_packets = self.tracker.tick_world(
            &mut players,
            &mut items,
            &mut boats,
            &mut mobs,
            &mut projectiles,
            &mut tnts,
            &mut minecarts,
            &paintings,
        );
        for p in players {
            if let Some(live) = self.entities.get_player_mut(p.entity_id) {
                live.mot_x = p.mot_x;
                live.mot_y = p.mot_y;
                live.mot_z = p.mot_z;
                live.velocity_changed = p.velocity_changed;
                live.equipment_cache = p.equipment_cache;
            }
        }
        for m in mobs {
            if let Some(live_mob) = self.entities.get_mob_mut(m.entity_id) {
                live_mob.velocity_changed = m.velocity_changed;
            }
        }

        for outbound in tracking_packets {
            if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                self.send(target.conn_id, outbound.packet);
            }
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
                        address,
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
            if let Some(p) = self.entities.get_by_conn(id) {
                let eid = p.entity_id;
                self.dismount_player(eid);
            }
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
                let mut meta_update = None;
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    match action.action {
                        1 => {
                            player.sneaking = true;
                            meta_update = Some((player.entity_id, player.metadata_flags()));
                        }
                        2 => {
                            player.sneaking = false;
                            meta_update = Some((player.entity_id, player.metadata_flags()));
                        }
                        3 => {
                            player.sleeping = false;
                            player.bed_pos = None;
                        }
                        _ => {}
                    }
                }
                if let Some((entity_id, flags)) = meta_update {
                    let pkt = Clientbound::EntityMetadata(EntityMetadata {
                        entity_id,
                        metadata: vec![0, flags, 127],
                    });
                    for viewer_id in self.tracker.observers_of_player(entity_id) {
                        if let Some(target) = self.entities.get_player(viewer_id) {
                            self.send(target.conn_id, pkt.clone());
                        }
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
        // Check if player is riding a vehicle (e.g. EntityBoat) — NetServerHandler.java:272-327
        let riding_vehicle =
            self.entities
                .get_by_conn(id)
                .and_then(|p| if p.dead { None } else { p.vehicle_id });
        if let Some(boat_id) = riding_vehicle {
            if let Some((px, py, pstance, pz)) = pos {
                if px.is_nan() || py.is_nan() || pz.is_nan() || pstance.is_nan() {
                    self.kick(id, "Nope!");
                    return;
                }
                if py == -999.0 && pstance == -999.0 {
                    // Boat crash fix ported from UberBukkit / Poseidon (NetServerHandler.java:295)
                    let d8 = px * px + pz * pz;
                    if d8 > 100.0 {
                        self.kick(id, "Boat crash attempt detected!");
                        return;
                    }
                    if let Some(player) = self.entities.get_by_conn_mut(id) {
                        player.mot_x = px;
                        player.mot_z = pz;
                        if let Some((yaw, pitch)) = rot {
                            player.yaw = yaw;
                            player.pitch = pitch;
                        }
                        player.on_ground = on_ground;
                    }
                    self.update_player_view_chunks(id);
                    return;
                }
            } else if let Some((yaw, pitch)) = rot {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.mot_x = 0.0;
                    player.mot_z = 0.0;
                    player.yaw = yaw;
                    player.pitch = pitch;
                    player.on_ground = on_ground;
                }
                self.update_player_view_chunks(id);
                return;
            } else {
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.mot_x = 0.0;
                    player.mot_z = 0.0;
                    player.on_ground = on_ground;
                }
                self.update_player_view_chunks(id);
                return;
            }
            let _ = boat_id;
        }

        let mut resync_teleport = None;
        let (fall_damage, entity_id, old_chunk, new_chunk) = {
            if let Some(player) = self.entities.get_by_conn_mut(id) {
                if player.dead {
                    return;
                }
                let old_chunk = player.chunk_pos();
                let prev_y = player.y;
                let mut fall_damage = 0i16;

                let (mut x, mut y, mut stance, mut z) =
                    pos.unwrap_or((player.x, player.y, player.stance, player.z));
                if y == -999.0 && stance == -999.0 {
                    x = player.x;
                    y = player.y;
                    stance = player.stance;
                    z = player.z;
                }
                let (yaw, pitch) = rot.unwrap_or((player.yaw, player.pitch));

                if x.is_nan() || y.is_nan() || z.is_nan() || stance.is_nan() {
                    self.kick(id, "Nope!");
                    return;
                }

                // Poseidon / NetServerHandler.java:385-394 — reject stale pre-teleport / >10-block single-packet jump
                if self.world.live_is_active() && pos.is_some() {
                    let dx_m = x - player.x;
                    let dz_m = z - player.z;
                    let mot_sq = player.mot_x * player.mot_x + player.mot_z * player.mot_z;
                    if (dx_m * dx_m + dz_m * dz_m) - mot_sq > 100.0 {
                        resync_teleport =
                            Some((player.x, player.y, player.z, player.yaw, player.pitch));
                    }
                }

                if resync_teleport.is_none() {
                    if player.vehicle_id.is_none() {
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
                    } else {
                        player.fall_distance = 0.0;
                    }

                    player.update_position(x, y, stance, z, yaw, pitch, on_ground);
                }
                let new_chunk = player.chunk_pos();

                (fall_damage, player.entity_id, old_chunk, new_chunk)
            } else {
                return;
            }
        };

        if let Some((tx, ty, tz, tyaw, tpitch)) = resync_teleport {
            self.send(
                id,
                Clientbound::PlayerLookMove(PlayerLookMove {
                    x: tx,
                    y: ty + 1.62,
                    stance: ty,
                    z: tz,
                    yaw: tyaw,
                    pitch: tpitch,
                    on_ground: false,
                }),
            );
            return;
        }

        self.entities
            .update_player_chunk(entity_id, old_chunk, new_chunk);

        if fall_damage > 0 {
            self.damage_player_direct(entity_id, fall_damage, "hit the ground too hard");
        }

        self.update_player_view_chunks(id);
    }

    fn update_player_view_chunks(&mut self, id: ConnId) {
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
                if let Some(p_mut) = self.entities.get_by_conn_mut(id) {
                    for p in removed {
                        p_mut.sent_chunks.remove(&p);
                        p_mut.chunk_queue.retain(|&q| q != p);
                        if p_mut.pre_chunk_sent.remove(&p) {
                            pre_false.push(p);
                        }
                    }
                    for p in added {
                        if !p_mut.sent_chunks.contains(&p) && !p_mut.chunk_queue.contains(&p) {
                            p_mut.chunk_queue.push(p);
                        }
                    }
                    p_mut.chunk_queue.sort_by_key(|&p| {
                        let unpop = self
                            .world
                            .get_chunk(p)
                            .map(|c| c.state != ChunkState::Populated)
                            .unwrap_or(true);
                        let dist = (p.x - cur_cx).abs().max((p.z - cur_cz).abs());
                        (unpop, dist)
                    });
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
                        if !was_in_old
                            && !p_mut.sent_chunks.contains(&p)
                            && !p_mut.chunk_queue.contains(&p)
                        {
                            p_mut.chunk_queue.push(p);
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

        let target_id = use_entity.target;

        // Check if target is a Boat (`EntityBoat.java`)
        if self.entities.get_boat(target_id).is_some() {
            if !use_entity.attack {
                // Right-click boat: mount or dismount (`EntityBoat.a(EntityHuman)`)
                let (bx, bz, cur_passenger) = match self.entities.get_boat(target_id) {
                    Some(b) if !b.dead => (b.x, b.z, b.passenger_id),
                    _ => return,
                };
                let dx = bx - attacker_x;
                let dz = bz - attacker_z;
                if dx * dx + dz * dz > 36.0 {
                    return;
                }
                if cur_passenger.is_some() && cur_passenger != Some(attacker_id) {
                    return;
                }
                if cur_passenger == Some(attacker_id) {
                    self.dismount_player(attacker_id);
                } else {
                    self.mount_player_on_boat(attacker_id, target_id);
                }
            } else {
                // Left-click boat: deal damage (`EntityBoat.damageEntity`)
                let raw_damage = held_item.map(|s| s.attack_damage()).unwrap_or(1) as i32;
                let mut destroyed = None;
                if let Some(boat) = self.entities.get_boat_mut(target_id) {
                    if !boat.dead {
                        let dx = boat.x - attacker_x;
                        let dz = boat.z - attacker_z;
                        if dx * dx + dz * dz <= 36.0 {
                            boat.rock_direction = -boat.rock_direction;
                            boat.time_since_hit = 10;
                            boat.damage += raw_damage * 10;
                            if boat.damage > 40 {
                                boat.dead = true;
                                destroyed = Some((boat.x, boat.y, boat.z, boat.passenger_id));
                            }
                        }
                    }
                }
                if let Some((bx, by, bz, passenger)) = destroyed {
                    if let Some(pid) = passenger {
                        self.dismount_player(pid);
                    }
                    let _ = self.entities.remove_boat(target_id);
                    let untrack = self.tracker.untrack_boat(target_id);
                    for outbound in untrack {
                        if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                            self.send(target.conn_id, outbound.packet);
                        }
                    }
                    // EntityBoat.java:124-130 — drop 3x WOOD_PLANKS + 2x STICK
                    for _ in 0..3 {
                        self.spawn_dropped_item_random(
                            bx,
                            by,
                            bz,
                            ItemStack::new(blocks::WOOD_PLANKS as i16, 1, 0),
                            10,
                        );
                    }
                    for _ in 0..2 {
                        self.spawn_dropped_item_random(
                            bx,
                            by,
                            bz,
                            ItemStack::new(items::STICK, 1, 0),
                            10,
                        );
                    }
                }
            }
            return;
        }

        // Check if target is a Mob (`EntityLiving` / `EntityAnimal` / `EntityMonster`)
        if self.entities.get_mob(target_id).is_some() {
            if !use_entity.attack {
                self.handle_interact_mob(id, attacker_id, &attacker_name, target_id);
            } else {
                let raw_damage = held_item.map(|s| s.attack_damage()).unwrap_or(1);
                self.damage_mob_from_player(
                    id,
                    attacker_id,
                    attacker_x,
                    attacker_z,
                    target_id,
                    raw_damage,
                );
            }
            return;
        }

        // Check if target is a Minecart (`EntityMinecart`)
        if self.entities.get_minecart(target_id).is_some() {
            if !use_entity.attack {
                self.handle_interact_minecart(id, attacker_id, target_id);
            } else {
                let raw_damage = held_item.map(|s| s.attack_damage()).unwrap_or(1) as i32;
                self.damage_minecart(target_id, raw_damage);
            }
            return;
        }

        // Check if target is a Painting (`EntityPainting`)
        if self.entities.get_painting(target_id).is_some() {
            if use_entity.attack {
                if let Some(p) = self.entities.remove_painting(target_id) {
                    for outbound in self.tracker.untrack_painting(target_id) {
                        if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                            self.send(target.conn_id, outbound.packet);
                        }
                    }
                    self.spawn_dropped_item_random(
                        p.x as f64 + 0.5,
                        p.y as f64 + 0.5,
                        p.z as f64 + 0.5,
                        ItemStack::new(items::PAINTING, 1, 0),
                        10,
                    );
                }
            }
            return;
        }

        // Check if target is a Ghast Fireball (`EntityFireball`: deflect when punched!)
        if let Some(proj) = self.entities.get_projectile_mut(target_id) {
            if use_entity.attack && proj.kind == ProjectileKind::Fireball {
                if let Some(p) = self.entities.get_by_conn(id) {
                    let deg = 0.017_453_292_f32;
                    let yaw_rad = -p.yaw * deg - std::f32::consts::PI;
                    let pitch_rad = -p.pitch * deg;
                    let f3 = yaw_rad.cos();
                    let f4 = yaw_rad.sin();
                    let f5 = -pitch_rad.cos();
                    let f6 = pitch_rad.sin();
                    let look_x = (f4 * f5) as f64;
                    let look_y = f6 as f64;
                    let look_z = (f3 * f5) as f64;
                    if let Some(fb) = self.entities.get_projectile_mut(target_id) {
                        fb.mot_x = look_x;
                        fb.mot_y = look_y;
                        fb.mot_z = look_z;
                        fb.accel_x = look_x * 0.1;
                        fb.accel_y = look_y * 0.1;
                        fb.accel_z = look_z * 0.1;
                        fb.shooter_id = Some(attacker_id);
                    }
                }
            }
            return;
        }

        if use_entity.attack {
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
                self.dismount_player(target_id);
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

    fn mount_player_on_boat(&mut self, player_id: i32, boat_id: i32) {
        // Dismount from any previous vehicle first
        self.dismount_player(player_id);

        let boat_pos = match self.entities.get_boat_mut(boat_id) {
            Some(b) if !b.dead && b.passenger_id.is_none() => {
                b.passenger_id = Some(player_id);
                Some((b.x, b.y + b.mounted_y_offset() + 1.12, b.z))
            }
            _ => None,
        };
        let Some((bx, by, bz)) = boat_pos else {
            return;
        };

        let mut player_conn_and_rot = None;
        if let Some(player) = self.entities.get_player_mut(player_id) {
            player.vehicle_id = Some(boat_id);
            player.x = bx;
            player.y = by;
            player.stance = by;
            player.z = bz;
            player_conn_and_rot = Some((player.conn_id, player.yaw, player.pitch));
        }

        let attach = Clientbound::AttachEntity(AttachEntity {
            entity_id: player_id,
            vehicle_id: boat_id,
        });
        self.broadcast(attach);

        if let Some((conn_id, yaw, pitch)) = player_conn_and_rot {
            self.send(
                conn_id,
                Clientbound::PlayerLookMove(PlayerLookMove {
                    x: bx,
                    y: by + 1.62,
                    stance: by,
                    z: bz,
                    yaw,
                    pitch,
                    on_ground: false,
                }),
            );
        }
    }

    fn dismount_player(&mut self, player_id: i32) {
        let old_vehicle = if let Some(player) = self.entities.get_player_mut(player_id) {
            player.vehicle_id.take()
        } else {
            None
        };
        if let Some(vid) = old_vehicle {
            if let Some(boat) = self.entities.get_boat_mut(vid) {
                if boat.passenger_id == Some(player_id) {
                    boat.passenger_id = None;
                }
            }
            if let Some(mob) = self.entities.get_mob_mut(vid) {
                if mob.passenger_id == Some(player_id) {
                    mob.passenger_id = None;
                }
            }
            if let Some(cart) = self.entities.get_minecart_mut(vid) {
                if cart.passenger_id == Some(player_id) {
                    cart.passenger_id = None;
                }
            }
            self.broadcast(Clientbound::AttachEntity(AttachEntity {
                entity_id: player_id,
                vehicle_id: -1,
            }));
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
        // BlockTNT.postBreak (`BlockTNT.java:41-52`): ignite when metadata bit 0 is set, else drop TNT block
        if old_block == blocks::TNT {
            if (old_meta & 1) != 0 || held_item.is_some_and(|s| s.id == items::FLINT_AND_STEEL) {
                self.world.ignite_tnt(x, y, z, false);
                self.process_world_events();
            } else {
                self.spawn_dropped_item_random(
                    x as f64,
                    y as f64,
                    z as f64,
                    ItemStack::new(blocks::TNT as i16, 1, 0),
                    10,
                );
            }
        } else if items::can_harvest_block(held_item, old_block) {
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
            blocks::TNT => {
                let holding_flint = self
                    .entities
                    .get_by_conn(id)
                    .and_then(|p| p.inventory.item_in_hand())
                    .is_some_and(|s| s.id == items::FLINT_AND_STEEL);
                if holding_flint {
                    self.world.java_set_raw_data(x, y, z, 1);
                }
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
            blocks::LEVER => {
                let data = self.world.get_data(x, y, z);
                let dir = data & 7;
                let toggled = dir | (8 - (data & 8));
                self.world.java_set_data(x, y, z, toggled);
                self.world
                    .notify_attachment_neighbors(x, y, z, blocks::LEVER, dir);
                self.process_world_events();
                self.send_block_change_to_observers(x, y, z);
                true
            }
            blocks::STONE_BUTTON => {
                let data = self.world.get_data(x, y, z);
                if (data & 8) == 0 {
                    let dir = data & 7;
                    self.world.java_set_data(x, y, z, dir | 8);
                    self.world
                        .notify_attachment_neighbors(x, y, z, blocks::STONE_BUTTON, dir);
                    self.world
                        .java_schedule_tick(x, y, z, blocks::STONE_BUTTON, 20);
                    self.process_world_events();
                    self.send_block_change_to_observers(x, y, z);
                }
                true
            }
            blocks::REPEATER_OFF | blocks::REPEATER_ON => {
                let data = self.world.get_data(x, y, z);
                let delay = (((data & 12) >> 2) + 1) & 3;
                let new_data = (data & 3) | (delay << 2);
                self.world.java_set_data(x, y, z, new_data);
                self.world.do_physics_repeater(x, y, z, block_id);
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
                let mut sleep_pkt = None;
                let mut only_at_night = false;
                let time_of_day = self.world.world_time().rem_euclid(24000);
                let is_night = (12541..=23458).contains(&time_of_day);
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    player.spawn_bed = Some((x, y, z));
                    if is_night {
                        player.sleeping = true;
                        player.bed_pos = Some((x, y, z));
                        sleep_pkt = Some(Sleep {
                            entity_id: player.entity_id,
                            in_bed_code: 0,
                            bed_x: x,
                            bed_y: y as u8,
                            bed_z: z,
                        });
                    } else {
                        only_at_night = true;
                    }
                }
                if let Some(pkt) = sleep_pkt {
                    self.broadcast(Clientbound::Sleep(pkt));
                } else if only_at_night {
                    self.send(id, Clientbound::Bed(Bed { reason: 1 }));
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

            self.use_item_right_click_air(id, place.item_id);
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
            // Flint and steel (259 -> Fire 51 or ignite TNT 46)
            items::FLINT_AND_STEEL => {
                if clicked_block == blocks::TNT {
                    self.world.ignite_tnt(cx, cy, cz, false);
                    self.send_block_change_to_observers(cx, cy, cz);
                    self.send_block_change_to_observers(tx, ty, tz);
                    self.process_world_events();
                } else if self.world.get_block(tx, ty, tz) == blocks::AIR {
                    self.world.java_set_type_id(tx, ty, tz, blocks::FIRE);
                    self.send_block_change_to_observers(tx, ty, tz);
                }
                durability_cost = 1;
            }
            // Minecarts (328, 342, 343) on rails (66, 27, 28)
            items::MINECART | items::STORAGE_MINECART | items::POWERED_MINECART => {
                if matches!(
                    clicked_block,
                    blocks::RAIL | blocks::POWERED_RAIL | blocks::DETECTOR_RAIL
                ) {
                    let ctype = match stack.id {
                        items::STORAGE_MINECART => 1,
                        items::POWERED_MINECART => 2,
                        _ => 0,
                    };
                    self.spawn_minecart_at(
                        ctype,
                        cx as f64 + 0.5,
                        cy as f64 + 0.5,
                        cz as f64 + 0.5,
                    );
                    consumed = true;
                }
            }
            // Painting (321 -> EntityPainting on wall faces 2..=5)
            items::PAINTING => {
                if (2..=5).contains(&face) && blocks::material(clicked_block).is_buildable() {
                    let dir = match face {
                        4 => 1,
                        2 => 2,
                        5 => 3,
                        _ => 0,
                    };
                    let title_idx = (self.current_tick as usize + (cx.unsigned_abs() as usize))
                        % PAINTING_TITLES.len();
                    let title = PAINTING_TITLES[title_idx].to_string();
                    self.spawn_painting_at(title, cx, cy, cz, dir);
                    consumed = true;
                }
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
            // Boat item (333 -> EntityBoat, ItemBoat.java:16-63)
            items::BOAT => {
                let mut by = cy;
                if clicked_block == blocks::SNOW {
                    by -= 1;
                }
                self.spawn_boat_at(cx as f64 + 0.5, by as f64 + 1.0, cz as f64 + 0.5);
                consumed = true;
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
                    let is_piston = block_id == blocks::PISTON || block_id == blocks::STICKY_PISTON;
                    let meta = if is_piston {
                        World::compute_piston_orientation(
                            tx, ty, tz, player_x, player_y, player_z, yaw,
                        )
                    } else {
                        items::filter_block_data(stack.id, stack.damage)
                    };
                    self.world
                        .java_set_type_id_and_data(tx, ty, tz, block_id, meta);
                    if is_piston {
                        self.world.update_piston_state(tx, ty, tz, block_id);
                    } else {
                        self.world
                            .post_place_block(block_id, tx, ty, tz, face, yaw, pitch);
                    }
                    self.process_world_events();
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
        self.process_world_events();
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

    fn use_item_right_click_air(&mut self, id: ConnId, packet_item_id: i16) {
        let (px, py, pz, yaw, pitch, held) = match self.entities.get_by_conn(id) {
            Some(p) if !p.dead => (
                p.x,
                p.y,
                p.z,
                p.yaw,
                p.pitch,
                p.inventory.item_in_hand().or_else(|| {
                    if packet_item_id > 0 {
                        Some(ItemStack::new(packet_item_id, 1, 0))
                    } else {
                        None
                    }
                }),
            ),
            _ => return,
        };
        let Some(stack) = held else {
            return;
        };

        let mut new_stack: Option<Option<ItemStack>> = None;

        if stack.is_food() {
            if let Some(player) = self.entities.get_by_conn_mut(id) {
                let heal = stack.food_heal_amount();
                player.heal(heal);
            }
            new_stack = Some(if stack.id == items::MUSHROOM_SOUP {
                Some(ItemStack::new(items::BOWL, 1, 0))
            } else if stack.count > 1 {
                Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
            } else {
                None
            });
        } else if stack.id == items::BOAT {
            // ItemBoat.java:16-63 — raytrace 5.0 blocks with hit_liquids = true
            if let Some((bx, mut by, bz, _face)) = self
                .world
                .ray_trace_from_player(px, py, pz, yaw, pitch, 5.0, true)
            {
                if self.world.get_block(bx, by, bz) == blocks::SNOW {
                    by -= 1;
                }
                self.spawn_boat_at(bx as f64 + 0.5, by as f64 + 1.0, bz as f64 + 0.5);
                new_stack = Some(if stack.count > 1 {
                    Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
                } else {
                    None
                });
            }
        } else if stack.id == items::BUCKET {
            // ItemBucket.java:21-83 — raytrace 5.0 blocks with hit_liquids = true
            if let Some((bx, by, bz, _face)) = self
                .world
                .ray_trace_from_player(px, py, pz, yaw, pitch, 5.0, true)
            {
                let b = self.world.get_block(bx, by, bz);
                let m = self.world.get_data(bx, by, bz);
                if blocks::is_water(b) && m == 0 {
                    self.world.java_set_type_id(bx, by, bz, blocks::AIR);
                    self.send_block_change_to_observers(bx, by, bz);
                    new_stack = Some(Some(ItemStack::new(items::WATER_BUCKET, 1, 0)));
                } else if blocks::is_lava(b) && m == 0 {
                    self.world.java_set_type_id(bx, by, bz, blocks::AIR);
                    self.send_block_change_to_observers(bx, by, bz);
                    new_stack = Some(Some(ItemStack::new(items::LAVA_BUCKET, 1, 0)));
                }
            }
        } else if stack.id == items::WATER_BUCKET || stack.id == items::LAVA_BUCKET {
            // ItemBucket.java:84-150 — raytrace 5.0 blocks with hit_liquids = false
            if let Some((bx, by, bz, face)) = self
                .world
                .ray_trace_from_player(px, py, pz, yaw, pitch, 5.0, false)
            {
                let (tx, ty, tz) = match face {
                    0 => (bx, by - 1, bz),
                    1 => (bx, by + 1, bz),
                    2 => (bx, by, bz - 1),
                    3 => (bx, by, bz + 1),
                    4 => (bx - 1, by, bz),
                    5 => (bx + 1, by, bz),
                    _ => (bx, by, bz),
                };
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
                    new_stack = Some(Some(ItemStack::new(items::BUCKET, 1, 0)));
                }
            }
        } else if stack.id == items::MILK_BUCKET {
            new_stack = Some(Some(ItemStack::new(items::BUCKET, 1, 0)));
        } else if stack.id == items::BOW {
            // ItemBow.java: consume 1 ARROW (262) from inventory and fire EntityArrow
            let can_fire = if let Some(player) = self.entities.get_by_conn_mut(id) {
                if player.inventory.consume_item(items::ARROW) {
                    let win_items = player.inventory.to_window_items();
                    Some((player.entity_id, win_items))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((shooter_id, win_items)) = can_fire {
                self.send(
                    id,
                    Clientbound::WindowItems(WindowItems {
                        window_id: 0,
                        items: win_items,
                    }),
                );
                self.launch_player_projectile(
                    shooter_id,
                    px,
                    py,
                    pz,
                    yaw,
                    pitch,
                    ProjectileKind::Arrow,
                );
                self.broadcast(Clientbound::LevelSound(LevelSound {
                    effect_id: 1002,
                    x: px.floor() as i32,
                    y: py.floor().clamp(0.0, 127.0) as u8,
                    z: pz.floor() as i32,
                    data: 0,
                }));
            }
        } else if stack.id == items::SNOW_BALL || stack.id == items::EGG {
            let kind = if stack.id == items::SNOW_BALL {
                ProjectileKind::Snowball
            } else {
                ProjectileKind::Egg
            };
            if let Some(p) = self.entities.get_by_conn(id) {
                let shooter_id = p.entity_id;
                self.launch_player_projectile(shooter_id, px, py, pz, yaw, pitch, kind);
            }
            new_stack = Some(if stack.count > 1 {
                Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
            } else {
                None
            });
        } else if stack.id == items::FISHING_ROD {
            // ItemFishingRod.java: cast or reel in FishingHook
            let shooter_id = self
                .entities
                .get_by_conn(id)
                .map(|p| p.entity_id)
                .unwrap_or(0);
            let existing_hook = self
                .entities
                .iter_projectiles()
                .find(|pr| {
                    pr.kind == ProjectileKind::FishingHook && pr.shooter_id == Some(shooter_id)
                })
                .map(|pr| (pr.entity_id, pr.x, pr.y, pr.z, pr.in_water_or_catching()));
            if let Some((hook_id, hx, hy, hz, catching)) = existing_hook {
                if self.entities.remove_projectile(hook_id).is_some() {
                    for out in self.tracker.untrack_projectile(hook_id) {
                        if let Some(t) = self.entities.get_player(out.recipient_id) {
                            self.send(t.conn_id, out.packet);
                        }
                    }
                }
                if catching {
                    let dx = px - hx;
                    let dy = py - hy;
                    let dz = pz - hz;
                    let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(0.1);
                    let item_id = self.entities.spawn_dropped_item(
                        ItemStack::new(items::RAW_FISH, 1, 0),
                        hx,
                        hy,
                        hz,
                        dx * 0.1,
                        dy * 0.1 + dist.sqrt() * 0.08,
                        dz * 0.1,
                        0,
                    );
                    let players: Vec<_> = self.entities.iter_players().cloned().collect();
                    if let Some(dropped) = self.entities.get_dropped_item(item_id) {
                        for out in self.tracker.track_item_and_scan(dropped, &players) {
                            if let Some(t) = self.entities.get_player(out.recipient_id) {
                                self.send(t.conn_id, out.packet);
                            }
                        }
                    }
                }
                let mut damaged = stack;
                let broken = damaged.damage_item(1);
                new_stack = Some(if broken { None } else { Some(damaged) });
            } else {
                self.launch_player_projectile(
                    shooter_id,
                    px,
                    py,
                    pz,
                    yaw,
                    pitch,
                    ProjectileKind::FishingHook,
                );
            }
        }

        if let Some(updated) = new_stack {
            let slot_update = if let Some(player) = self.entities.get_by_conn_mut(id) {
                player.inventory.set_item_in_hand(updated);
                let slot_idx = 36 + player.inventory.held_slot as i16;
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

    fn spawn_boat_at(&mut self, x: f64, y: f64, z: f64) {
        let boat_id = self.entities.spawn_boat(x, y, z);
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(boat) = self.entities.get_boat(boat_id) {
            let packets = self.tracker.track_boat_and_scan(boat, &players);
            for outbound in packets {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }
    }

    fn boat_aabb_collides_solid(&self, x: f64, y: f64, z: f64) -> bool {
        let min_x = (x - 0.7).floor() as i32;
        let max_x = (x + 0.7).floor() as i32;
        let min_y = (y - 0.25).floor() as i32;
        let max_y = (y + 0.25).floor() as i32;
        let min_z = (z - 0.7).floor() as i32;
        let max_z = (z + 0.7).floor() as i32;

        for bx in min_x..=max_x {
            for by in min_y..=max_y {
                for bz in min_z..=max_z {
                    let bid = self.world.get_block(bx, by, bz);
                    if blocks::is_solid(bid) && !blocks::is_water(bid) && bid != blocks::SNOW {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn tick_boats(&mut self) {
        let boat_ids: Vec<i32> = self.entities.iter_boats().map(|b| b.entity_id).collect();
        if boat_ids.is_empty() {
            return;
        }
        let mut crashed_boats = Vec::new();
        let players_pos: Vec<(i32, f64, f64, f64, bool)> = self
            .entities
            .iter_players()
            .map(|p| (p.entity_id, p.x, p.y, p.z, p.dead))
            .collect();

        for boat_id in boat_ids {
            let passenger_vel = self
                .entities
                .get_boat(boat_id)
                .and_then(|b| b.passenger_id)
                .and_then(|pid| self.entities.get_player(pid))
                .and_then(|p| {
                    if p.dead {
                        None
                    } else {
                        Some((p.mot_x, p.mot_z))
                    }
                });

            let mut clear_dead_passenger = false;
            if let Some(pid) = self.entities.get_boat(boat_id).and_then(|b| b.passenger_id) {
                if self.entities.get_player(pid).map_or(true, |p| p.dead) {
                    clear_dead_passenger = true;
                }
            }
            if clear_dead_passenger {
                if let Some(b) = self.entities.get_boat_mut(boat_id) {
                    b.passenger_id = None;
                }
            }

            let Some(mut boat_snap) = self.entities.get_boat(boat_id).cloned() else {
                continue;
            };
            if boat_snap.time_since_hit > 0 {
                boat_snap.time_since_hit -= 1;
            }
            if boat_snap.damage > 0 {
                boat_snap.damage -= 1;
            }
            boat_snap.prev_x = boat_snap.x;
            boat_snap.prev_y = boat_snap.y;
            boat_snap.prev_z = boat_snap.z;

            // 5-slice water buoyancy check (`EntityBoat.java:166-177`)
            let min_y = boat_snap.y - 0.3;
            let max_y = boat_snap.y + 0.3;
            let mut d0 = 0.0f64;
            for i in 0..5 {
                let y1 = min_y + (max_y - min_y) * (i as f64) / 5.0 - 0.125;
                let by = y1.floor() as i32;
                let bx = boat_snap.x.floor() as i32;
                let bz = boat_snap.z.floor() as i32;
                if blocks::is_water(self.world.get_block(bx, by, bz)) {
                    d0 += 0.2;
                }
            }

            if d0 < 1.0 {
                let d3 = d0 * 2.0 - 1.0;
                boat_snap.mot_y += 0.03999999910593033 * d3;
            } else {
                if boat_snap.mot_y < 0.0 {
                    boat_snap.mot_y /= 2.0;
                }
                boat_snap.mot_y += 0.007000000216066837;
            }

            if let Some((pmx, pmz)) = passenger_vel {
                boat_snap.mot_x += pmx * 0.2;
                boat_snap.mot_z += pmz * 0.2;
            }

            // Entity collision pushing (`Entity.collide` / `EntityBoat.java:349-360`)
            for &(pid, px, py, pz, pdead) in &players_pos {
                if pdead || Some(pid) == boat_snap.passenger_id {
                    continue;
                }
                let dx = boat_snap.x - px;
                let dz = boat_snap.z - pz;
                if dx.abs() < 1.25 && dz.abs() < 1.25 && (boat_snap.y - py).abs() < 1.5 {
                    let d2 = dx.abs().max(dz.abs());
                    if d2 >= 0.01 {
                        let d2_sqrt = d2.sqrt();
                        let d3 = (1.0 / d2_sqrt).min(1.0);
                        let push_x = (dx / d2_sqrt) * d3 * 0.05;
                        let push_z = (dz / d2_sqrt) * d3 * 0.05;
                        boat_snap.mot_x += push_x;
                        boat_snap.mot_z += push_z;
                    }
                }
            }

            let max_speed = 0.4;
            boat_snap.mot_x = boat_snap.mot_x.clamp(-max_speed, max_speed);
            boat_snap.mot_z = boat_snap.mot_z.clamp(-max_speed, max_speed);

            let below_probe_y = (boat_snap.y - 0.35).floor() as i32;
            let below_probe_id = self.world.get_block(
                boat_snap.x.floor() as i32,
                below_probe_y,
                boat_snap.z.floor() as i32,
            );
            if d0 == 0.0
                && (boat_snap.on_ground
                    || (blocks::is_solid(below_probe_id) && !blocks::is_water(below_probe_id)))
            {
                boat_snap.mot_x *= 0.5;
                boat_snap.mot_y *= 0.5;
                boat_snap.mot_z *= 0.5;
            }

            let pre_move_speed =
                (boat_snap.mot_x * boat_snap.mot_x + boat_snap.mot_z * boat_snap.mot_z).sqrt();
            let next_x = boat_snap.x + boat_snap.mot_x;
            let next_y = boat_snap.y + boat_snap.mot_y;
            let next_z = boat_snap.z + boat_snap.mot_z;

            let mut horiz_collided = false;
            if !self.boat_aabb_collides_solid(next_x, boat_snap.y, boat_snap.z) {
                boat_snap.x = next_x;
            } else {
                horiz_collided = true;
                boat_snap.mot_x = 0.0;
            }

            if !self.boat_aabb_collides_solid(boat_snap.x, boat_snap.y, next_z) {
                boat_snap.z = next_z;
            } else {
                horiz_collided = true;
                boat_snap.mot_z = 0.0;
            }

            let below_y = (next_y - 0.3).floor() as i32;
            let below_id = self.world.get_block(
                boat_snap.x.floor() as i32,
                below_y,
                boat_snap.z.floor() as i32,
            );
            if boat_snap.mot_y < 0.0
                && ((blocks::is_solid(below_id) && !blocks::is_water(below_id))
                    || self.boat_aabb_collides_solid(boat_snap.x, next_y, boat_snap.z))
            {
                boat_snap.y = (below_y + 1) as f64 + 0.3;
                boat_snap.mot_y = 0.0;
                boat_snap.on_ground = true;
            } else {
                boat_snap.y = next_y;
                boat_snap.on_ground = false;
            }

            if horiz_collided && pre_move_speed > 0.15 {
                boat_snap.dead = true;
                crashed_boats.push((
                    boat_id,
                    boat_snap.x,
                    boat_snap.y,
                    boat_snap.z,
                    boat_snap.passenger_id,
                ));
            } else {
                if boat_snap.on_ground && d0 == 0.0 {
                    boat_snap.mot_x *= 0.5;
                    boat_snap.mot_z *= 0.5;
                }
                boat_snap.mot_x *= 0.9900000095367432;
                boat_snap.mot_y *= 0.949999988079071;
                boat_snap.mot_z *= 0.9900000095367432;
            }

            boat_snap.pitch = 0.0;
            let dx = boat_snap.prev_x - boat_snap.x;
            let dz = boat_snap.prev_z - boat_snap.z;
            if dx * dx + dz * dz > 0.001 {
                let target_yaw = (dz.atan2(dx) * 180.0 / std::f64::consts::PI) as f32;
                let mut diff = target_yaw - boat_snap.yaw;
                while diff >= 180.0 {
                    diff -= 360.0;
                }
                while diff < -180.0 {
                    diff += 360.0;
                }
                diff = diff.clamp(-20.0, 20.0);
                boat_snap.yaw += diff;
            }

            // Clear snow layers touched by boat (`EntityBoat.java:362-370`)
            for l in 0..4 {
                let sx = (boat_snap.x + ((l % 2) as f64 - 0.5) * 0.8).floor() as i32;
                let sy = boat_snap.y.floor() as i32;
                let sz = (boat_snap.z + ((l / 2) as f64 - 0.5) * 0.8).floor() as i32;
                if self.world.get_block(sx, sy, sz) == blocks::SNOW {
                    self.world.java_set_type_id(sx, sy, sz, blocks::AIR);
                }
            }

            let mut passenger_sync = None;
            if let Some(pid) = boat_snap.passenger_id {
                let rad = (boat_snap.yaw as f64) * std::f64::consts::PI / 180.0;
                let ox = rad.cos() * 0.4;
                let oz = rad.sin() * 0.4;
                passenger_sync = Some((
                    pid,
                    boat_snap.x + ox,
                    boat_snap.y + boat_snap.mounted_y_offset() + 1.12,
                    boat_snap.z + oz,
                ));
            }
            if let Some(b) = self.entities.get_boat_mut(boat_id) {
                *b = boat_snap;
            }

            if let Some((pid, px, py, pz)) = passenger_sync {
                let mut conn_to_update = None;
                if let Some(p) = self.entities.get_player_mut(pid) {
                    let old_c = p.chunk_pos();
                    p.x = px;
                    p.y = py;
                    p.stance = py;
                    p.z = pz;
                    let new_c = p.chunk_pos();
                    conn_to_update = Some(p.conn_id);
                    self.entities.update_player_chunk(pid, old_c, new_c);
                }
                if let Some(cid) = conn_to_update {
                    self.update_player_view_chunks(cid);
                }
            }
        }

        for (boat_id, bx, by, bz, passenger) in crashed_boats {
            if let Some(pid) = passenger {
                self.dismount_player(pid);
            }
            let _ = self.entities.remove_boat(boat_id);
            let untrack = self.tracker.untrack_boat(boat_id);
            for outbound in untrack {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
            for _ in 0..3 {
                self.spawn_dropped_item_random(
                    bx,
                    by,
                    bz,
                    ItemStack::new(blocks::WOOD_PLANKS as i16, 1, 0),
                    10,
                );
            }
            for _ in 0..2 {
                self.spawn_dropped_item_random(bx, by, bz, ItemStack::new(items::STICK, 1, 0), 10);
            }
        }
    }

    pub fn spawn_mob_at(&mut self, mob_type: MobType, x: f64, y: f64, z: f64) -> i32 {
        let seed = self.world.seed() ^ (self.current_tick as i64);
        let mob_id = self.entities.spawn_mob(mob_type, x, y, z, seed);
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(mob) = self.entities.get_mob(mob_id) {
            for out in self.tracker.track_mob_and_scan(mob, &players) {
                if let Some(target) = self.entities.get_player(out.recipient_id) {
                    self.send(target.conn_id, out.packet);
                }
            }
        }
        mob_id
    }

    fn tick_mob_spawner(&mut self) {
        let player_positions: Vec<(f64, f64, f64)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead)
            .map(|p| (p.x, p.y, p.z))
            .collect();
        if player_positions.is_empty() {
            return;
        }
        let (monsters, animals, water) = self.entities.count_mob_categories();
        let mut rng = std::mem::replace(&mut self.world.tick_random, JavaRandom::new(0));
        let specs = evaluate_natural_spawns(
            &self.world,
            &mut rng,
            &player_positions,
            true,
            true,
            monsters,
            animals,
            water,
        );
        self.world.tick_random = rng;

        for spec in specs {
            let id = self.spawn_mob_at(spec.mob_type, spec.x, spec.y, spec.z);
            if spec.spider_jockey {
                let skel_id = self.spawn_mob_at(MobType::Skeleton, spec.x, spec.y, spec.z);
                if let Some(skel) = self.entities.get_mob_mut(skel_id) {
                    skel.vehicle_id = Some(id);
                }
                if let Some(spider) = self.entities.get_mob_mut(id) {
                    spider.passenger_id = Some(skel_id);
                }
                self.broadcast(Clientbound::AttachEntity(AttachEntity {
                    entity_id: skel_id,
                    vehicle_id: id,
                }));
            }
        }
    }

    /// Java `Entity.m_()` / `EntityLiving.m_()` — fire and lava contact + burn tick damage for players.
    fn tick_player_environmental_damage(&mut self) {
        let player_ids: Vec<i32> = self.entities.iter_players().map(|p| p.entity_id).collect();
        let mut damages: Vec<(i32, i16, &'static str)> = Vec::new();

        for pid in player_ids {
            let Some((px, py, pz, dead, cur_fire)) = self
                .entities
                .get_player(pid)
                .map(|p| (p.x, p.y, p.z, p.dead, p.fire_ticks))
            else {
                continue;
            };
            if dead {
                continue;
            }

            let min_x = (px - 0.3).floor() as i32;
            let max_x = (px + 0.3).floor() as i32;
            let min_y = py.floor() as i32;
            let max_y = (py + 1.79).floor() as i32;
            let min_z = (pz - 0.3).floor() as i32;
            let max_z = (pz + 0.3).floor() as i32;

            let mut in_water = false;
            let mut in_lava = false;
            let mut in_fire = false;

            for bx in min_x..=max_x {
                for by in min_y..=max_y {
                    for bz in min_z..=max_z {
                        let bid = self.world.get_block(bx, by, bz);
                        if blocks::is_water(bid) {
                            in_water = true;
                        } else if blocks::is_lava(bid) {
                            in_lava = true;
                        } else if bid == blocks::FIRE {
                            in_fire = true;
                        }
                    }
                }
            }

            if in_water {
                if let Some(p) = self.entities.get_player_mut(pid) {
                    p.fire_ticks = 0;
                }
                continue;
            }

            if in_lava {
                if let Some(p) = self.entities.get_player_mut(pid) {
                    p.fire_ticks = 600;
                }
                damages.push((pid, 4, "tried to swim in lava"));
                continue;
            }

            if in_fire {
                let mut new_fire = cur_fire + 1;
                if cur_fire == 0 {
                    new_fire = 160;
                    damages.push((pid, 1, "went up in flames"));
                }
                if let Some(p) = self.entities.get_player_mut(pid) {
                    p.fire_ticks = new_fire.max(160);
                }
            } else if cur_fire > 0 && cur_fire % 20 == 0 {
                damages.push((pid, 1, "burned to death"));
            }
        }

        for (pid, dmg, reason) in damages {
            self.damage_player_direct(pid, dmg, reason);
        }
    }

    /// Java `TileEntityMobSpawner.g_()` (`TileEntityMobSpawner.java:31-123`) — tick all loaded mob spawner tile entities.
    fn tick_mob_spawners(&mut self) {
        let player_positions: Vec<(f64, f64, f64)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead)
            .map(|p| (p.x, p.y, p.z))
            .collect();
        if player_positions.is_empty() {
            return;
        }

        // Collect loaded spawner positions
        let mut spawners = Vec::new();
        for chunk in self.world.chunks.values() {
            for te in &chunk.tile_entities {
                if let TileEntityKind::MobSpawner { entity_id, delay } = &te.kind {
                    let sx = te.x as f64 + 0.5;
                    let sy = te.y as f64 + 0.5;
                    let sz = te.z as f64 + 0.5;
                    let player_near = player_positions.iter().any(|&(px, py, pz)| {
                        (px - sx).powi(2) + (py - sy).powi(2) + (pz - sz).powi(2) <= 256.0
                    });
                    if player_near {
                        spawners.push((te.x, te.y, te.z, entity_id.clone(), *delay));
                    }
                }
            }
        }

        if spawners.is_empty() {
            return;
        }

        let sky_sub = world_sky_subtracted(self.world.world_time());
        let mut rng = std::mem::replace(&mut self.world.tick_random, JavaRandom::new(0));
        let mut to_spawn = Vec::new();
        let mut delay_updates = Vec::new();

        for (sx, sy, sz, entity_id, mut delay) in spawners {
            if delay == -1 {
                delay = (200 + rng.next_i32_bound(600)) as i16;
            }
            if delay > 0 {
                delay -= 1;
                delay_updates.push((sx, sy, sz, delay));
                continue;
            }

            let mob_type = MobType::from_nbt_id(&entity_id).unwrap_or(MobType::Pig);
            let nearby_same = self
                .entities
                .iter_mobs()
                .filter(|m| {
                    !m.dead
                        && m.mob_type == mob_type
                        && (m.x - (sx as f64 + 0.5)).abs() <= 8.5
                        && (m.y - (sy as f64 + 0.5)).abs() <= 4.5
                        && (m.z - (sz as f64 + 0.5)).abs() <= 8.5
                })
                .count();

            if nearby_same >= 6 {
                delay = (200 + rng.next_i32_bound(600)) as i16;
                delay_updates.push((sx, sy, sz, delay));
                continue;
            }

            let mut spawned_any = false;
            for _ in 0..4 {
                let mx = sx as f64 + (rng.next_f64() - rng.next_f64()) * 4.0;
                let my = (sy + rng.next_i32_bound(3) - 1) as f64;
                let mz = sz as f64 + (rng.next_f64() - rng.next_f64()) * 4.0;

                let bx = mx.floor() as i32;
                let by = my.floor() as i32;
                let bz = mz.floor() as i32;
                if !(1..126).contains(&by) {
                    continue;
                }
                let b0 = self.world.get_block(bx, by, bz);
                let b1 = self.world.get_block(bx, by + 1, bz);
                if blocks::is_solid(b0)
                    || blocks::is_liquid(b0)
                    || blocks::is_solid(b1)
                    || blocks::is_liquid(b1)
                {
                    continue;
                }

                let can_spawn = if mob_type.is_monster() {
                    chunk_light_at(&self.world, bx, by, bz, sky_sub) <= 7
                } else {
                    // For animal/Pig spawners (including player-placed spawners), allow spawning on any solid floor or grass
                    blocks::is_solid(self.world.get_block(bx, by - 1, bz))
                        || self.world.get_block(sx, sy - 1, sz) != blocks::AIR
                };

                if can_spawn {
                    to_spawn.push((mob_type, mx, my, mz, sx, sy, sz));
                    spawned_any = true;
                }
            }

            if spawned_any {
                delay = (200 + rng.next_i32_bound(600)) as i16;
            }
            delay_updates.push((sx, sy, sz, delay));
        }

        self.world.tick_random = rng;

        for (sx, sy, sz, new_delay) in delay_updates {
            if let Some(te) = self.world.get_tile_entity_mut(sx, sy, sz) {
                if let TileEntityKind::MobSpawner { delay, .. } = &mut te.kind {
                    *delay = new_delay;
                }
            }
        }

        for (mob_type, mx, my, mz, sx, sy, sz) in to_spawn {
            let _ = self.spawn_mob_at(mob_type, mx, my, mz);
            self.broadcast(Clientbound::LevelSound(LevelSound {
                effect_id: 2004,
                x: sx,
                y: sy.clamp(0, 127) as u8,
                z: sz,
                data: 0,
            }));
        }
    }

    fn handle_interact_mob(
        &mut self,
        conn_id: ConnId,
        player_id: i32,
        player_name: &str,
        mob_id: i32,
    ) {
        let held = self
            .entities
            .get_by_conn(conn_id)
            .and_then(|p| p.inventory.item_in_hand());
        let Some((mob_type, mx, my, mz)) = self
            .entities
            .get_mob(mob_id)
            .filter(|m| !m.dead)
            .map(|m| (m.mob_type, m.x, m.y, m.z))
        else {
            return;
        };

        let mut updated_hand: Option<Option<ItemStack>> = None;

        match mob_type {
            MobType::Pig => {
                let saddled = self.entities.get_mob(mob_id).is_some_and(|m| m.pig_saddled);
                if saddled {
                    let cur_rider = self.entities.get_mob(mob_id).and_then(|m| m.passenger_id);
                    if cur_rider == Some(player_id) {
                        self.dismount_player(player_id);
                    } else if cur_rider.is_none() {
                        self.dismount_player(player_id);
                        if let Some(pig) = self.entities.get_mob_mut(mob_id) {
                            pig.passenger_id = Some(player_id);
                        }
                        if let Some(p) = self.entities.get_player_mut(player_id) {
                            p.vehicle_id = Some(mob_id);
                        }
                        self.broadcast(Clientbound::AttachEntity(AttachEntity {
                            entity_id: player_id,
                            vehicle_id: mob_id,
                        }));
                    }
                } else if let Some(stack) = held {
                    if stack.id == items::SADDLE {
                        if let Some(pig) = self.entities.get_mob_mut(mob_id) {
                            pig.pig_saddled = true;
                        }
                        updated_hand = Some(if stack.count > 1 {
                            Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
                        } else {
                            None
                        });
                    }
                }
            }
            MobType::Sheep => {
                if let Some(mut stack) = held {
                    let (sheared, color) = self
                        .entities
                        .get_mob(mob_id)
                        .map(|m| (m.sheep_sheared, m.sheep_color))
                        .unwrap_or((true, 0));
                    if !sheared && stack.id == items::SHEARS {
                        let wool_count = if let Some(sheep) = self.entities.get_mob_mut(mob_id) {
                            sheep.sheep_sheared = true;
                            2 + sheep.rng.next_i32_bound(3)
                        } else {
                            2
                        };
                        for _ in 0..wool_count {
                            self.spawn_dropped_item_random(
                                mx,
                                my + 0.5,
                                mz,
                                ItemStack::new(blocks::WOOL as i16, 1, color as i16),
                                10,
                            );
                        }
                        let broken = stack.damage_item(1);
                        updated_hand = Some(if broken { None } else { Some(stack) });
                    } else if !sheared && stack.id == items::INK_SACK {
                        let new_color = (15 - (stack.damage & 15)) as u8;
                        if new_color != color {
                            if let Some(sheep) = self.entities.get_mob_mut(mob_id) {
                                sheep.sheep_color = new_color;
                            }
                            updated_hand = Some(if stack.count > 1 {
                                Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
                            } else {
                                None
                            });
                        }
                    }
                }
            }
            MobType::Cow => {
                if let Some(stack) = held {
                    if stack.id == items::BUCKET {
                        updated_hand = Some(Some(ItemStack::new(items::MILK_BUCKET, 1, 0)));
                    }
                }
            }
            MobType::Wolf => {
                let (tamed, angry, owner, health) = self
                    .entities
                    .get_mob(mob_id)
                    .map(|m| (m.wolf_tamed, m.wolf_angry, m.wolf_owner.clone(), m.health))
                    .unwrap_or((false, false, String::new(), 8));
                if !tamed {
                    if let Some(stack) = held {
                        if stack.id == items::BONE && !angry {
                            updated_hand = Some(if stack.count > 1 {
                                Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
                            } else {
                                None
                            });
                            let success = self
                                .entities
                                .get_mob_mut(mob_id)
                                .is_some_and(|w| w.rng.next_i32_bound(3) == 0);
                            if success {
                                if let Some(w) = self.entities.get_mob_mut(mob_id) {
                                    w.wolf_tamed = true;
                                    w.wolf_sitting = true;
                                    w.health = 20;
                                    w.max_health = 20;
                                    w.wolf_owner = player_name.to_string();
                                    w.target_id = None;
                                    w.path.clear();
                                }
                                self.broadcast(Clientbound::EntityStatus(EntityStatus {
                                    entity_id: mob_id,
                                    status: 7, // Hearts
                                }));
                            } else {
                                self.broadcast(Clientbound::EntityStatus(EntityStatus {
                                    entity_id: mob_id,
                                    status: 6, // Smoke
                                }));
                            }
                        }
                    }
                } else {
                    let mut ate_meat = false;
                    if let Some(stack) = held {
                        if (stack.id == items::PORK || stack.id == items::GRILLED_PORK)
                            && health < 20
                        {
                            let heal = stack.food_heal_amount();
                            if let Some(w) = self.entities.get_mob_mut(mob_id) {
                                w.health = (w.health + heal).min(20);
                            }
                            updated_hand = Some(if stack.count > 1 {
                                Some(ItemStack::new(stack.id, stack.count - 1, stack.damage))
                            } else {
                                None
                            });
                            ate_meat = true;
                        }
                    }
                    if !ate_meat && owner.eq_ignore_ascii_case(player_name) {
                        if let Some(w) = self.entities.get_mob_mut(mob_id) {
                            w.wolf_sitting = !w.wolf_sitting;
                            w.jumping = false;
                            w.path.clear();
                            w.target_id = None;
                        }
                    }
                }
            }
            _ => {}
        }

        if let Some(new_item) = updated_hand {
            let slot_update = if let Some(p) = self.entities.get_by_conn_mut(conn_id) {
                p.inventory.set_item_in_hand(new_item);
                let slot_idx = 36 + p.inventory.held_slot as i16;
                let item = InventoryPlayer::to_slot_item(p.inventory.item_in_hand());
                Some((slot_idx, item))
            } else {
                None
            };
            if let Some((slot, item)) = slot_update {
                self.send(
                    conn_id,
                    Clientbound::SetSlot(SetSlot {
                        window_id: 0,
                        slot,
                        item,
                    }),
                );
            }
        }
    }

    fn damage_mob_from_player(
        &mut self,
        conn_id: ConnId,
        attacker_id: i32,
        attacker_x: f64,
        attacker_z: f64,
        mob_id: i32,
        damage: i16,
    ) {
        let within_reach = self.entities.get_mob(mob_id).is_some_and(|m| {
            !m.dead && (m.x - attacker_x).powi(2) + (m.z - attacker_z).powi(2) <= 36.0
        });
        if !within_reach {
            return;
        }

        let dealt = self.damage_mob(
            mob_id,
            damage,
            Some((attacker_id, attacker_x, attacker_z)),
            false,
        );
        if dealt {
            // Damage weapon/tool durability on attacker
            if let Some(attacker) = self.entities.get_by_conn_mut(conn_id) {
                let attacker_name = attacker.username.clone();
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
                        attacker
                            .inventory
                            .set_item_in_hand(if broken { None } else { Some(item) });
                        let slot_idx = 36 + attacker.inventory.held_slot as i16;
                        let slot_item =
                            InventoryPlayer::to_slot_item(attacker.inventory.item_in_hand());
                        self.send(
                            conn_id,
                            Clientbound::SetSlot(SetSlot {
                                window_id: 0,
                                slot: slot_idx,
                                item: slot_item,
                            }),
                        );
                    }
                }
                // Command tamed non-sitting wolves owned by attacker to target this mob
                for w in self.entities.iter_mobs_mut() {
                    if w.mob_type == MobType::Wolf
                        && w.wolf_tamed
                        && !w.wolf_sitting
                        && w.entity_id != mob_id
                        && w.wolf_owner.eq_ignore_ascii_case(&attacker_name)
                    {
                        w.target_id = Some(mob_id);
                    }
                }
            }
        }
    }

    pub fn damage_mob(
        &mut self,
        mob_id: i32,
        damage: i16,
        attacker: Option<(i32, f64, f64)>,
        killed_by_skeleton_arrow: bool,
    ) -> bool {
        let mut died = false;
        let mut death_info = None;
        let mut wolf_pack_alert = None;
        let mut pigzombie_alert = None;

        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            if mob.dead || mob.no_damage_ticks > 10 || damage <= 0 {
                return false;
            }
            mob.health -= damage;
            mob.no_damage_ticks = 20;

            if let Some((att_id, ax, az)) = attacker {
                let dx = ax - mob.x;
                let dz = az - mob.z;
                let dist = (dx * dx + dz * dz).sqrt().max(0.001);
                mob.mot_x = mob.mot_x / 2.0 - (dx / dist) * 0.4;
                mob.mot_y = (mob.mot_y / 2.0 + 0.4).min(0.4);
                mob.mot_z = mob.mot_z / 2.0 - (dz / dist) * 0.4;
                mob.velocity_changed = true;

                if mob.mob_type == MobType::Wolf {
                    mob.wolf_sitting = false;
                    if !mob.wolf_tamed {
                        mob.wolf_angry = true;
                        mob.target_id = Some(att_id);
                        wolf_pack_alert = Some((mob.x, mob.y, mob.z, att_id));
                    }
                } else if mob.mob_type == MobType::PigZombie {
                    mob.target_id = Some(att_id);
                    mob.pigzombie_anger = 400 + mob.rng.next_i32_bound(400);
                    pigzombie_alert = Some((mob.x, mob.y, mob.z, att_id));
                } else if mob.mob_type.is_monster() {
                    mob.target_id = Some(att_id);
                } else if mob.mob_type.is_animal() {
                    // Animals panic and pick a new random path
                    mob.path.clear();
                }
            }

            if mob.health <= 0 {
                mob.dead = true;
                died = true;
                let drops = mob.get_death_drops(killed_by_skeleton_arrow);
                death_info = Some((
                    mob.mob_type,
                    mob.x,
                    mob.y,
                    mob.z,
                    mob.slime_size,
                    mob.passenger_id,
                    drops,
                ));
            }
        } else {
            return false;
        }

        if let Some((wx, wy, wz, att_id)) = wolf_pack_alert {
            for w in self.entities.iter_mobs_mut() {
                if w.mob_type == MobType::Wolf
                    && !w.wolf_tamed
                    && w.target_id.is_none()
                    && (w.x - wx).abs() <= 16.0
                    && (w.y - wy).abs() <= 4.0
                    && (w.z - wz).abs() <= 16.0
                {
                    w.wolf_angry = true;
                    w.target_id = Some(att_id);
                }
            }
        }

        if let Some((px, py, pz, att_id)) = pigzombie_alert {
            for pz_mob in self.entities.iter_mobs_mut() {
                if pz_mob.mob_type == MobType::PigZombie
                    && (pz_mob.x - px).abs() <= 32.0
                    && (pz_mob.y - py).abs() <= 32.0
                    && (pz_mob.z - pz).abs() <= 32.0
                {
                    pz_mob.target_id = Some(att_id);
                    pz_mob.pigzombie_anger = 400 + pz_mob.rng.next_i32_bound(400);
                }
            }
        }

        self.broadcast(Clientbound::EntityStatus(EntityStatus {
            entity_id: mob_id,
            status: 2,
        }));

        if died {
            self.broadcast(Clientbound::EntityStatus(EntityStatus {
                entity_id: mob_id,
                status: 3,
            }));
            if let Some((mtype, mx, my, mz, slime_size, passenger_id, drops)) = death_info {
                if let Some(pid) = passenger_id {
                    if self.entities.get_player(pid).is_some() {
                        self.dismount_player(pid);
                    } else if let Some(rider_mob) = self.entities.get_mob_mut(pid) {
                        rider_mob.vehicle_id = None;
                    }
                }
                let _ = self.entities.remove_mob(mob_id);
                for out in self.tracker.untrack_mob(mob_id) {
                    if let Some(target) = self.entities.get_player(out.recipient_id) {
                        self.send(target.conn_id, out.packet);
                    }
                }
                for item in drops {
                    self.spawn_dropped_item_random(mx, my + 0.2, mz, item, 10);
                }
                // EntitySlime.die(): split into 4 slimes of size / 2 when size > 1
                if mtype == MobType::Slime && slime_size > 1 {
                    let half = slime_size / 2;
                    for j in 0..4 {
                        let ox = (((j % 2) as f64) - 0.5) * (slime_size as f64) / 4.0;
                        let oz = (((j / 2) as f64) - 0.5) * (slime_size as f64) / 4.0;
                        let child_id =
                            self.spawn_mob_at(MobType::Slime, mx + ox, my + 0.5, mz + oz);
                        if let Some(child) = self.entities.get_mob_mut(child_id) {
                            child.set_slime_size(half);
                        }
                    }
                }
            }
        }

        true
    }

    fn tick_mobs(&mut self) {
        let mob_ids: Vec<i32> = self.entities.iter_mobs().map(|m| m.entity_id).collect();
        if mob_ids.is_empty() {
            return;
        }
        let sky_sub = world_sky_subtracted(self.world.world_time());
        let is_daytime = sky_sub < 4;
        let players_snapshot: Vec<(i32, String, f64, f64, f64, bool)> = self
            .entities
            .iter_players()
            .map(|p| (p.entity_id, p.username.clone(), p.x, p.y, p.z, p.dead))
            .collect();

        let mut despawn_list = Vec::new();
        let mut eggs_to_drop = Vec::new();
        let mut melee_hits_player = Vec::new();
        let mut melee_hits_mob = Vec::new();
        let mut arrows_to_shoot = Vec::new();
        let mut fireballs_to_shoot = Vec::new();
        let mut creeper_explosions = Vec::new();
        let mut status_packets = Vec::new();
        let mut burned_mobs = Vec::new();

        for mob_id in mob_ids {
            let Some((mtype, mx, my, mz, can_desp)) = self
                .entities
                .get_mob(mob_id)
                .filter(|m| !m.dead)
                .map(|m| (m.mob_type, m.x, m.y, m.z, m.can_despawn()))
            else {
                continue;
            };

            if !self.world.is_chunk_loaded(ChunkPos::new(
                (mx.floor() as i32) >> 4,
                (mz.floor() as i32) >> 4,
            )) {
                continue;
            }

            // Find closest living player and distance squared
            let mut closest_player: Option<(i32, f64, f64, f64, f64)> = None;
            for &(pid, _, px, py, pz, pdead) in &players_snapshot {
                if pdead {
                    continue;
                }
                let dsq = (px - mx).powi(2) + (py - my).powi(2) + (pz - mz).powi(2);
                if closest_player.map_or(true, |(_, _, _, _, best)| dsq < best) {
                    closest_player = Some((pid, px, py, pz, dsq));
                }
            }

            if can_desp {
                if let Some((_, _, _, _, dsq)) = closest_player {
                    if dsq > 128.0 * 128.0 {
                        despawn_list.push(mob_id);
                        continue;
                    }
                    if let Some(m) = self.entities.get_mob_mut(mob_id) {
                        if dsq > 32.0 * 32.0 {
                            m.no_despawn_ticks += 1;
                            if m.no_despawn_ticks > 600 && m.rng.next_i32_bound(800) == 0 {
                                despawn_list.push(mob_id);
                                continue;
                            }
                        } else {
                            m.no_despawn_ticks = 0;
                        }
                    }
                }
            }

            // Daylight combustion for Zombie and Skeleton (`EntityZombie.v()`, `EntitySkeleton.v()`)
            if is_daytime && matches!(mtype, MobType::Zombie | MobType::Skeleton) {
                let bx = mx.floor() as i32;
                let by = my.floor() as i32;
                let bz = mz.floor() as i32;
                let light = chunk_light_at(&self.world, bx, by, bz, sky_sub);
                let can_see_sky = self
                    .world
                    .get_chunk(ChunkPos::new(bx >> 4, bz >> 4))
                    .is_some_and(|c| {
                        c.can_block_see_sky(
                            (bx & 15) as usize,
                            by.clamp(0, 127) as usize,
                            (bz & 15) as usize,
                        )
                    });
                if light > 7 && can_see_sky {
                    if let Some(m) = self.entities.get_mob_mut(mob_id) {
                        if m.fire_ticks <= 0 && !m.in_water {
                            m.fire_ticks = 160;
                        }
                    }
                }
            }

            // Resolve current target position (player or sheep/mob)
            let cur_target_id = self.entities.get_mob(mob_id).and_then(|m| m.target_id);
            let mut target_pos: Option<(i32, bool, f64, f64, f64)> = None;
            if let Some(tid) = cur_target_id {
                if let Some(&(_, _, px, py, pz, pdead)) =
                    players_snapshot.iter().find(|&&(pid, ..)| pid == tid)
                {
                    if !pdead {
                        target_pos = Some((tid, true, px, py, pz));
                    }
                } else if let Some(tmob) = self.entities.get_mob(tid) {
                    if !tmob.dead {
                        target_pos = Some((tid, false, tmob.x, tmob.y, tmob.z));
                    }
                }
            }

            // Acquire new target if none
            if target_pos.is_none() {
                match mtype {
                    MobType::Zombie | MobType::Skeleton | MobType::Creeper | MobType::Giant => {
                        if let Some((pid, px, py, pz, dsq)) = closest_player {
                            if dsq <= 16.0 * 16.0 {
                                target_pos = Some((pid, true, px, py, pz));
                            }
                        }
                    }
                    MobType::Spider => {
                        let bx = mx.floor() as i32;
                        let by = my.floor() as i32;
                        let bz = mz.floor() as i32;
                        let light = chunk_light_at(&self.world, bx, by, bz, sky_sub);
                        if light <= 7 {
                            if let Some((pid, px, py, pz, dsq)) = closest_player {
                                if dsq <= 16.0 * 16.0 {
                                    target_pos = Some((pid, true, px, py, pz));
                                }
                            }
                        }
                    }
                    MobType::Ghast => {
                        if let Some((pid, px, py, pz, dsq)) = closest_player {
                            if dsq <= 100.0 * 100.0 {
                                target_pos = Some((pid, true, px, py, pz));
                            }
                        }
                    }
                    MobType::PigZombie => {
                        let angry = self
                            .entities
                            .get_mob(mob_id)
                            .is_some_and(|m| m.pigzombie_anger > 0);
                        if angry {
                            if let Some((pid, px, py, pz, dsq)) = closest_player {
                                if dsq <= 16.0 * 16.0 {
                                    target_pos = Some((pid, true, px, py, pz));
                                }
                            }
                        }
                    }
                    MobType::Wolf => {
                        let (tamed, angry) = self
                            .entities
                            .get_mob(mob_id)
                            .map(|m| (m.wolf_tamed, m.wolf_angry))
                            .unwrap_or((false, false));
                        if angry {
                            if let Some((pid, px, py, pz, dsq)) = closest_player {
                                if dsq <= 16.0 * 16.0 {
                                    target_pos = Some((pid, true, px, py, pz));
                                }
                            }
                        } else if !tamed {
                            let hunt_sheep = self
                                .entities
                                .get_mob_mut(mob_id)
                                .is_some_and(|m| m.rng.next_i32_bound(100) == 0);
                            if hunt_sheep {
                                let sheep = self.entities.iter_mobs().find_map(|s| {
                                    if !s.dead
                                        && s.mob_type == MobType::Sheep
                                        && (s.x - mx).abs() <= 16.0
                                        && (s.y - my).abs() <= 4.0
                                        && (s.z - mz).abs() <= 16.0
                                    {
                                        Some((s.entity_id, false, s.x, s.y, s.z))
                                    } else {
                                        None
                                    }
                                });
                                target_pos = sheep;
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Spider forgets target in bright daylight with 1/100 chance
            if mtype == MobType::Spider && target_pos.is_some() {
                let bx = mx.floor() as i32;
                let by = my.floor() as i32;
                let bz = mz.floor() as i32;
                if chunk_light_at(&self.world, bx, by, bz, sky_sub) > 7
                    && self
                        .entities
                        .get_mob_mut(mob_id)
                        .is_some_and(|m| m.rng.next_i32_bound(100) == 0)
                {
                    target_pos = None;
                }
            }

            // Owner info for tamed wolf
            let owner_pos = if mtype == MobType::Wolf {
                let owner_name = self
                    .entities
                    .get_mob(mob_id)
                    .filter(|m| m.wolf_tamed && !m.wolf_sitting)
                    .map(|m| m.wolf_owner.clone());
                owner_name.and_then(|name| {
                    players_snapshot
                        .iter()
                        .find(|(_, uname, _, _, _, dead)| {
                            !dead && uname.eq_ignore_ascii_case(&name)
                        })
                        .map(|&(_, _, ox, oy, oz, _)| (ox, oy, oz))
                })
            } else {
                None
            };

            // Compute A* path when needed
            let mut new_path: Option<Vec<(i32, i32, i32)>> = None;
            let mut teleport_to_owner: Option<(f64, f64, f64)> = None;
            if let Some((ox, oy, oz)) = owner_pos {
                let dist = ((ox - mx).powi(2) + (oy - my).powi(2) + (oz - mz).powi(2)).sqrt();
                if dist > 12.0 {
                    // Teleport wolf near owner (`EntityWolf.c`)
                    let bx = ox.floor() as i32 - 2;
                    let by = oy.floor() as i32;
                    let bz = oz.floor() as i32 - 2;
                    'tp: for l in 0..=4 {
                        for i1 in 0..=4 {
                            if (l < 1 || i1 < 1 || l > 3 || i1 > 3)
                                && self.world.is_solid_normal_at(bx + l, by - 1, bz + i1)
                                && !self.world.is_solid_normal_at(bx + l, by, bz + i1)
                                && !self.world.is_solid_normal_at(bx + l, by + 1, bz + i1)
                            {
                                teleport_to_owner = Some((
                                    (bx + l) as f64 + 0.5,
                                    by as f64,
                                    (bz + i1) as f64 + 0.5,
                                ));
                                break 'tp;
                            }
                        }
                    }
                } else if dist > 5.0 {
                    let should_repath = self
                        .entities
                        .get_mob_mut(mob_id)
                        .is_some_and(|m| m.path.is_empty() || m.rng.next_i32_bound(20) == 0);
                    if should_repath {
                        new_path = Some(find_path(
                            &self.world,
                            mx,
                            my,
                            mz,
                            ox.floor() as i32,
                            oy.floor() as i32,
                            oz.floor() as i32,
                            16.0,
                        ));
                    }
                }
            } else if let Some((_, _, tx, ty, tz)) = target_pos {
                if !matches!(mtype, MobType::Ghast | MobType::Slime | MobType::Squid) {
                    let should_repath = self
                        .entities
                        .get_mob_mut(mob_id)
                        .is_some_and(|m| m.path.is_empty() || m.rng.next_i32_bound(20) == 0);
                    if should_repath {
                        new_path = Some(find_path(
                            &self.world,
                            mx,
                            my,
                            mz,
                            tx.floor() as i32,
                            ty.floor() as i32,
                            tz.floor() as i32,
                            16.0,
                        ));
                    }
                }
            } else if !matches!(mtype, MobType::Ghast | MobType::Slime | MobType::Squid) {
                // Random wandering (`EntityCreature.B()`)
                let mut wander_goal = None;
                if let Some(m) = self.entities.get_mob_mut(mob_id) {
                    if !m.wolf_sitting
                        && (m.path.is_empty() && m.rng.next_i32_bound(80) == 0
                            || m.rng.next_i32_bound(80) == 0)
                    {
                        let mut best_score = -99999.0f32;
                        for _ in 0..10 {
                            let rx = (m.x + (m.rng.next_i32_bound(13) - 6) as f64).floor() as i32;
                            let ry = (m.y + (m.rng.next_i32_bound(7) - 3) as f64).floor() as i32;
                            let rz = (m.z + (m.rng.next_i32_bound(13) - 6) as f64).floor() as i32;
                            if !(1..126).contains(&ry) {
                                continue;
                            }
                            let score = if m.mob_type.is_animal() {
                                if self.world.get_block(rx, ry - 1, rz) == blocks::GRASS {
                                    10.0
                                } else {
                                    (chunk_light_at(&self.world, rx, ry, rz, sky_sub) as f32) / 15.0
                                        - 0.5
                                }
                            } else {
                                0.5 - (chunk_light_at(&self.world, rx, ry, rz, sky_sub) as f32)
                                    / 15.0
                            };
                            if score > best_score {
                                best_score = score;
                                wander_goal = Some((rx, ry, rz));
                            }
                        }
                    }
                }
                if let Some((gx, gy, gz)) = wander_goal {
                    new_path = Some(find_path(&self.world, mx, my, mz, gx, gy, gz, 10.0));
                }
            }

            // Now update the mob state and physics
            let mut rider_sync = None;
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                mob.target_id = target_pos.map(|(tid, ..)| tid);
                if let Some((tx, ty, tz)) = teleport_to_owner {
                    mob.x = tx;
                    mob.y = ty;
                    mob.z = tz;
                    mob.mot_x = 0.0;
                    mob.mot_y = 0.0;
                    mob.mot_z = 0.0;
                    mob.path.clear();
                } else if let Some(p) = new_path {
                    mob.path = p;
                    mob.path_index = 0;
                }

                if mob.no_damage_ticks > 0 {
                    mob.no_damage_ticks -= 1;
                }
                if mob.attack_ticks > 0 {
                    mob.attack_ticks -= 1;
                }
                if mob.pigzombie_anger > 0 {
                    mob.pigzombie_anger -= 1;
                }
                if mob.fire_ticks > 0 {
                    if mob.fire_proof() || mob.in_water {
                        mob.fire_ticks = 0;
                    } else {
                        if mob.fire_ticks % 20 == 0 {
                            burned_mobs.push(mob_id);
                        }
                        mob.fire_ticks -= 1;
                    }
                }

                let bx = mob.x.floor() as i32;
                let by = mob.y.floor() as i32;
                let bz = mob.z.floor() as i32;
                let cur_block = self.world.get_block(bx, by, bz);
                mob.in_water = blocks::is_water(cur_block);
                if blocks::is_lava(cur_block) && !mob.fire_proof() {
                    mob.fire_ticks = 600;
                    burned_mobs.push(mob_id);
                } else if cur_block == blocks::FIRE && !mob.fire_proof() {
                    if mob.fire_ticks == 0 {
                        burned_mobs.push(mob_id);
                    }
                    mob.fire_ticks = mob.fire_ticks.max(160);
                }

                // Chicken egg timer (`EntityChicken.v()`)
                if mob.mob_type == MobType::Chicken {
                    mob.chicken_egg_timer -= 1;
                    if mob.chicken_egg_timer <= 0 {
                        eggs_to_drop.push((mob.x, mob.y, mob.z));
                        mob.chicken_egg_timer = mob.rng.next_i32_bound(6000) + 6000;
                    }
                }

                // Wolf water shaking (`EntityWolf.v()`, `m_()`)
                if mob.mob_type == MobType::Wolf {
                    if mob.in_water {
                        mob.wolf_wet = true;
                        mob.wolf_shaking = false;
                        mob.wolf_shake_time = 0.0;
                        mob.wolf_sitting = false;
                    } else if mob.wolf_wet
                        && !mob.wolf_shaking
                        && mob.on_ground
                        && mob.path.is_empty()
                    {
                        mob.wolf_shaking = true;
                        mob.wolf_shake_time = 0.0;
                        status_packets.push((mob_id, 8));
                    } else if mob.wolf_shaking {
                        mob.wolf_shake_time += 0.05;
                        if mob.wolf_shake_time >= 2.0 {
                            mob.wolf_wet = false;
                            mob.wolf_shaking = false;
                            mob.wolf_shake_time = 0.0;
                        }
                    }
                }

                // Specific AI movement & attacks
                match mob.mob_type {
                    MobType::Squid => {
                        if mob.rng.next_i32_bound(50) == 0
                            || !mob.in_water
                            || (mob.squid_swim_vec.0 == 0.0
                                && mob.squid_swim_vec.1 == 0.0
                                && mob.squid_swim_vec.2 == 0.0)
                        {
                            let angle = mob.rng.next_f32() * std::f32::consts::PI * 2.0;
                            mob.squid_swim_vec = (
                                angle.cos() * 0.2,
                                -0.1 + mob.rng.next_f32() * 0.2,
                                angle.sin() * 0.2,
                            );
                        }
                        if mob.in_water {
                            mob.mot_x = mob.squid_swim_vec.0 as f64;
                            mob.mot_y = mob.squid_swim_vec.1 as f64;
                            mob.mot_z = mob.squid_swim_vec.2 as f64;
                            mob.yaw =
                                (-mob.mot_x.atan2(mob.mot_z) * 180.0 / std::f64::consts::PI) as f32;
                        } else {
                            mob.mot_x = 0.0;
                            mob.mot_y -= 0.08;
                            mob.mot_y *= 0.98;
                            mob.mot_z = 0.0;
                        }
                    }
                    MobType::Slime => {
                        if let Some((pid, px, py, pz, dsq)) = closest_player {
                            if dsq <= 16.0 * 16.0 {
                                let dx = px - mob.x;
                                let dz = pz - mob.z;
                                mob.yaw =
                                    (dz.atan2(dx) * 180.0 / std::f64::consts::PI) as f32 - 90.0;
                                let reach = 0.6 * (mob.slime_size as f64);
                                if mob.slime_size > 1
                                    && dsq < reach * reach
                                    && (py - mob.y).abs() < 2.0
                                {
                                    melee_hits_player.push((
                                        pid,
                                        mob.slime_size as i16,
                                        mob.x,
                                        mob.z,
                                    ));
                                }
                            }
                        }
                        if mob.on_ground {
                            mob.slime_jump_delay -= 1;
                            if mob.slime_jump_delay <= 0 {
                                mob.slime_jump_delay = mob.rng.next_i32_bound(20) + 10;
                                if closest_player.is_some_and(|(_, _, _, _, dsq)| dsq <= 256.0) {
                                    mob.slime_jump_delay /= 3;
                                }
                                let rad = (mob.yaw + 90.0) * std::f32::consts::PI / 180.0;
                                mob.mot_x = (rad.cos() * 0.25) as f64;
                                mob.mot_z = (rad.sin() * 0.25) as f64;
                                mob.mot_y = 0.42;
                            } else {
                                mob.mot_x = 0.0;
                                mob.mot_z = 0.0;
                            }
                        }
                    }
                    MobType::Ghast => {
                        let dx = mob.ghast_waypoint.0 - mob.x;
                        let dy = mob.ghast_waypoint.1 - mob.y;
                        let dz = mob.ghast_waypoint.2 - mob.z;
                        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                        if !(1.0..=60.0).contains(&dist) {
                            mob.ghast_waypoint = (
                                mob.x + ((mob.rng.next_f32() * 2.0 - 1.0) * 16.0) as f64,
                                (mob.y + ((mob.rng.next_f32() * 2.0 - 1.0) * 16.0) as f64)
                                    .clamp(10.0, 115.0),
                                mob.z + ((mob.rng.next_f32() * 2.0 - 1.0) * 16.0) as f64,
                            );
                        } else {
                            mob.ghast_course_cooldown -= 1;
                            if mob.ghast_course_cooldown <= 0 {
                                mob.ghast_course_cooldown += mob.rng.next_i32_bound(5) + 2;
                                mob.mot_x += dx / dist * 0.1;
                                mob.mot_y += dy / dist * 0.1;
                                mob.mot_z += dz / dist * 0.1;
                            }
                        }

                        if let Some((_, _, tx, ty, tz)) = target_pos {
                            let tdx = tx - mob.x;
                            let tdy = (ty + 1.0) - (mob.y + 2.0);
                            let tdz = tz - mob.z;
                            if tdx * tdx + tdy * tdy + tdz * tdz < 64.0 * 64.0 {
                                mob.yaw = (-tdx.atan2(tdz) * 180.0 / std::f64::consts::PI) as f32;
                                mob.ghast_charge += 1;
                                if mob.ghast_charge == 20 {
                                    fireballs_to_shoot.push((
                                        mob_id,
                                        mob.x,
                                        mob.y + 2.0,
                                        mob.z,
                                        tdx,
                                        tdy,
                                        tdz,
                                    ));
                                    mob.ghast_charge = -40;
                                }
                            } else if mob.ghast_charge > 0 {
                                mob.ghast_charge -= 1;
                            }
                        } else {
                            mob.yaw =
                                (-mob.mot_x.atan2(mob.mot_z) * 180.0 / std::f64::consts::PI) as f32;
                            if mob.ghast_charge > 0 {
                                mob.ghast_charge -= 1;
                            }
                        }
                    }
                    _ => {
                        // Standard path-following for Creature / Animal / Monster
                        let mut move_forward = false;
                        if !mob.wolf_sitting && !mob.wolf_shaking {
                            while mob.path_index < mob.path.len() {
                                let (wx, wy, wz) = mob.path[mob.path_index];
                                let dx = (wx as f64 + 0.5) - mob.x;
                                let dz = (wz as f64 + 0.5) - mob.z;
                                if dx * dx + dz * dz < 0.64 {
                                    mob.path_index += 1;
                                } else {
                                    let target_yaw =
                                        (dz.atan2(dx) * 180.0 / std::f64::consts::PI) as f32 - 90.0;
                                    let mut diff = target_yaw - mob.yaw;
                                    while diff < -180.0 {
                                        diff += 360.0;
                                    }
                                    while diff >= 180.0 {
                                        diff -= 360.0;
                                    }
                                    mob.yaw += diff.clamp(-30.0, 30.0);
                                    move_forward = true;
                                    if (wy as f64) > mob.y + 0.1 && mob.on_ground {
                                        mob.jumping = true;
                                    }
                                    break;
                                }
                            }
                            if mob.path_index >= mob.path.len() {
                                mob.path.clear();
                                mob.path_index = 0;
                            }
                        }

                        // Target engagement (Melee / Creeper fuse / Skeleton bow / Spider & Wolf leap)
                        if let Some((tid, is_player, tx, ty, tz)) = target_pos {
                            let dx = tx - mob.x;
                            let dy = ty - mob.y;
                            let dz = tz - mob.z;
                            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                            mob.yaw = (dz.atan2(dx) * 180.0 / std::f64::consts::PI) as f32 - 90.0;

                            match mob.mob_type {
                                MobType::Creeper => {
                                    if (mob.creeper_state > 0 && dist < 7.0) || dist < 3.0 {
                                        mob.creeper_state = 1;
                                        mob.creeper_fuse += 1;
                                        move_forward = false;
                                        if mob.creeper_fuse >= 30 {
                                            let radius =
                                                if mob.creeper_powered { 6.0 } else { 3.0 };
                                            mob.dead = true;
                                            creeper_explosions
                                                .push((mob_id, mob.x, mob.y, mob.z, radius));
                                        }
                                    } else {
                                        mob.creeper_state = -1;
                                        if mob.creeper_fuse > 0 {
                                            mob.creeper_fuse -= 1;
                                        }
                                    }
                                }
                                MobType::Skeleton => {
                                    if dist < 10.0 {
                                        if mob.attack_ticks == 0 {
                                            arrows_to_shoot.push((
                                                mob_id,
                                                mob.x,
                                                mob.y + 1.0,
                                                mob.z,
                                                dx,
                                                (ty + 1.4) - (mob.y + 1.2),
                                                dz,
                                            ));
                                            mob.attack_ticks = 30;
                                        }
                                        if dist < 5.0 {
                                            move_forward = false;
                                        }
                                    }
                                }
                                MobType::Spider | MobType::Wolf => {
                                    if dist > 2.0
                                        && dist < 6.0
                                        && mob.on_ground
                                        && mob.rng.next_i32_bound(10) == 0
                                    {
                                        let h = (dx * dx + dz * dz).sqrt().max(0.01);
                                        mob.mot_x = dx / h * 0.4 + mob.mot_x * 0.2;
                                        mob.mot_z = dz / h * 0.4 + mob.mot_z * 0.2;
                                        mob.mot_y = 0.4;
                                    } else if dist < 2.0 && dy.abs() < 2.0 && mob.attack_ticks <= 0
                                    {
                                        mob.attack_ticks = 20;
                                        let dmg = mob
                                            .mob_type
                                            .attack_damage(mob.slime_size, mob.wolf_tamed);
                                        if is_player {
                                            melee_hits_player.push((tid, dmg, mob.x, mob.z));
                                        } else {
                                            melee_hits_mob.push((tid, dmg, mob_id, mob.x, mob.z));
                                        }
                                    }
                                }
                                _ => {
                                    let dmg =
                                        mob.mob_type.attack_damage(mob.slime_size, mob.wolf_tamed);
                                    if dmg > 0
                                        && dist < 2.0
                                        && dy.abs() < 2.0
                                        && mob.attack_ticks <= 0
                                    {
                                        mob.attack_ticks = 20;
                                        if is_player {
                                            melee_hits_player.push((tid, dmg, mob.x, mob.z));
                                        } else {
                                            melee_hits_mob.push((tid, dmg, mob_id, mob.x, mob.z));
                                        }
                                    }
                                }
                            }
                        } else if mob.mob_type == MobType::Creeper && mob.creeper_fuse > 0 {
                            mob.creeper_state = -1;
                            mob.creeper_fuse -= 1;
                        }

                        let speed = if mob.mob_type == MobType::PigZombie && mob.target_id.is_some()
                        {
                            0.18
                        } else {
                            (mob.mob_type.move_speed() * 0.15) as f64
                        };

                        if move_forward {
                            let rad = (mob.yaw + 90.0) * std::f32::consts::PI / 180.0;
                            mob.mot_x += (rad.cos() as f64) * speed * 0.4;
                            mob.mot_z += (rad.sin() as f64) * speed * 0.4;
                        }
                    }
                }

                // Step & gravity physics for non-flying / non-squid mobs
                if !matches!(mob.mob_type, MobType::Ghast | MobType::Squid) {
                    if mob.in_water {
                        mob.mot_y += 0.04;
                    } else {
                        mob.mot_y -= 0.08;
                    }
                    if mob.mob_type == MobType::Chicken && !mob.on_ground && mob.mot_y < 0.0 {
                        mob.mot_y *= 0.6;
                    }
                    if mob.jumping && (mob.on_ground || mob.in_water) {
                        mob.mot_y = 0.42;
                        mob.jumping = false;
                    }
                }

                let next_x = mob.x + mob.mot_x;
                let next_y = mob.y + mob.mot_y;
                let next_z = mob.z + mob.mot_z;

                mob.horiz_collided = false;
                let cy = mob.y.floor() as i32;
                if !blocks::is_solid(self.world.get_block(
                    next_x.floor() as i32,
                    cy,
                    mob.z.floor() as i32,
                )) {
                    mob.x = next_x;
                } else if !blocks::is_solid(self.world.get_block(
                    next_x.floor() as i32,
                    cy + 1,
                    mob.z.floor() as i32,
                )) && !blocks::is_solid(self.world.get_block(
                    next_x.floor() as i32,
                    cy + 2,
                    mob.z.floor() as i32,
                )) && mob.on_ground
                {
                    mob.x = next_x;
                    mob.y = (cy + 1) as f64;
                } else {
                    mob.horiz_collided = true;
                    mob.mot_x = 0.0;
                }

                let cy2 = mob.y.floor() as i32;
                if !blocks::is_solid(self.world.get_block(
                    mob.x.floor() as i32,
                    cy2,
                    next_z.floor() as i32,
                )) {
                    mob.z = next_z;
                } else if !blocks::is_solid(self.world.get_block(
                    mob.x.floor() as i32,
                    cy2 + 1,
                    next_z.floor() as i32,
                )) && !blocks::is_solid(self.world.get_block(
                    mob.x.floor() as i32,
                    cy2 + 2,
                    next_z.floor() as i32,
                )) && mob.on_ground
                {
                    mob.z = next_z;
                    mob.y = (cy2 + 1) as f64;
                } else {
                    mob.horiz_collided = true;
                    mob.mot_z = 0.0;
                }

                // Spider wall climbing (`EntitySpider.p()`)
                if mob.mob_type == MobType::Spider && mob.horiz_collided {
                    mob.mot_y = 0.2;
                }

                let below_y = next_y.floor() as i32;
                let below_id =
                    self.world
                        .get_block(mob.x.floor() as i32, below_y, mob.z.floor() as i32);
                if mob.mot_y < 0.0 && blocks::is_solid(below_id) {
                    mob.y = (below_y + 1) as f64;
                    mob.mot_y = 0.0;
                    mob.on_ground = true;
                } else {
                    mob.y = next_y.clamp(1.0, 126.0);
                    mob.on_ground = false;
                }

                let drag = if mob.mob_type == MobType::Ghast {
                    0.91
                } else if mob.on_ground {
                    0.546
                } else {
                    0.91
                };
                mob.mot_x *= drag;
                mob.mot_y *= if mob.mob_type == MobType::Ghast {
                    0.91
                } else {
                    0.98
                };
                mob.mot_z *= drag;

                if let Some(pid) = mob.passenger_id {
                    rider_sync = Some((pid, mob.x, mob.y + mob.mounted_y_offset(), mob.z));
                }
            }

            if let Some((pid, rx, ry, rz)) = rider_sync {
                if let Some(p) = self.entities.get_player_mut(pid) {
                    let old_c = p.chunk_pos();
                    p.x = rx;
                    p.y = ry;
                    p.stance = ry;
                    p.z = rz;
                    let new_c = p.chunk_pos();
                    self.entities.update_player_chunk(pid, old_c, new_c);
                } else if let Some(rmob) = self.entities.get_mob_mut(pid) {
                    rmob.x = rx;
                    rmob.y = ry;
                    rmob.z = rz;
                }
            }
        }

        for mob_id in despawn_list {
            let _ = self.entities.remove_mob(mob_id);
            for out in self.tracker.untrack_mob(mob_id) {
                if let Some(target) = self.entities.get_player(out.recipient_id) {
                    self.send(target.conn_id, out.packet);
                }
            }
        }

        for (ex, ey, ez) in eggs_to_drop {
            self.spawn_dropped_item_random(ex, ey, ez, ItemStack::new(items::EGG, 1, 0), 10);
        }

        for (entity_id, status) in status_packets {
            self.broadcast(Clientbound::EntityStatus(EntityStatus {
                entity_id,
                status,
            }));
        }

        for mob_id in burned_mobs {
            let _ = self.damage_mob(mob_id, 1, None, false);
        }

        for (pid, dmg, ax, az) in melee_hits_player {
            self.damage_player_from_mob(pid, dmg, ax, az);
        }

        for (target_mob_id, dmg, att_id, ax, az) in melee_hits_mob {
            let _ = self.damage_mob(target_mob_id, dmg, Some((att_id, ax, az)), false);
        }

        for (skel_id, sx, sy, sz, dx, dy, dz) in arrows_to_shoot {
            let h = (dx * dx + dz * dz).sqrt() * 0.2;
            let len = (dx * dx + (dy + h).powi(2) + dz * dz).sqrt().max(0.001);
            let vx = (dx / len) * 0.6;
            let vy = ((dy + h) / len) * 0.6;
            let vz = (dz / len) * 0.6;
            self.spawn_projectile_at(
                ProjectileKind::Arrow,
                Some(skel_id),
                true,
                false,
                sx,
                sy,
                sz,
                vx,
                vy,
                vz,
            );
            self.broadcast(Clientbound::LevelSound(LevelSound {
                effect_id: 1002,
                x: sx.floor() as i32,
                y: sy.floor().clamp(0.0, 127.0) as u8,
                z: sz.floor() as i32,
                data: 0,
            }));
        }

        for (ghast_id, gx, gy, gz, dx, dy, dz) in fireballs_to_shoot {
            let len = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
            let vx = (dx / len) * 0.9;
            let vy = (dy / len) * 0.9;
            let vz = (dz / len) * 0.9;
            self.spawn_projectile_at(
                ProjectileKind::Fireball,
                Some(ghast_id),
                false,
                false,
                gx + (dx / len) * 4.0,
                gy,
                gz + (dz / len) * 4.0,
                vx,
                vy,
                vz,
            );
        }

        for (creeper_id, cx, cy, cz, radius) in creeper_explosions {
            let _ = self.entities.remove_mob(creeper_id);
            for out in self.tracker.untrack_mob(creeper_id) {
                if let Some(target) = self.entities.get_player(out.recipient_id) {
                    self.send(target.conn_id, out.packet);
                }
            }
            self.create_explosion(cx, cy, cz, radius, false);
        }
    }

    fn damage_player_from_mob(
        &mut self,
        player_id: i32,
        damage: i16,
        attacker_x: f64,
        attacker_z: f64,
    ) {
        let mut hit = None;
        if let Some(p) = self.entities.get_player_mut(player_id) {
            if p.dead {
                return;
            }
            let (dealt, died) = p.take_damage(damage);
            if dealt <= 0 {
                return;
            }
            p.apply_knockback(attacker_x, attacker_z);
            let drops = if died {
                let px = p.x;
                let py = p.y;
                let pz = p.z;
                p.inventory
                    .drop_all_on_death()
                    .into_iter()
                    .map(|it| (px, py, pz, it))
                    .collect()
            } else {
                Vec::new()
            };
            hit = Some((
                p.conn_id,
                p.username.clone(),
                p.health,
                died,
                (p.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16,
                (p.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16,
                (p.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16,
                drops,
            ));
        }

        if let Some((conn_id, username, health, died, mx, my, mz, drops)) = hit {
            let vel = Clientbound::EntityVelocity(EntityVelocity {
                entity_id: player_id,
                mot_x: mx,
                mot_y: my,
                mot_z: mz,
            });
            self.send(conn_id, vel.clone());
            self.broadcast(vel);
            self.broadcast(Clientbound::EntityStatus(EntityStatus {
                entity_id: player_id,
                status: 2,
            }));
            self.send(conn_id, Clientbound::UpdateHealth(UpdateHealth { health }));
            if died {
                self.dismount_player(player_id);
                for (dx, dy, dz, stack) in drops {
                    self.spawn_dropped_item_random(dx, dy, dz, stack, 40);
                }
                self.broadcast_chat(&format!("\u{00A7}e{username} was slain by a mob"));
                self.broadcast(Clientbound::EntityStatus(EntityStatus {
                    entity_id: player_id,
                    status: 3,
                }));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Projectiles, Primed TNT, Minecarts, Paintings, Explosions
    // ---------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn launch_player_projectile(
        &mut self,
        shooter_id: i32,
        px: f64,
        py: f64,
        pz: f64,
        yaw: f32,
        pitch: f32,
        kind: ProjectileKind,
    ) {
        let deg = std::f32::consts::PI / 180.0;
        let speed = if kind == ProjectileKind::FishingHook {
            0.75
        } else {
            1.5
        };
        let mx = (-(yaw * deg).sin() * (pitch * deg).cos()) as f64 * speed;
        let mz = ((yaw * deg).cos() * (pitch * deg).cos()) as f64 * speed;
        let my = (-(pitch * deg).sin()) as f64 * speed;
        self.spawn_projectile_at(
            kind,
            Some(shooter_id),
            false,
            true,
            px,
            py + 1.52,
            pz,
            mx,
            my,
            mz,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_projectile_at(
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
    ) {
        let pid = self.entities.spawn_projectile(
            kind,
            shooter_id,
            shooter_is_skeleton,
            from_player,
            x,
            y,
            z,
            mot_x,
            mot_y,
            mot_z,
        );
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(proj) = self.entities.get_projectile(pid) {
            for out in self.tracker.track_projectile_and_scan(proj, &players) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
        }
    }

    fn tick_projectiles(&mut self) {
        let proj_ids: Vec<i32> = self
            .entities
            .iter_projectiles()
            .map(|p| p.entity_id)
            .collect();
        if proj_ids.is_empty() {
            return;
        }

        let mut to_remove = Vec::new();
        let mut mob_hits = Vec::new();
        let mut player_hits = Vec::new();
        let mut egg_hatches = Vec::new();
        let mut fireball_explosions = Vec::new();
        let mut arrow_pickups = Vec::new();

        for pid in proj_ids {
            let Some(proj_snapshot) = self.entities.get_projectile(pid).cloned() else {
                continue;
            };
            if proj_snapshot.dead {
                to_remove.push(pid);
                continue;
            }

            if proj_snapshot.in_ground {
                if proj_snapshot.kind == ProjectileKind::Arrow && proj_snapshot.from_player {
                    // Check if a player walks over the stuck arrow to pick it up
                    let collector = self.entities.iter_players().find_map(|p| {
                        if !p.dead
                            && (p.x - proj_snapshot.x).abs() <= 1.2
                            && (p.y - proj_snapshot.y).abs() <= 1.8
                            && (p.z - proj_snapshot.z).abs() <= 1.2
                        {
                            Some((p.entity_id, p.conn_id))
                        } else {
                            None
                        }
                    });
                    if let Some((peid, pconn)) = collector {
                        arrow_pickups.push((pid, peid, pconn));
                        continue;
                    }
                }
                if let Some(p) = self.entities.get_projectile_mut(pid) {
                    p.ticks_in_ground += 1;
                    if p.ticks_in_ground >= 1200 {
                        to_remove.push(pid);
                    }
                }
                continue;
            }

            let next_x = proj_snapshot.x + proj_snapshot.mot_x;
            let next_y = proj_snapshot.y + proj_snapshot.mot_y;
            let next_z = proj_snapshot.z + proj_snapshot.mot_z;

            // Check entity collisions along segment
            let hit_mob = self.entities.iter_mobs().find_map(|m| {
                if m.dead || Some(m.entity_id) == proj_snapshot.shooter_id {
                    return None;
                }
                let (w, h) = m.size();
                let hw = (w as f64) * 0.5 + 0.3;
                if (next_x - m.x).abs() <= hw
                    && next_y >= m.y - 0.2
                    && next_y <= m.y + (h as f64) + 0.2
                    && (next_z - m.z).abs() <= hw
                {
                    Some(m.entity_id)
                } else {
                    None
                }
            });

            if let Some(mob_id) = hit_mob {
                let dmg = match proj_snapshot.kind {
                    ProjectileKind::Arrow => 4,
                    ProjectileKind::Fireball => 6,
                    _ => 0,
                };
                mob_hits.push((
                    mob_id,
                    dmg,
                    proj_snapshot.shooter_id,
                    proj_snapshot.x,
                    proj_snapshot.z,
                    proj_snapshot.shooter_is_skeleton,
                ));
                if proj_snapshot.kind == ProjectileKind::Egg {
                    egg_hatches.push((next_x, next_y, next_z));
                } else if proj_snapshot.kind == ProjectileKind::Fireball {
                    fireball_explosions.push((next_x, next_y, next_z));
                }
                if proj_snapshot.kind != ProjectileKind::FishingHook {
                    to_remove.push(pid);
                    continue;
                }
            }

            let hit_player = self.entities.iter_players().find_map(|p| {
                if p.dead
                    || (Some(p.entity_id) == proj_snapshot.shooter_id
                        && proj_snapshot.ticks_in_air < 5)
                {
                    return None;
                }
                if (next_x - p.x).abs() <= 0.6
                    && next_y >= p.y - 0.2
                    && next_y <= p.y + 1.9
                    && (next_z - p.z).abs() <= 0.6
                {
                    Some(p.entity_id)
                } else {
                    None
                }
            });

            if let Some(player_id) = hit_player {
                let dmg = match proj_snapshot.kind {
                    ProjectileKind::Arrow => 4,
                    ProjectileKind::Fireball => 5,
                    _ => 0,
                };
                if dmg > 0 {
                    player_hits.push((player_id, dmg, proj_snapshot.x, proj_snapshot.z));
                }
                if proj_snapshot.kind == ProjectileKind::Egg {
                    egg_hatches.push((next_x, next_y, next_z));
                } else if proj_snapshot.kind == ProjectileKind::Fireball {
                    fireball_explosions.push((next_x, next_y, next_z));
                }
                if proj_snapshot.kind != ProjectileKind::FishingHook {
                    to_remove.push(pid);
                    continue;
                }
            }

            // Check block collision
            let bx = next_x.floor() as i32;
            let by = next_y.floor() as i32;
            let bz = next_z.floor() as i32;
            let hit_block =
                !(0..128).contains(&by) || blocks::is_solid(self.world.get_block(bx, by, bz));

            if hit_block {
                match proj_snapshot.kind {
                    ProjectileKind::Arrow => {
                        if let Some(p) = self.entities.get_projectile_mut(pid) {
                            p.x = next_x;
                            p.y = next_y;
                            p.z = next_z;
                            p.mot_x = 0.0;
                            p.mot_y = 0.0;
                            p.mot_z = 0.0;
                            p.in_ground = true;
                        }
                    }
                    ProjectileKind::Egg => {
                        egg_hatches.push((proj_snapshot.x, proj_snapshot.y, proj_snapshot.z));
                        to_remove.push(pid);
                    }
                    ProjectileKind::Snowball => {
                        to_remove.push(pid);
                    }
                    ProjectileKind::Fireball => {
                        fireball_explosions.push((
                            proj_snapshot.x,
                            proj_snapshot.y,
                            proj_snapshot.z,
                        ));
                        to_remove.push(pid);
                    }
                    ProjectileKind::FishingHook => {
                        if let Some(p) = self.entities.get_projectile_mut(pid) {
                            p.mot_x = 0.0;
                            p.mot_y = 0.0;
                            p.mot_z = 0.0;
                            p.in_ground = true;
                        }
                    }
                }
                continue;
            }

            if let Some(p) = self.entities.get_projectile_mut(pid) {
                p.ticks_in_air += 1;
                p.x = next_x;
                p.y = next_y;
                p.z = next_z;

                match p.kind {
                    ProjectileKind::FishingHook => {
                        if blocks::is_water(self.world.get_block(bx, by, bz)) {
                            p.mot_y += 0.04;
                            p.mot_x *= 0.8;
                            p.mot_z *= 0.8;
                            if p.catch_ticks > 0 {
                                p.catch_ticks -= 1;
                            } else if (self.current_tick + pid as u64) % 100 == 0 {
                                p.catch_ticks = 40;
                                p.mot_y = -0.2;
                            }
                        } else {
                            p.mot_y -= 0.03;
                            p.mot_x *= 0.92;
                            p.mot_y *= 0.92;
                            p.mot_z *= 0.92;
                        }
                    }
                    ProjectileKind::Fireball => {
                        p.mot_x = (p.mot_x + p.accel_x) * 0.95;
                        p.mot_y = (p.mot_y + p.accel_y) * 0.95;
                        p.mot_z = (p.mot_z + p.accel_z) * 0.95;
                        if p.ticks_in_air > 200 {
                            to_remove.push(pid);
                        }
                    }
                    _ => {
                        p.mot_x *= 0.99;
                        p.mot_y = p.mot_y * 0.99 - 0.03;
                        p.mot_z *= 0.99;
                    }
                }
            }
        }

        for (pid, peid, pconn) in arrow_pickups {
            let added = if let Some(player) = self.entities.get_player_mut(peid) {
                if player
                    .inventory
                    .add_item(ItemStack::new(items::ARROW, 1, 0))
                {
                    Some(player.inventory.to_window_items())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(win_items) = added {
                self.broadcast(Clientbound::Collect(Collect {
                    collected_id: pid,
                    collector_id: peid,
                }));
                self.send(
                    pconn,
                    Clientbound::WindowItems(WindowItems {
                        window_id: 0,
                        items: win_items,
                    }),
                );
                to_remove.push(pid);
            }
        }

        for pid in to_remove {
            let _ = self.entities.remove_projectile(pid);
            for out in self.tracker.untrack_projectile(pid) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
        }

        for (mob_id, dmg, shooter_id, sx, sz, is_skel) in mob_hits {
            let att = shooter_id.map(|sid| (sid, sx, sz));
            let _ = self.damage_mob(mob_id, dmg, att, is_skel);
        }

        for (player_id, dmg, sx, sz) in player_hits {
            self.damage_player_from_mob(player_id, dmg, sx, sz);
        }

        for (ex, ey, ez) in egg_hatches {
            // EntityEgg.java:39-51 — 1/8 chance to hatch 1 chicken (1/32 for 4 chickens)
            let mut rng = std::mem::replace(&mut self.world.tick_random, JavaRandom::new(0));
            let count = if rng.next_i32_bound(8) == 0 {
                if rng.next_i32_bound(32) == 0 {
                    4
                } else {
                    1
                }
            } else {
                0
            };
            self.world.tick_random = rng;
            for _ in 0..count {
                self.spawn_mob_at(MobType::Chicken, ex, ey, ez);
            }
        }

        for (fx, fy, fz) in fireball_explosions {
            self.create_explosion(fx, fy, fz, 1.0, true);
        }
    }

    fn spawn_primed_tnt_at(&mut self, x: f64, y: f64, z: f64, fuse: i32) {
        let tid = self.entities.spawn_primed_tnt(x, y, z, fuse);
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(tnt) = self.entities.get_primed_tnt(tid) {
            for out in self.tracker.track_tnt_and_scan(tnt, &players) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
        }
    }

    fn tick_primed_tnts(&mut self) {
        let tnt_ids: Vec<i32> = self
            .entities
            .iter_primed_tnts()
            .map(|t| t.entity_id)
            .collect();
        if tnt_ids.is_empty() {
            return;
        }
        let mut exploded = Vec::new();

        for tid in tnt_ids {
            if let Some(tnt) = self.entities.get_primed_tnt_mut(tid) {
                tnt.mot_y -= 0.04;
                let nx = tnt.x + tnt.mot_x;
                let ny = tnt.y + tnt.mot_y;
                let nz = tnt.z + tnt.mot_z;

                if !blocks::is_solid(self.world.get_block(
                    nx.floor() as i32,
                    tnt.y.floor() as i32,
                    tnt.z.floor() as i32,
                )) {
                    tnt.x = nx;
                } else {
                    tnt.mot_x = 0.0;
                }

                if !blocks::is_solid(self.world.get_block(
                    tnt.x.floor() as i32,
                    tnt.y.floor() as i32,
                    nz.floor() as i32,
                )) {
                    tnt.z = nz;
                } else {
                    tnt.mot_z = 0.0;
                }

                let by = (ny - 0.49).floor() as i32;
                if tnt.mot_y < 0.0
                    && blocks::is_solid(self.world.get_block(
                        tnt.x.floor() as i32,
                        by,
                        tnt.z.floor() as i32,
                    ))
                {
                    tnt.y = (by + 1) as f64 + 0.49;
                    tnt.mot_y *= -0.5;
                    tnt.mot_x *= 0.7;
                    tnt.mot_z *= 0.7;
                    tnt.on_ground = true;
                } else {
                    tnt.y = ny;
                    tnt.on_ground = false;
                }

                tnt.mot_x *= 0.98;
                tnt.mot_y *= 0.98;
                tnt.mot_z *= 0.98;
                tnt.fuse -= 1;
                if tnt.fuse <= 0 {
                    tnt.dead = true;
                    exploded.push((tid, tnt.x, tnt.y, tnt.z));
                }
            }
        }

        for (tid, tx, ty, tz) in exploded {
            let _ = self.entities.remove_primed_tnt(tid);
            for out in self.tracker.untrack_tnt(tid) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
            self.create_explosion(tx, ty, tz, 4.0, false);
        }
    }

    fn spawn_minecart_at(&mut self, cart_type: u8, x: f64, y: f64, z: f64) {
        let cid = self.entities.spawn_minecart(cart_type, x, y, z);
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(cart) = self.entities.get_minecart(cid) {
            for out in self.tracker.track_minecart_and_scan(cart, &players) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
        }
    }

    fn handle_interact_minecart(&mut self, conn_id: ConnId, player_id: i32, cart_id: i32) {
        let Some((ctype, cur_rider)) = self
            .entities
            .get_minecart(cart_id)
            .filter(|c| !c.dead)
            .map(|c| (c.cart_type, c.passenger_id))
        else {
            return;
        };
        if ctype == 0 {
            if cur_rider == Some(player_id) {
                self.dismount_player(player_id);
            } else if cur_rider.is_none() {
                self.dismount_player(player_id);
                if let Some(c) = self.entities.get_minecart_mut(cart_id) {
                    c.passenger_id = Some(player_id);
                }
                if let Some(p) = self.entities.get_player_mut(player_id) {
                    p.vehicle_id = Some(cart_id);
                }
                self.broadcast(Clientbound::AttachEntity(AttachEntity {
                    entity_id: player_id,
                    vehicle_id: cart_id,
                }));
            }
        } else if ctype == 2 {
            // Powered minecart: right-click with COAL adds 3600 ticks of fuel and pushes in player look dir
            let (px, pz, has_coal) = self
                .entities
                .get_by_conn(conn_id)
                .map(|p| {
                    (
                        p.x,
                        p.z,
                        p.inventory
                            .item_in_hand()
                            .is_some_and(|s| s.id == items::COAL),
                    )
                })
                .unwrap_or((0.0, 0.0, false));
            if has_coal {
                if let Some(c) = self.entities.get_minecart_mut(cart_id) {
                    c.fuel += 3600;
                    c.push_x = c.x - px;
                    c.push_z = c.z - pz;
                }
            }
        }
    }

    fn damage_minecart(&mut self, cart_id: i32, raw_damage: i32) {
        let mut destroyed = None;
        if let Some(cart) = self.entities.get_minecart_mut(cart_id) {
            cart.damage += raw_damage * 10;
            if cart.damage > 40 {
                cart.dead = true;
                destroyed = Some((cart.cart_type, cart.x, cart.y, cart.z, cart.passenger_id));
            }
        }
        if let Some((ctype, cx, cy, cz, passenger)) = destroyed {
            if let Some(pid) = passenger {
                self.dismount_player(pid);
            }
            let _ = self.entities.remove_minecart(cart_id);
            for out in self.tracker.untrack_minecart(cart_id) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
            self.spawn_dropped_item_random(cx, cy, cz, ItemStack::new(items::MINECART, 1, 0), 10);
            if ctype == 1 {
                self.spawn_dropped_item_random(
                    cx,
                    cy,
                    cz,
                    ItemStack::new(blocks::CHEST as i16, 1, 0),
                    10,
                );
            } else if ctype == 2 {
                self.spawn_dropped_item_random(
                    cx,
                    cy,
                    cz,
                    ItemStack::new(blocks::FURNACE as i16, 1, 0),
                    10,
                );
            }
        }
    }

    fn tick_minecarts(&mut self) {
        let cart_ids: Vec<i32> = self
            .entities
            .iter_minecarts()
            .map(|c| c.entity_id)
            .collect();
        if cart_ids.is_empty() {
            return;
        }

        for cid in cart_ids {
            let passenger_vel = self
                .entities
                .get_minecart(cid)
                .and_then(|c| c.passenger_id)
                .and_then(|pid| self.entities.get_player(pid))
                .map(|p| (p.mot_x, p.mot_z));

            let mut detector_pos = None;
            let mut rider_sync = None;
            if let Some(cart) = self.entities.get_minecart_mut(cid) {
                if cart.damage > 0 {
                    cart.damage -= 1;
                }
                let bx = cart.x.floor() as i32;
                let by = cart.y.floor() as i32;
                let bz = cart.z.floor() as i32;
                let block_id = self.world.get_block(bx, by, bz);

                if matches!(
                    block_id,
                    blocks::RAIL | blocks::POWERED_RAIL | blocks::DETECTOR_RAIL
                ) {
                    let meta = self.world.get_data(bx, by, bz);
                    let shape =
                        if block_id == blocks::POWERED_RAIL || block_id == blocks::DETECTOR_RAIL {
                            meta & 7
                        } else {
                            meta
                        };
                    cart.y = by as f64 + 0.1 + if (2..=5).contains(&shape) { 0.5 } else { 0.0 };
                    if let Some((pmx, pmz)) = passenger_vel {
                        cart.mot_x += pmx * 0.1;
                        cart.mot_z += pmz * 0.1;
                    }
                    if cart.fuel > 0 {
                        cart.fuel -= 1;
                        let d = (cart.push_x * cart.push_x + cart.push_z * cart.push_z).sqrt();
                        if d > 0.01 {
                            cart.mot_x += (cart.push_x / d) * 0.04;
                            cart.mot_z += (cart.push_z / d) * 0.04;
                        }
                    }
                    // Slope gravity (`EntityMinecart.java:244-260`)
                    match shape {
                        2 => cart.mot_x -= 0.0078125,
                        3 => cart.mot_x += 0.0078125,
                        4 => cart.mot_z += 0.0078125,
                        5 => cart.mot_z -= 0.0078125,
                        _ => {}
                    }
                    // Curved track redirection (`6..=9`)
                    match shape {
                        6 => {
                            if cart.mot_x < 0.0 {
                                cart.mot_z = -cart.mot_x;
                                cart.mot_x = 0.0;
                            } else if cart.mot_z < 0.0 {
                                cart.mot_x = -cart.mot_z;
                                cart.mot_z = 0.0;
                            }
                        }
                        7 => {
                            if cart.mot_x > 0.0 {
                                cart.mot_z = cart.mot_x;
                                cart.mot_x = 0.0;
                            } else if cart.mot_z < 0.0 {
                                cart.mot_x = cart.mot_z;
                                cart.mot_z = 0.0;
                            }
                        }
                        8 => {
                            if cart.mot_x > 0.0 {
                                cart.mot_z = -cart.mot_x;
                                cart.mot_x = 0.0;
                            } else if cart.mot_z > 0.0 {
                                cart.mot_x = -cart.mot_z;
                                cart.mot_z = 0.0;
                            }
                        }
                        9 => {
                            if cart.mot_x < 0.0 {
                                cart.mot_z = cart.mot_x;
                                cart.mot_x = 0.0;
                            } else if cart.mot_z > 0.0 {
                                cart.mot_x = cart.mot_z;
                                cart.mot_z = 0.0;
                            }
                        }
                        0 | 4 | 5 => cart.mot_x = 0.0,
                        1..=3 => cart.mot_z = 0.0,
                        _ => {}
                    }
                    if block_id == blocks::POWERED_RAIL {
                        if (meta & 8) != 0 {
                            let spd = (cart.mot_x * cart.mot_x + cart.mot_z * cart.mot_z).sqrt();
                            if spd > 0.01 {
                                cart.mot_x += (cart.mot_x / spd) * 0.06;
                                cart.mot_z += (cart.mot_z / spd) * 0.06;
                            }
                        } else {
                            cart.mot_x *= 0.5;
                            cart.mot_z *= 0.5;
                        }
                    } else if block_id == blocks::DETECTOR_RAIL && (meta & 8) == 0 {
                        detector_pos = Some((bx, by, bz, meta));
                    }
                    cart.mot_x = cart.mot_x.clamp(-0.4, 0.4) * 0.99;
                    cart.mot_z = cart.mot_z.clamp(-0.4, 0.4) * 0.99;
                    cart.x += cart.mot_x;
                    cart.z += cart.mot_z;
                    // Step up/down onto connected slope rail
                    let nbx = cart.x.floor() as i32;
                    let nbz = cart.z.floor() as i32;
                    if (nbx != bx || nbz != bz) && self.world.is_rail_at(nbx, by + 1, nbz) {
                        cart.y = (by + 1) as f64 + 0.1;
                    } else if (nbx != bx || nbz != bz) && self.world.is_rail_at(nbx, by - 1, nbz) {
                        cart.y = (by - 1) as f64 + 0.1;
                    }
                } else {
                    cart.mot_y -= 0.04;
                    cart.x += cart.mot_x;
                    cart.y += cart.mot_y;
                    cart.z += cart.mot_z;
                    if blocks::is_solid(self.world.get_block(bx, (cart.y - 0.2).floor() as i32, bz))
                    {
                        cart.y = by as f64;
                        cart.mot_y = 0.0;
                        cart.mot_x *= 0.5;
                        cart.mot_z *= 0.5;
                    }
                }
                if let Some(pid) = cart.passenger_id {
                    rider_sync = Some((pid, cart.x, cart.y + 0.85, cart.z));
                }
            }

            if let Some((dx, dy, dz, meta)) = detector_pos {
                self.world.java_set_data(dx, dy, dz, meta | 8);
                self.world
                    .java_apply_physics(dx, dy - 1, dz, blocks::DETECTOR_RAIL as i32);
                self.world
                    .java_schedule_tick(dx, dy, dz, blocks::DETECTOR_RAIL, 20);
            }
            if let Some((pid, rx, ry, rz)) = rider_sync {
                let mut conn_to_update = None;
                if let Some(p) = self.entities.get_player_mut(pid) {
                    let old_c = p.chunk_pos();
                    p.x = rx;
                    p.y = ry;
                    p.stance = ry;
                    p.z = rz;
                    let new_c = p.chunk_pos();
                    conn_to_update = Some(p.conn_id);
                    self.entities.update_player_chunk(pid, old_c, new_c);
                }
                if let Some(cid) = conn_to_update {
                    self.update_player_view_chunks(cid);
                }
            }
        }
    }

    fn spawn_painting_at(&mut self, title: String, x: i32, y: i32, z: i32, direction: i32) {
        let pid = self.entities.spawn_painting(title, x, y, z, direction);
        let players: Vec<_> = self.entities.iter_players().cloned().collect();
        if let Some(p) = self.entities.get_painting(pid) {
            for out in self.tracker.track_painting_and_scan(p, &players) {
                if let Some(t) = self.entities.get_player(out.recipient_id) {
                    self.send(t.conn_id, out.packet);
                }
            }
        }
    }

    fn tick_pressure_plates_and_beds(&mut self) {
        let mut plate_positions = Vec::new();
        for p in self.entities.iter_players() {
            if p.dead {
                continue;
            }
            let bx = p.x.floor() as i32;
            let by = p.y.floor() as i32;
            let bz = p.z.floor() as i32;
            let bid = self.world.get_block(bx, by, bz);
            if bid == blocks::STONE_PLATE || bid == blocks::WOOD_PLATE {
                plate_positions.push((bx, by, bz, bid));
            }
        }
        for m in self.entities.iter_mobs() {
            if m.dead {
                continue;
            }
            let bx = m.x.floor() as i32;
            let by = m.y.floor() as i32;
            let bz = m.z.floor() as i32;
            let bid = self.world.get_block(bx, by, bz);
            if bid == blocks::STONE_PLATE || bid == blocks::WOOD_PLATE {
                plate_positions.push((bx, by, bz, bid));
            }
        }
        for it in self.entities.iter_dropped_items() {
            if it.dead {
                continue;
            }
            let bx = it.x.floor() as i32;
            let by = it.y.floor() as i32;
            let bz = it.z.floor() as i32;
            if self.world.get_block(bx, by, bz) == blocks::WOOD_PLATE {
                plate_positions.push((bx, by, bz, blocks::WOOD_PLATE));
            }
        }

        for (px, py, pz, pid) in plate_positions {
            self.world.set_pressure_plate_state(px, py, pz, pid, true);
        }

        // Bed sleep check (`World.everyoneDeeplySleeping()`)
        let all_sleeping = self.entities.player_count() > 0
            && self.entities.iter_players().all(|p| !p.dead && p.sleeping);
        if all_sleeping {
            let cur_time = self.world.world_time();
            let next_morning = (cur_time + 24000) - (cur_time + 24000).rem_euclid(24000);
            self.world.live_time = next_morning;
            self.world.level_data.borrow_mut().time = next_morning;

            let mut woke_players = Vec::new();
            for p in self.entities.iter_players_mut() {
                p.sleeping = false;
                p.bed_pos = None;
                woke_players.push(p.entity_id);
            }
            for eid in woke_players {
                self.broadcast(Clientbound::ArmAnimation(ArmAnimation {
                    entity_id: eid,
                    animate: 3, // Leave bed animation
                }));
            }
            self.broadcast(Clientbound::UpdateTime(UpdateTime { time: next_morning }));
        }
    }

    fn process_world_events(&mut self) {
        let events = self.world.take_world_events();
        for ev in events {
            match ev {
                WorldEvent::DropItem { x, y, z, item } => {
                    self.spawn_dropped_item_random(x, y, z, item, 10);
                }
                WorldEvent::DispenseItem {
                    x,
                    y,
                    z,
                    dx,
                    dz,
                    item,
                } => {
                    let mot_x = (dx as f64) * 0.2;
                    let mot_z = (dz as f64) * 0.2;
                    let item_id = self
                        .entities
                        .spawn_dropped_item(item, x, y, z, mot_x, 0.2, mot_z, 10);
                    let players: Vec<_> = self.entities.iter_players().cloned().collect();
                    if let Some(dropped) = self.entities.get_dropped_item(item_id) {
                        for out in self.tracker.track_item_and_scan(dropped, &players) {
                            if let Some(t) = self.entities.get_player(out.recipient_id) {
                                self.send(t.conn_id, out.packet);
                            }
                        }
                    }
                }
                WorldEvent::DispenseProjectile {
                    x,
                    y,
                    z,
                    dx,
                    dy,
                    dz,
                    item_id,
                } => {
                    let kind = match item_id {
                        items::ARROW => ProjectileKind::Arrow,
                        items::EGG => ProjectileKind::Egg,
                        _ => ProjectileKind::Snowball,
                    };
                    self.spawn_projectile_at(
                        kind,
                        None,
                        false,
                        true,
                        x,
                        y,
                        z,
                        dx * 1.1,
                        dy,
                        dz * 1.1,
                    );
                }
                WorldEvent::SpawnPrimedTnt { x, y, z, fuse } => {
                    let bx = x.floor() as i32;
                    let by = y.floor() as i32;
                    let bz = z.floor() as i32;
                    self.send_block_change_to_observers(bx, by, bz);
                    self.spawn_primed_tnt_at(x, y, z, fuse);
                }
                WorldEvent::PlayNote {
                    x,
                    y,
                    z,
                    instrument,
                    pitch,
                } => {
                    self.broadcast(Clientbound::PlayNoteBlock(PlayNoteBlock {
                        x,
                        y,
                        z,
                        instrument,
                        pitch,
                    }));
                }
                WorldEvent::LevelSound {
                    effect_id,
                    x,
                    y,
                    z,
                    data,
                } => {
                    self.broadcast(Clientbound::LevelSound(LevelSound {
                        effect_id,
                        x,
                        y,
                        z,
                        data,
                    }));
                }
            }
        }
    }

    /// Java `Explosion.java` — 16x16x16 raycast block destruction, entity damage/knockback, and `Packet60Explosion`.
    pub fn create_explosion(&mut self, ex: f64, ey: f64, ez: f64, radius: f32, set_fire: bool) {
        let mut destroyed_blocks = HashSet::new();
        let mut rng = std::mem::replace(&mut self.world.tick_random, JavaRandom::new(0));

        for i in 0..16 {
            for j in 0..16 {
                for k in 0..16 {
                    if i == 0 || i == 15 || j == 0 || j == 15 || k == 0 || k == 15 {
                        let mut dx = (i as f64) / 15.0 * 2.0 - 1.0;
                        let mut dy = (j as f64) / 15.0 * 2.0 - 1.0;
                        let mut dz = (k as f64) / 15.0 * 2.0 - 1.0;
                        let len = (dx * dx + dy * dy + dz * dz).sqrt();
                        dx /= len;
                        dy /= len;
                        dz /= len;

                        let mut power = radius * (0.7 + rng.next_f32() * 0.6);
                        let mut rx = ex;
                        let mut ry = ey;
                        let mut rz = ez;

                        while power > 0.0 {
                            let bx = rx.floor() as i32;
                            let by = ry.floor() as i32;
                            let bz = rz.floor() as i32;
                            if !(0..128).contains(&by) {
                                break;
                            }
                            let id = self.world.get_block(bx, by, bz);
                            if id != blocks::AIR {
                                let res = Self::block_blast_resistance(id);
                                power -= (res + 0.3) * 0.3;
                            }
                            if power > 0.0 && id != blocks::BEDROCK {
                                destroyed_blocks.insert((bx, by, bz));
                            }
                            rx += dx * 0.3;
                            ry += dy * 0.3;
                            rz += dz * 0.3;
                            power -= 0.225;
                        }
                    }
                }
            }
        }

        // Entity damage within `radius * 2.0`
        let blast_size = (radius * 2.0) as f64;
        let player_ids: Vec<i32> = self.entities.iter_players().map(|p| p.entity_id).collect();
        for pid in player_ids {
            let dist_info = self.entities.get_player(pid).and_then(|p| {
                if p.dead {
                    return None;
                }
                let dx = p.x - ex;
                let dy = p.y - ey;
                let dz = p.z - ez;
                let d = (dx * dx + dy * dy + dz * dz).sqrt();
                if d <= blast_size {
                    Some((d, dx, dy, dz))
                } else {
                    None
                }
            });
            if let Some((d, _dx, _dy, _dz)) = dist_info {
                let d7 = d / blast_size;
                let impact = 1.0 - d7;
                let dmg =
                    (((impact * impact + impact) / 2.0 * 8.0 * blast_size + 1.0) as i16).max(1);
                self.damage_player_from_mob(pid, dmg, ex, ez);
            }
        }

        let mob_ids: Vec<i32> = self.entities.iter_mobs().map(|m| m.entity_id).collect();
        for mid in mob_ids {
            let dist_info = self.entities.get_mob(mid).and_then(|m| {
                if m.dead {
                    return None;
                }
                let dx = m.x - ex;
                let dy = m.y - ey;
                let dz = m.z - ez;
                let d = (dx * dx + dy * dy + dz * dz).sqrt();
                if d <= blast_size {
                    Some(d)
                } else {
                    None
                }
            });
            if let Some(d) = dist_info {
                let impact = 1.0 - (d / blast_size);
                let dmg =
                    (((impact * impact + impact) / 2.0 * 8.0 * blast_size + 1.0) as i16).max(1);
                let _ = self.damage_mob(mid, dmg, Some((0, ex, ez)), false);
            }
        }

        // Apply block destruction and 30% drop rate
        // Java `Packet60Explosion.java:36-38, 55-57` uses `(int) this.a` (truncation toward zero), NOT floor!
        let mut records = Vec::with_capacity(destroyed_blocks.len());
        let base_x = ex as i32;
        let base_y = ey as i32;
        let base_z = ez as i32;
        let mut drops_to_spawn = Vec::new();
        let mut chain_tnts = Vec::new();

        for &(bx, by, bz) in &destroyed_blocks {
            let id = self.world.get_block(bx, by, bz);
            let meta = self.world.get_data(bx, by, bz);
            if id != blocks::AIR {
                if id == blocks::TNT {
                    chain_tnts.push((bx, by, bz));
                } else if rng.next_f32() <= 0.3 {
                    for item in blocks::get_block_drops(id, meta, &mut rng, None) {
                        drops_to_spawn.push((bx as f64, by as f64, bz as f64, item));
                    }
                }
                self.world.java_set_type_id(bx, by, bz, blocks::AIR);
            }
            if set_fire
                && self.world.get_block(bx, by, bz) == blocks::AIR
                && blocks::is_opaque_cube(self.world.get_block(bx, by - 1, bz))
                && rng.next_i32_bound(3) == 0
            {
                self.world.java_set_type_id(bx, by, bz, blocks::FIRE);
            }
            records.push((
                (bx - base_x).clamp(-128, 127) as i8,
                (by - base_y).clamp(-128, 127) as i8,
                (bz - base_z).clamp(-128, 127) as i8,
            ));
        }
        self.world.tick_random = rng;

        for (tx, ty, tz) in chain_tnts {
            self.world.ignite_tnt(tx, ty, tz, true);
        }
        for (dx, dy, dz, item) in drops_to_spawn {
            self.spawn_dropped_item_random(dx, dy, dz, item, 10);
        }
        self.process_world_events();

        self.broadcast(Clientbound::Explosion(Explosion {
            x: ex,
            y: ey,
            z: ez,
            radius,
            records,
        }));
        self.flush_dirty_chunks();
    }

    fn block_blast_resistance(id: u8) -> f32 {
        match id {
            blocks::BEDROCK => 18_000_000.0,
            blocks::OBSIDIAN => 2000.0,
            blocks::WATER | blocks::STATIONARY_WATER | blocks::LAVA | blocks::STATIONARY_LAVA => {
                100.0
            }
            blocks::STONE
            | blocks::COBBLESTONE
            | blocks::MOSSY_COBBLESTONE
            | blocks::BRICK
            | blocks::DOUBLE_STEP
            | blocks::STEP
            | blocks::COBBLESTONE_STAIRS
            | blocks::IRON_BLOCK
            | blocks::GOLD_BLOCK
            | blocks::DIAMOND_BLOCK => 6.0,
            blocks::FURNACE | blocks::LIT_FURNACE | blocks::DISPENSER => 3.5,
            _ => blocks::hardness(id).max(0.0),
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
        let dirty_list = self.world.take_dirty_chunks();
        if dirty_list.is_empty() {
            return;
        }
        while self.world.java_do_lighting() {}

        for (pos, dirty, watchers) in dirty_list {
            if dirty.full_resync || dirty.count >= 64 {
                if let Ok((pre, map)) = self.world.prepare_chunk_packets(pos) {
                    let pkt = Clientbound::MapChunk(map);
                    for conn_id in watchers {
                        let should_send = self
                            .entities
                            .get_by_conn(conn_id)
                            .is_some_and(|p| p.sent_chunks.contains(&pos));
                        if should_send {
                            let need_pre = self
                                .entities
                                .get_by_conn_mut(conn_id)
                                .is_some_and(|p| p.pre_chunk_sent.insert(pos));
                            if need_pre {
                                self.send(conn_id, Clientbound::PreChunk(pre));
                            }
                            self.send(conn_id, pkt.clone());
                            if let Some(chunk) = self.world.get_chunk(pos) {
                                for te in &chunk.tile_entities {
                                    if let TileEntityKind::Sign { lines } = &te.kind {
                                        self.send(
                                            conn_id,
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
            } else if dirty.count == 1 {
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
            } else if dirty.count > 1 {
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
            }
        }
    }

    fn handle_respawn(&mut self, id: ConnId) {
        if self.world.live_is_active() {
            self.world.live_player_quit(id);
        }
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();

        // Untrack player in EntityTracker before respawn (`ServerConfigurationManager.moveToWorld`)
        // so both the respawned player and all observers clear stale tracked_players state
        // and re-receive NamedEntitySpawn packets after respawn!
        if let Some(p) = self.entities.get_by_conn(id) {
            let eid = p.entity_id;
            self.dismount_player(eid);
            let untrack = self.tracker.untrack_player(eid);
            for outbound in untrack {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }

        // Poseidon double-respawn client hack: send opposite dimension first, then dimension 0
        self.send(id, Clientbound::Respawn(Respawn { dimension: -1 }));
        self.send(id, Clientbound::Respawn(Respawn { dimension: 0 }));

        let mut respawned_eid = None;
        if let Some(player) = self.entities.get_by_conn_mut(id) {
            let old_chunk = player.chunk_pos();
            player.respawn(spawn_x as f64 + 0.5, spawn_y as f64, spawn_z as f64 + 0.5);
            let new_chunk = player.chunk_pos();
            let entity_id = player.entity_id;
            respawned_eid = Some(entity_id);
            self.entities
                .update_player_chunk(entity_id, old_chunk, new_chunk);
        }

        self.stream_initial_chunks(id);

        if let Some(eid) = respawned_eid {
            if let Some(player) = self.entities.get_player(eid) {
                self.tracker.track_player(player);
            }
        }

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

        self.run_tracker_tick();
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

        let name = login.username.trim();
        // Poseidon username validity check (`poseidon.yml`: length 3..=16, `[a-zA-Z0-9_.]*`)
        if name.len() < 3 {
            self.kick(
                id,
                "Sorry, your username is too short. The minimum length is: 3",
            );
            return;
        }
        if name.len() > 16 {
            self.kick(
                id,
                "Sorry, your username is too long. The maximum length is: 16",
            );
            return;
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '?')
        {
            self.kick(
                id,
                "Sorry, your username is invalid, allowed characters: [a-zA-Z0-9_.]*",
            );
            return;
        }

        let lower_name = name.to_lowercase();
        let peer_ip = self
            .connections
            .get(&id)
            .map(|c| c.address.ip().to_string())
            .unwrap_or_default();

        if self.banned_players.contains(&lower_name) {
            self.kick(id, "You are banned from this server!");
            return;
        }
        if self.banned_ips.contains(&peer_ip.to_lowercase()) {
            self.kick(id, "Your IP address is banned from this server!");
            return;
        }
        if self.whitelist_enabled
            && !self.whitelist.contains(&lower_name)
            && !self.ops.contains(&lower_name)
        {
            self.kick(id, "You are not white-listed on this server!");
            return;
        }
        if self.entities.player_count() >= self.config.server.max_players as usize {
            self.kick(id, "The server is full!");
            return;
        }

        // Prevent duplicate username login (`LoginProcessHandler.java:222`, `poseidon.yml`)
        let already_online = self
            .entities
            .iter_players()
            .any(|p| p.username.eq_ignore_ascii_case(name));
        if already_online {
            self.kick(
                id,
                "\u{00A7}cA player with your username or uuid is already online, try reconnecting in a minute.",
            );
            return;
        }

        let seed = self.world.seed();
        let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();
        let player_x = spawn_x as f64 + 0.5;
        let player_y = spawn_y as f64;
        let player_z = spawn_z as f64 + 0.5;

        let entity_id =
            self.entities
                .insert_player(id, name.to_string(), player_x, player_y, player_z);

        if self.config.world.save_enabled {
            let mut chunk_shift = None;
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let old_c = player.chunk_pos();
                let _ = self.player_store.load_player(player);
                player.last_eval_x = player.x;
                player.last_eval_z = player.z;
                let new_c = player.chunk_pos();
                if old_c != new_c {
                    chunk_shift = Some((old_c, new_c));
                }
            }
            if let Some((old_c, new_c)) = chunk_shift {
                self.entities.update_player_chunk(entity_id, old_c, new_c);
            }
        }

        if let Some(connection) = self.connections.get_mut(&id) {
            connection.state = ConnectionState::Playing;
            connection.username = Some(name.to_string());
            connection.entity_id = entity_id;
        }

        if let Some(player) = self.entities.get_player(entity_id) {
            self.tracker.track_player(player);
        }

        log::info(&format!("{name} logged in with entity id {entity_id}"));

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

        // Poseidon: PlayerJoinEvent -> broadcast join message to other players
        let join_msg = self
            .config
            .server
            .join_message
            .replacen("%player%", name, 1);
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
            // Flush any population dirty records for already-online players before inserting `id`'s sent_chunks
            self.flush_dirty_chunks();
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

        // Send immediate area synchronously so player spawns on solid terrain
        let immediate_radius = if self.world.live_is_active() { 2 } else { 1 };
        for dz in -immediate_radius..=immediate_radius {
            for dx in -immediate_radius..=immediate_radius {
                let pos = ChunkPos::new(center.x + dx, center.z + dz);
                if self.world.live_is_active() {
                    self.world.server_get_chunk_at(pos);
                } else {
                    self.world.populate_chunk(pos);
                }
            }
        }
        if self.world.live_is_active() {
            while self.world.java_do_lighting() {}
            self.flush_dirty_chunks();
        }
        for dz in -immediate_radius..=immediate_radius {
            for dx in -immediate_radius..=immediate_radius {
                let pos = ChunkPos::new(center.x + dx, center.z + dz);
                if let Ok((pre, map)) = self.world.prepare_chunk_packets(pos) {
                    self.send(id, Clientbound::PreChunk(pre));
                    self.send(id, Clientbound::MapChunk(map));
                    if let Some(player) = self.entities.get_by_conn_mut(id) {
                        player.pre_chunk_sent.insert(pos);
                        player.sent_chunks.insert(pos);
                    }
                    if let Some(chunk) = self.world.get_chunk(pos) {
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

        // Queue remaining chunks in spiral order (populated inner chunks before unpopulated outer border)
        if let Some(player) = self.entities.get_by_conn_mut(id) {
            for pos in spiral {
                if !player.sent_chunks.contains(&pos) {
                    player.chunk_queue.push(pos);
                }
            }
            if self.world.live_is_active() {
                player.chunk_queue.sort_by_key(|&p| {
                    let unpop = self
                        .world
                        .get_chunk(p)
                        .map(|c| c.state != ChunkState::Populated)
                        .unwrap_or(true);
                    let dist = (p.x - center.x).abs().max((p.z - center.z).abs());
                    (unpop, dist)
                });
            }
        }
    }

    fn stream_chunks(&mut self) {
        let max_per_tick = if self.world.live_is_active() { 16 } else { 2 };
        let chunks_per_tick = self.config.world.chunks_per_tick.clamp(1, max_per_tick);

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
                        self.world.server_get_chunk_at(*pos);
                    }
                    while self.world.java_do_lighting() {}
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

                    // Evict from packet_cache once no player has this chunk queued anymore
                    let still_queued = self
                        .entities
                        .iter_players()
                        .any(|p| p.chunk_queue.contains(pos));
                    if !still_queued {
                        self.world.packet_cache.borrow_mut().remove(pos);
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
        let mut online_players: Vec<String> = self
            .entities
            .iter_players()
            .map(|p| p.username.clone())
            .collect();
        online_players.sort();
        let mut whitelisted_players: Vec<String> = self.whitelist.iter().cloned().collect();
        whitelisted_players.sort();

        let ctx = CommandContext {
            online_players,
            whitelisted_players,
            tps_records: self.tps_records.iter().copied().collect(),
            plugins: Vec::new(),
        };

        let result = self
            .commands
            .dispatch_with_context(&username, cmd_body, &ctx);

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

        for action in result.actions {
            self.execute_command_action(action);
        }

        for msg in &result.sender_messages {
            self.send(
                id,
                Clientbound::Chat(Chat {
                    message: msg.clone(),
                }),
            );
        }

        for (target_name, msg) in &result.target_messages {
            if let Some(target_conn) = self
                .entities
                .iter_players()
                .find(|p| p.username.eq_ignore_ascii_case(target_name))
                .map(|p| p.conn_id)
            {
                if target_conn != id {
                    self.send(
                        target_conn,
                        Clientbound::Chat(Chat {
                            message: msg.clone(),
                        }),
                    );
                }
            }
        }

        for msg in &result.broadcast_messages {
            self.broadcast_chat(msg);
        }
    }

    fn execute_command_action(&mut self, action: CommandAction) {
        match action {
            CommandAction::Give {
                target,
                item_id,
                amount,
                damage,
            } => {
                let target_info = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&target))
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
            CommandAction::TeleportToPlayer { victim, target } => {
                let target_pos = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&target))
                    .map(|p| (p.x, p.y, p.z, p.yaw, p.pitch));
                let victim_info = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&victim))
                    .map(|p| (p.conn_id, p.entity_id));
                if let (Some((tx, ty, tz, tyaw, tpitch)), Some((vconn, veid))) =
                    (target_pos, victim_info)
                {
                    self.dismount_player(veid);
                    if let Some(vp) = self.entities.get_player_mut(veid) {
                        let old_c = vp.chunk_pos();
                        vp.update_position(tx, ty, ty, tz, tyaw, tpitch, false);
                        vp.fall_distance = 0.0;
                        let new_c = vp.chunk_pos();
                        self.entities.update_player_chunk(veid, old_c, new_c);
                    }
                    self.send(
                        vconn,
                        Clientbound::PlayerLookMove(PlayerLookMove {
                            x: tx,
                            y: ty + 1.62,
                            stance: ty,
                            z: tz,
                            yaw: tyaw,
                            pitch: tpitch,
                            on_ground: false,
                        }),
                    );
                    self.handle_movement(
                        vconn,
                        Some((tx, ty, ty, tz)),
                        Some((tyaw, tpitch)),
                        false,
                    );
                }
            }
            CommandAction::TeleportToCoords { victim, x, y, z } => {
                let victim_info = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&victim))
                    .map(|p| (p.conn_id, p.entity_id, p.yaw, p.pitch));
                if let Some((vconn, veid, vyaw, vpitch)) = victim_info {
                    self.dismount_player(veid);
                    if let Some(vp) = self.entities.get_player_mut(veid) {
                        let old_c = vp.chunk_pos();
                        vp.update_position(x, y, y, z, vyaw, vpitch, false);
                        vp.fall_distance = 0.0;
                        let new_c = vp.chunk_pos();
                        self.entities.update_player_chunk(veid, old_c, new_c);
                    }
                    self.send(
                        vconn,
                        Clientbound::PlayerLookMove(PlayerLookMove {
                            x,
                            y: y + 1.62,
                            stance: y,
                            z,
                            yaw: vyaw,
                            pitch: vpitch,
                            on_ground: false,
                        }),
                    );
                    self.handle_movement(vconn, Some((x, y, y, z)), Some((vyaw, vpitch)), false);
                }
            }
            CommandAction::SetTime(t) => {
                self.world.live_time = t;
                self.world.level_data.borrow_mut().time = t;
                self.broadcast(Clientbound::UpdateTime(UpdateTime { time: t }));
            }
            CommandAction::AddTime(delta) => {
                let t = self.world.world_time().wrapping_add(delta);
                self.world.live_time = t;
                self.world.level_data.borrow_mut().time = t;
                self.broadcast(Clientbound::UpdateTime(UpdateTime { time: t }));
            }
            CommandAction::Kill(target) => {
                let target_eid = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&target))
                    .map(|p| p.entity_id);
                if let Some(eid) = target_eid {
                    self.damage_player_direct(eid, 1000, "died");
                }
            }
            CommandAction::Kick { target, reason } => {
                let target_conn = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&target))
                    .map(|p| p.conn_id);
                if let Some(conn_id) = target_conn {
                    self.kick(conn_id, &reason);
                }
            }
            CommandAction::Ban(target) => {
                self.banned_players.insert(target.to_lowercase());
                Self::save_list_file("banned-players.txt", &self.banned_players);
                let target_conn = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&target))
                    .map(|p| p.conn_id);
                if let Some(conn_id) = target_conn {
                    self.kick(conn_id, "You are banned from this server!");
                }
            }
            CommandAction::Pardon(target) => {
                self.banned_players.remove(&target.to_lowercase());
                Self::save_list_file("banned-players.txt", &self.banned_players);
            }
            CommandAction::BanIp(ip) => {
                let lower = ip.to_lowercase();
                self.banned_ips.insert(lower.clone());
                Self::save_list_file("banned-ips.txt", &self.banned_ips);
                let matching_conns: Vec<ConnId> = self
                    .connections
                    .iter()
                    .filter_map(|(&cid, c)| {
                        if c.address.ip().to_string().eq_ignore_ascii_case(&lower) {
                            Some(cid)
                        } else {
                            None
                        }
                    })
                    .collect();
                for cid in matching_conns {
                    self.kick(cid, "Your IP address is banned from this server!");
                }
            }
            CommandAction::PardonIp(ip) => {
                self.banned_ips.remove(&ip.to_lowercase());
                Self::save_list_file("banned-ips.txt", &self.banned_ips);
            }
            CommandAction::Op(target) => {
                self.ops.insert(target.to_lowercase());
                Self::save_list_file("ops.txt", &self.ops);
            }
            CommandAction::Deop(target) => {
                self.ops.remove(&target.to_lowercase());
                Self::save_list_file("ops.txt", &self.ops);
            }
            CommandAction::WhitelistOn => {
                self.whitelist_enabled = true;
            }
            CommandAction::WhitelistOff => {
                self.whitelist_enabled = false;
            }
            CommandAction::WhitelistAdd(target) => {
                self.whitelist.insert(target.to_lowercase());
                Self::save_list_file("white-list.txt", &self.whitelist);
            }
            CommandAction::WhitelistRemove(target) => {
                self.whitelist.remove(&target.to_lowercase());
                Self::save_list_file("white-list.txt", &self.whitelist);
            }
            CommandAction::WhitelistReload => {
                self.whitelist = Self::load_list_file("white-list.txt");
            }
            CommandAction::SaveAll => {
                for player in self.entities.iter_players() {
                    let _ = self.player_store.save_player(player);
                }
                let _ = self.world.save_all();
            }
            CommandAction::SaveOn => {
                self.config.world.save_enabled = true;
            }
            CommandAction::SaveOff => {
                self.config.world.save_enabled = false;
            }
            CommandAction::Stop => {
                for player in self.entities.iter_players() {
                    let _ = self.player_store.save_player(player);
                }
                let _ = self.world.save_all();
                let conn_ids: Vec<ConnId> = self.connections.keys().copied().collect();
                for cid in conn_ids {
                    self.kick(
                        cid,
                        "\u{00A7}cServer is shutting down, please rejoin later.",
                    );
                }
                self.running.store(false, Ordering::Relaxed);
            }
            CommandAction::Reload => {
                self.ops = Self::load_list_file("ops.txt");
                self.banned_players = Self::load_list_file("banned-players.txt");
                self.banned_ips = Self::load_list_file("banned-ips.txt");
                self.whitelist = Self::load_list_file("white-list.txt");
            }
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
            self.dismount_player(entity_id);
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
            self.dismount_player(entity_id);
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
                let _ = connection.sender.send(Outbound::Packet(packet.clone()));
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
                    .send(Outbound::Packet(Clientbound::Chat(Chat { message: line })));
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
                    .send(Outbound::Packet(Clientbound::Chat(Chat { message: line })));
            }
        }
    }

    fn send(&self, id: ConnId, packet: Clientbound) {
        if let Some(connection) = self.connections.get(&id) {
            let _ = connection.sender.send(Outbound::Packet(packet));
        }
    }

    #[allow(dead_code)]
    fn try_send(&self, id: ConnId, packet: Clientbound) -> bool {
        if let Some(connection) = self.connections.get(&id) {
            connection.sender.send(Outbound::Packet(packet)).is_ok()
        } else {
            false
        }
    }

    fn kick(&self, id: ConnId, reason: &str) {
        if let Some(connection) = self.connections.get(&id) {
            let _ = connection
                .sender
                .send(Outbound::Packet(Clientbound::KickDisconnect(
                    KickDisconnect {
                        reason: reason.to_string(),
                    },
                )));
            let _ = connection.sender.send(Outbound::Close);
        }
    }
}
