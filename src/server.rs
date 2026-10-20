use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::config::Config;
use crate::entity::mob::{
    chunk_light_at, evaluate_natural_spawns, find_path, light_curve, world_sky_subtracted, MobType,
};
use crate::entity::physics::Aabb;
use crate::entity::player::DiggingState;
use crate::entity::projectile::{Painting, ProjectileKind, PAINTING_TITLES};
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
    ArmAnimation, AttachEntity, Bed, BlockChange, Chat, Clientbound, CloseWindow, Collect,
    CraftProgressBar, EntityMetadata, EntityStatus, EntityVelocity, Explosion, Handshake,
    KeepAlive, KickDisconnect, LevelSound, Login, MapData, MultiBlockChange, OpenWindow,
    PlayNoteBlock, PlayerLookMove, PreChunk, Respawn, Serverbound, SetSlot, Sleep, SpawnPosition,
    Transaction, UpdateHealth, UpdateSign, UpdateTime, Weather, WindowItems,
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

/// Who issued a server command.
///
/// Java dispatches console commands with `MinecraftServer` as the
/// `ICommandListener` and player commands with the player's
/// `NetServerHandler`; both end up in the same command map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandSender {
    Player(ConnId),
    Console,
}

impl CommandSender {
    /// The connection of a player sender; the console has none.
    fn player_connection(self) -> Option<ConnId> {
        match self {
            CommandSender::Player(id) => Some(id),
            CommandSender::Console => None,
        }
    }
}

pub struct Server {
    config: Config,
    inbound: mpsc::Receiver<NetEvent>,
    connections: HashMap<ConnId, Connection>,
    running: Arc<AtomicBool>,
    entities: EntityStore,
    tracker: EntityTracker,
    world: World,
    /// Java `MinecraftServer.worlds` dimension -1 entry.
    nether: Option<World>,
    /// Which dimension currently occupies `self.world` (0 or -1). Packet
    /// handlers swap the two worlds so the huge body of gameplay code keeps
    /// operating on `self.world` unchanged.
    active_dimension: i32,
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
    /// Commands typed into the server console, queued by the standard input
    /// reader thread (`MinecraftServer.issueCommand`).
    console_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Sink the console writes its own output to (`MinecraftServer.sendMessage`).
    console_sink: Box<dyn Fn(&str) + Send>,
    /// Java `Entity.SHARED_RANDOM`: the random stream of the entities.
    ///
    /// The reference shares one `Random` between all the entities and seeds
    /// it with the seed of the world, so the behaviour of the mobs is
    /// reproducible.
    pub entity_random: JavaRandom,
}

impl Server {
    pub fn new(
        mut config: Config,
        inbound: mpsc::Receiver<NetEvent>,
        running: Arc<AtomicBool>,
    ) -> Self {
        let mut world = World::new(config.world.clone());
        world.allow_nether = config.server.allow_nether;
        // The random stream of the entities (`Entity.SHARED_RANDOM`).
        let entity_random = JavaRandom::new(config.world.seed);
        world.live.keep_spawn_in_memory = config.world.keep_spawn_in_memory;
        world.spawning = config.spawning.clone();
        world.worldgen = config.worldgen.clone();
        world.mechanics = config.mechanics.clone();
        world.fixes = config.fixes.clone();
        crate::world::generator::set_generate_sandstone(config.worldgen.generate_sandstone);
        crate::gameplay::crafting::set_crafting_mechanics(
            config.mechanics.glowstone_pre_1_6_6,
            config.mechanics.wool_recipe_pre_1_6_6,
            config.mechanics.old_slab_recipe,
            config.mechanics.allow_ladder_gap,
        );
        if config.world.save_enabled {
            world.live_enable();
            if world.is_fresh_world() {
                let custom_spawn = !config.world.auto_find_spawn
                    || config.world.override_saved_spawn
                    || config.world.spawn_x != 0
                    || config.world.spawn_y != 64
                    || config.world.spawn_z != 0;
                if custom_spawn {
                    world.set_spawn_xyz(
                        config.world.spawn_x,
                        config.world.spawn_y,
                        config.world.spawn_z,
                    );
                } else {
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
                }
            } else {
                let (sx, sy, sz) = world.spawn_point();
                config.world.spawn_x = sx;
                config.world.spawn_y = sy;
                config.world.spawn_z = sz;
            }
            if config.world.keep_spawn_in_memory {
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
        }
        let player_store = crate::world::storage::PlayerStore::new(world.world_dir());
        let view_distance = config.world.view_distance;

        // Java `MinecraftServer`: the Nether world is created up front when
        // `allow-nether` is enabled. Its seed must match the (possibly
        // level.dat-loaded) overworld seed.
        let nether = if config.server.allow_nether {
            let mut nconfig = config.world.clone();
            nconfig.seed = world.seed();
            let mut nether = World::new_nether(nconfig);
            nether.allow_nether = true;
            nether.live.keep_spawn_in_memory = config.world.keep_spawn_in_memory;
            nether.spawning = config.spawning.clone();
            nether.worldgen = config.worldgen.clone();
            nether.mechanics = config.mechanics.clone();
            nether.fixes = config.fixes.clone();
            // Beta shares the overworld's world time with DIM-1.
            let world_time = world.world_time();
            nether.live_time = world_time;
            nether.level_data.borrow_mut().time = world_time;
            if config.world.save_enabled {
                nether.live_enable();
            }
            Some(nether)
        } else {
            None
        };
        let whitelist_enabled = config.server.white_list;
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
            nether,
            active_dimension: 0,
            player_store,
            commands: CommandMap::new(),
            plugin_manager: PluginManager::new(),
            current_tick: 0,
            ops,
            banned_players,
            banned_ips,
            whitelist,
            whitelist_enabled,
            tps_records: VecDeque::with_capacity(900),
            last_tps_sample: Instant::now(),
            ticks_since_sample: 0,
            console_rx: None,
            console_sink: Box::new(|message: &str| log::console(message)),
            entity_random,
        }
    }

    /// Attaches the standard input console to the server.
    ///
    /// The receiver is drained once per tick (`MinecraftServer.b()`), so the
    /// commands run on the simulation thread and cannot race with the world
    /// simulation.
    pub fn attach_console(&mut self, rx: std::sync::mpsc::Receiver<String>) {
        self.console_rx = Some(rx);
    }

    /// Replaces the sink the console prints its answers to.
    pub fn set_console_sink(&mut self, sink: Box<dyn Fn(&str) + Send>) {
        self.console_sink = sink;
    }

    /// Whether the console terminal is attached.
    pub fn console_attached(&self) -> bool {
        self.console_rx.is_some()
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
            if self.nether.is_some() {
                self.with_dimension(-1, |s| {
                    let _ = s.world.save_all();
                });
            }
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
        if self.config.server.time_speed_multiplier != 1 {
            let extra = self.config.server.time_speed_multiplier - 1;
            self.world.live_time = self.world.live_time.wrapping_add(extra);
            self.world.level_data.borrow_mut().time = self.world.live_time;
        }

        // Tick the Nether's world-level machinery (scheduled block/fluid ticks,
        // lighting, furnaces, random blocks) with its world active.
        let overworld_time = self.world.world_time();
        if self.nether.is_some() {
            self.with_dimension(-1, |s| {
                if s.world.live_is_active() {
                    s.world.live_tick();
                } else {
                    s.world.increment_time();
                }
                // Keep the Nether clock in lockstep with the overworld.
                s.world.live_time = overworld_time;
                s.world.level_data.borrow_mut().time = overworld_time;
                s.world.tick_furnaces();
                let nether_players: Vec<(f64, f64)> = s
                    .entities
                    .iter_players()
                    .filter(|p| p.dimension == -1 && !p.dead)
                    .map(|p| (p.x, p.z))
                    .collect();
                if !nether_players.is_empty() {
                    s.world.tick_random_blocks(&nether_players);
                }
                s.process_world_events();
                s.world.flush_pending_piston_syncs();
                while s.world.java_do_lighting() {}
            });
        }

        self.drain_inbound();

        // Incremental auto-save (`ChunkProviderServer.saveChunks(false, null)` — up to 24 dirty chunks)
        let autosave_ticks = self.config.server.autosave_interval_ticks;
        if ticks > 0 && self.config.world.save_enabled && autosave_ticks > 0 {
            if ticks % 100 == 0 {
                let _ = self.world.save_chunks_incremental(24);
                if self.nether.is_some() {
                    self.with_dimension(-1, |s| {
                        let _ = s.world.save_chunks_incremental(24);
                    });
                }
            }
            let player_save_every = autosave_ticks.min(1200);
            if self.config.server.save_playerdata && ticks % player_save_every == 0 {
                for player in self.entities.iter_players() {
                    let _ = self.player_store.save_player(player);
                }
            }
        }

        // Weather cycle (`World.i()` + `WorldServer.java:182-195` Packet70Bed rain toggle)
        if self.world.live_is_active() {
            if let Some(new_raining) = self.world.tick_weather() {
                self.broadcast(Clientbound::Bed(Bed {
                    reason: if new_raining { 1 } else { 2 },
                }));
            }
        }

        // Tick furnaces and sync open furnace windows
        self.world.tick_furnaces();
        self.sync_open_containers();
        if self.nether.is_some() {
            self.with_dimension(-1, |s| s.sync_open_containers());
        }
        self.tick_held_maps(ticks);
        if self.nether.is_some() {
            self.with_dimension(-1, |s| s.tick_held_maps(ticks));
        }

        // Random block ticks (grass, crops, saplings, reeds, leaves, ice/snow) when players online
        let player_xz: Vec<(f64, f64)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead && p.dimension == 0)
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
        if self.nether.is_some() {
            self.with_dimension(-1, |s| s.tick_player_environmental_damage());
        }
        self.tick_deferred_digging();
        if self.nether.is_some() {
            self.with_dimension(-1, |s| s.tick_deferred_digging());
        }
        self.tick_player_portals();
        self.tick_pressure_plates_and_beds();
        if self.nether.is_some() {
            self.with_dimension(-1, |s| s.tick_pressure_plates_and_beds());
        }

        // Natural mob spawning every 20 ticks (`SpawnerCreature.spawnEntities`)
        let spawn_interval = self.config.spawning.spawn_interval_ticks.max(1);
        if ticks % spawn_interval == 0 {
            if self.world.live_is_active() {
                self.tick_mob_spawner();
            }
            if self.nether.is_some()
                && self
                    .nether
                    .as_ref()
                    .is_some_and(|nether| nether.live_is_active())
            {
                self.with_dimension(-1, |s| s.tick_mob_spawner());
            }
        }
        self.tick_mob_spawners();

        // Tick boats, mobs, projectiles, primed TNTs, minecarts, paintings, and dropped items
        self.tick_boats();
        self.tick_mobs();
        self.tick_projectiles();
        self.tick_primed_tnts();
        self.tick_minecarts();
        if ticks % 100 == 0 {
            self.tick_paintings();
        }
        self.tick_dropped_items();
        self.process_world_events();
        self.world.flush_pending_piston_syncs();

        // Nether entity simulation. `active_dimension` makes each tick routine
        // only consider entities tagged with this dimension.
        if self.nether.is_some() {
            self.with_dimension(-1, |s| {
                s.tick_mob_spawners();
                s.tick_boats();
                s.tick_mobs();
                s.tick_projectiles();
                s.tick_primed_tnts();
                s.tick_minecarts();
                if ticks % 100 == 0 {
                    s.tick_paintings();
                }
                s.tick_dropped_items();
                s.process_world_events();
                s.world.flush_pending_piston_syncs();
                while s.world.java_do_lighting() {}
            });
        }

        while self.world.java_do_lighting() {}
        if ticks % 200 == 0 {
            self.world.compact_dirty_light_chunks();
        }

        self.flush_dirty_chunks();
        if self.nether.is_some() {
            self.with_dimension(-1, |s| s.flush_dirty_chunks());
        }
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

        // Console commands are executed at the end of the tick, like the
        // vanilla `MinecraftServer.b()`.
        self.process_console_commands();
    }

    fn run_tracker_tick(&mut self) {
        let dimensions: Vec<i32> = if self.nether.is_some() {
            vec![0, -1]
        } else {
            vec![0]
        };
        for dimension in dimensions {
            self.with_dimension(dimension, |s| s.run_tracker_tick_inner());
        }
    }

    fn run_tracker_tick_inner(&mut self) {
        let active_dimension = self.active_dimension;
        let mut players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        let mut items: Vec<_> = self
            .entities
            .iter_dropped_items()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();
        let mut boats: Vec<_> = self
            .entities
            .iter_boats()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();
        let mut mobs: Vec<_> = self
            .entities
            .iter_mobs()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();
        let mut projectiles: Vec<_> = self
            .entities
            .iter_projectiles()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();
        let mut tnts: Vec<_> = self
            .entities
            .iter_primed_tnts()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();
        let mut minecarts: Vec<_> = self
            .entities
            .iter_minecarts()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();
        let paintings: Vec<_> = self
            .entities
            .iter_paintings()
            .filter(|e| self.entities.dimension_of(e.entity_id) == active_dimension)
            .cloned()
            .collect();

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
            self.send_tracked(outbound.recipient_id, outbound.packet);
        }
    }

    fn send_tracked(&mut self, recipient_id: i32, packet: Clientbound) {
        let active_dimension = self.active_dimension;
        if let Some(target) = self.entities.get_player(recipient_id) {
            if target.dimension == active_dimension {
                self.send(target.conn_id, packet);
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
            NetEvent::LegacyPing { id } => self.handle_legacy_ping(id),
            NetEvent::Disconnected { id } => {
                self.remove_and_handle_disconnect(id);
            }
        }
    }

    /// Answer the pre-1.7 `0xFE` server-list ping. The wire format of
    /// `Packet0xFF` is exactly `motd§online§max`; Beta 1.7.3 expects the
    /// unversioned three-field response.
    fn handle_legacy_ping(&mut self, id: ConnId) {
        let Some(connection) = self.connections.get(&id) else {
            return;
        };
        if connection.state != ConnectionState::Handshaking {
            return;
        }
        let max_players = self.config.server.max_players;
        let online = self.entities.player_count().min(max_players as usize);
        let mut response = format!(
            "{}\u{00A7}{}\u{00A7}{}",
            self.config.server.motd, online, max_players
        );
        // `Packet0xFF` caps strings at 100 UTF-16 code units.
        while response.encode_utf16().count() > 100 {
            response.pop();
        }
        let _ = connection
            .sender
            .send(Outbound::Packet(Clientbound::KickDisconnect(
                KickDisconnect { reason: response },
            )));
        let _ = connection.sender.send(Outbound::Close);
    }

    /// Removes the player of a connection: the open container, the vehicle,
    /// the entity, the world anchor, the player data and the tracker entry.
    ///
    /// The leave message is broadcast as well, so the caller must not send it
    /// a second time.
    fn cleanup_player(&mut self, id: ConnId) {
        let dimension = self
            .entities
            .get_by_conn(id)
            .map(|p| p.dimension)
            .unwrap_or(0);
        self.with_dimension(dimension, |s| {
            s.close_player_container_and_drop_transient(id, false);
            if let Some(p) = s.entities.get_by_conn(id) {
                let eid = p.entity_id;
                s.dismount_player(eid);
            }
        });
        let Some(player) = self.entities.remove_by_conn(id) else {
            return;
        };
        if self.nether.is_some() {
            self.with_dimension(dimension, |s| {
                if s.world.live_is_active() {
                    s.world.live_player_quit(id);
                }
            });
        } else if self.world.live_is_active() {
            self.world.live_player_quit(id);
        }
        if self.config.world.save_enabled && self.config.server.save_playerdata {
            let _ = self.player_store.save_player(&player);
        }

        let untrack_packets = self.tracker.untrack_player(player.entity_id);
        for outbound in untrack_packets {
            if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                if target.dimension == dimension {
                    self.send(target.conn_id, outbound.packet);
                }
            }
        }

        if !self.config.messages.remove_join_leave_debug {
            log::info(&format!(
                "{} lost connection: disconnect.quitting",
                player.username
            ));
        }
        let msg = self
            .config
            .messages
            .leave
            .replacen("%player%", &player.username, 1);
        self.broadcast_chat(&msg);
    }

    fn remove_and_handle_disconnect(&mut self, id: ConnId) {
        if let Some(conn) = self.connections.remove(&id) {
            self.cleanup_player(id);
            if !self.config.messages.remove_join_leave_debug {
                log::info(&format!(
                    "connection {id} ({}) disconnected",
                    conn.username.as_deref().unwrap_or("unknown")
                ));
            }
        }
    }

    /// Whether the session of a player is still alive.
    ///
    /// A kicked player keeps its connection until the socket is closed and
    /// the disconnect event is processed, and a client that vanishes leaves a
    /// connection whose writer is already gone. Such a player is gone: it
    /// must not block the name for the next login.
    fn player_session_is_alive(&self, conn_id: ConnId, entity_id: i32) -> bool {
        match self.connections.get(&conn_id) {
            Some(conn) => !conn.sender.is_closed() && conn.entity_id == entity_id,
            None => false,
        }
    }

    /// Removes the players whose session is gone.
    ///
    /// The disconnect event is processed asynchronously, so a kicked player can
    /// still be in the list for a moment. Such a ghost would block the name
    /// for the next login and count towards the player limit.
    fn purge_dead_players(&mut self) {
        let dead: Vec<ConnId> = self
            .entities
            .iter_players()
            .filter(|player| !self.player_session_is_alive(player.conn_id, player.entity_id))
            .map(|player| player.conn_id)
            .collect();
        for conn_id in dead {
            log::info(&format!(
                "removing the player of the dead connection {conn_id}"
            ));
            self.cleanup_player(conn_id);
        }
    }

    /// Run `f` with the requested dimension's `World` occupying `self.world`.
    /// The server is single-threaded, so a swap is indistinguishable from
    /// having per-dimension world fields for the duration of a packet handler.
    fn with_dimension<R>(&mut self, dimension: i32, f: impl FnOnce(&mut Self) -> R) -> R {
        if self.nether.is_none()
            || dimension == self.active_dimension
            || !matches!(dimension, 0 | -1)
        {
            return f(self);
        }
        let previous_spawn_dimension = self.entities.spawn_dimension;
        let previous_active_dimension = self.active_dimension;
        self.entities.spawn_dimension = dimension;
        std::mem::swap(&mut self.world, self.nether.as_mut().unwrap());
        self.active_dimension = dimension;
        let result = f(self);
        std::mem::swap(&mut self.world, self.nether.as_mut().unwrap());
        // Restore the *previous* dimension: `with_dimension` nests (the Nether
        // tick calls `with_dimension(0, ...)` while the Nether is active).
        self.active_dimension = previous_active_dimension;
        self.entities.spawn_dimension = previous_spawn_dimension;
        result
    }

    fn handle_packet(&mut self, id: ConnId, packet: Serverbound) {
        let dimension = self
            .entities
            .get_by_conn(id)
            .map(|p| p.dimension)
            .unwrap_or(0);
        self.with_dimension(dimension, |s| s.handle_packet_inner(id, packet));
    }

    fn handle_packet_inner(&mut self, id: ConnId, packet: Serverbound) {
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
        let mut stepped_block = None;
        let mut kick_flight = false;
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

                if x.is_nan()
                    || y.is_nan()
                    || z.is_nan()
                    || stance.is_nan()
                    || x.is_infinite()
                    || y.is_infinite()
                    || z.is_infinite()
                    || stance.is_infinite()
                    || yaw.is_infinite()
                    || pitch.is_infinite()
                {
                    self.kick(id, "Nope!");
                    return;
                }
                if self.config.fixes.head_rotation_patch_enabled
                    && rot.is_some()
                    && !(-90.0..=90.0).contains(&pitch)
                {
                    self.kick(id, "Illegal stance");
                    return;
                }
                if x.abs() > 3.2e7 || z.abs() > 3.2e7 {
                    self.kick(id, "Illegal position");
                    return;
                }

                // Poseidon / NetServerHandler.java:385-394 — reject stale pre-teleport / >max_distance single-packet jump
                if self.world.live_is_active()
                    && self.config.fixes.speed_hack_check_enabled
                    && pos.is_some()
                {
                    let dx_m = x - player.x;
                    let dz_m = z - player.z;
                    let mot_sq = player.mot_x * player.mot_x + player.mot_z * player.mot_z;
                    if (dx_m * dx_m + dz_m * dz_m) - mot_sq
                        > self.config.fixes.speed_hack_max_distance
                        && self.config.fixes.speed_hack_teleport
                    {
                        resync_teleport =
                            Some((player.x, player.y, player.z, player.yaw, player.pitch));
                    }
                }

                if resync_teleport.is_none() {
                    let bx = x.floor() as i32;
                    let by = y.floor() as i32;
                    let bz = z.floor() as i32;
                    let at_block = self.world.get_block(bx, by, bz);
                    let in_climb_or_fluid = blocks::is_water(at_block)
                        || blocks::is_lava(at_block)
                        || at_block == blocks::WEB
                        || at_block == blocks::LADDER
                        || (self.config.mechanics.allow_ladder_gap
                            && self.world.get_block(bx, by + 1, bz) == blocks::LADDER);
                    if in_climb_or_fluid {
                        player.fall_distance = 0.0;
                    }
                    if player.vehicle_id.is_none() {
                        if !on_ground {
                            if y < prev_y {
                                player.fall_distance += (prev_y - y) as f32;
                            }
                            if !in_climb_or_fluid && y >= prev_y - 0.03125 {
                                player.air_hover_ticks += 1;
                                if player.air_hover_ticks > 80 {
                                    let is_op = self.ops.contains(&player.username.to_lowercase());
                                    if !self.config.server.allow_flight
                                        && !(self.config.server.exempt_staff_from_flight_kick
                                            && is_op)
                                    {
                                        kick_flight = true;
                                    }
                                }
                            } else {
                                player.air_hover_ticks = 0;
                            }
                        } else {
                            player.air_hover_ticks = 0;
                            if player.fall_distance > 3.0 {
                                fall_damage = ((player.fall_distance - 3.0).ceil() as i16).max(0);
                            }
                            player.fall_distance = 0.0;
                        }
                        if on_ground
                            && !player.sneaking
                            && pos.is_some()
                            && ((x - player.x).abs() > 0.01
                                || (z - player.z).abs() > 0.01
                                || fall_damage > 0)
                        {
                            stepped_block = Some((bx, (y - 0.2).floor() as i32, bz));
                        }
                    } else {
                        player.air_hover_ticks = 0;
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

        if kick_flight {
            self.kick(id, "Flying is not enabled on this server");
            return;
        }

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

        if let Some((sbx, sby, sbz)) = stepped_block {
            if (0..128).contains(&sby) {
                let bid = self.world.get_block(sbx, sby, sbz);
                if bid == blocks::FARMLAND {
                    if !self.config.mechanics.trample_farmland_above_fence
                        || sby == 0
                        || self.world.get_block(sbx, sby - 1, sbz) != blocks::FENCE
                    {
                        self.world.trample_farmland(sbx, sby, sbz);
                    }
                } else if bid == blocks::REDSTONE_ORE {
                    self.world
                        .java_set_type_id(sbx, sby, sbz, blocks::LIT_REDSTONE_ORE);
                }
            }
        }

        self.entities
            .update_player_chunk(entity_id, old_chunk, new_chunk);

        if fall_damage > 0 {
            self.damage_player_direct(entity_id, fall_damage, "hit the ground too hard");
        }

        self.update_player_view_chunks(id);
    }

    fn update_player_view_chunks(&mut self, id: ConnId) {
        let dimension = self
            .entities
            .get_by_conn(id)
            .map(|p| p.dimension)
            .unwrap_or(0);
        self.with_dimension(dimension, |s| s.update_player_view_chunks_inner(id));
    }

    fn update_player_view_chunks_inner(&mut self, id: ConnId) {
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
            let active_dimension = self.active_dimension;
            let player_centers: Vec<(i32, i32)> = self
                .entities
                .iter_players()
                .filter(|p| p.dimension == active_dimension)
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
                        self.send_tracked(outbound.recipient_id, outbound.packet);
                    }
                    if self.config.mechanics.boats_drop_boat_not_wood {
                        self.spawn_dropped_item_random(
                            bx,
                            by,
                            bz,
                            ItemStack::new(items::BOAT, 1, 0),
                            10,
                        );
                    } else {
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
            }
            return;
        }

        let crit_bonus = self
            .entities
            .get_by_conn(id)
            .is_some_and(|p| p.mot_y < 0.0 || (!p.on_ground && p.fall_distance > 0.0));

        // Check if target is a Mob (`EntityLiving` / `EntityAnimal` / `EntityMonster`)
        if self.entities.get_mob(target_id).is_some() {
            if !use_entity.attack
                || (held_item.is_some_and(|s| s.id == items::SADDLE)
                    && self
                        .entities
                        .get_mob(target_id)
                        .is_some_and(|m| m.mob_type == MobType::Pig && !m.pig_saddled))
            {
                self.handle_interact_mob(id, attacker_id, &attacker_name, target_id);
            } else {
                let raw_damage =
                    held_item.map(|s| s.attack_damage()).unwrap_or(1) + i16::from(crit_bonus);
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
                        self.send_tracked(outbound.recipient_id, outbound.packet);
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
            if !self.config.server.pvp {
                return;
            }
            let keep_inv = self.config.mechanics.keep_inventory_on_death;
            let mut woke_target = false;
            let armor_sync;
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

                    let raw_damage =
                        held_item.map(|s| s.attack_damage()).unwrap_or(1) + i16::from(crit_bonus);
                    let (dealt, died) = target.take_damage(raw_damage);
                    if dealt <= 0 && raw_damage > 0 {
                        return;
                    }
                    if target.sleeping {
                        target.sleeping = false;
                        target.sleep_ticks = 0;
                        target.bed_pos = None;
                        woke_target = true;
                    }
                    armor_sync = Some((
                        target.conn_id,
                        [
                            InventoryPlayer::to_slot_item(target.inventory.armor[3]),
                            InventoryPlayer::to_slot_item(target.inventory.armor[2]),
                            InventoryPlayer::to_slot_item(target.inventory.armor[1]),
                            InventoryPlayer::to_slot_item(target.inventory.armor[0]),
                        ],
                    ));

                    target.apply_knockback(attacker_x, attacker_z);
                    let mot_x = (target.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16;
                    let mot_y = (target.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16;
                    let mot_z = (target.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16;
                    let drops = if died && !keep_inv {
                        let tx = target.x;
                        let ty = target.y;
                        let tz = target.z;
                        let mut list: Vec<(f64, f64, f64, ItemStack)> = target
                            .inventory
                            .drop_all_on_death()
                            .into_iter()
                            .map(|it| (tx, ty, tz, it))
                            .collect();
                        if let Some(OpenContainer::Workbench { craft_grid, .. }) =
                            target.open_container.take()
                        {
                            for it in craft_grid.into_iter().flatten() {
                                list.push((tx, ty, tz, it));
                            }
                        }
                        list
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

            if woke_target {
                self.broadcast(Clientbound::ArmAnimation(ArmAnimation {
                    entity_id: target_id,
                    animate: 3,
                }));
            }
            if let Some((cid, slots)) = armor_sync {
                for (i, item) in slots.into_iter().enumerate() {
                    self.send(
                        cid,
                        Clientbound::SetSlot(SetSlot {
                            window_id: 0,
                            slot: 5 + i as i16,
                            item,
                        }),
                    );
                }
            }
            // Alert tamed wolves owned by target (`EntityHuman.java:455-495`)
            for w in self.entities.iter_mobs_mut() {
                if w.mob_type == MobType::Wolf
                    && w.wolf_tamed
                    && w.wolf_owner.eq_ignore_ascii_case(&target_name)
                {
                    w.wolf_sitting = false;
                    w.target_id = Some(attacker_id);
                }
            }

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
            let mut dismount_pos = None;
            if let Some(boat) = self.entities.get_boat_mut(vid) {
                if boat.passenger_id == Some(player_id) {
                    boat.passenger_id = None;
                    dismount_pos = Some((boat.x, boat.y + 0.6, boat.z));
                }
            }
            if let Some(mob) = self.entities.get_mob_mut(vid) {
                if mob.passenger_id == Some(player_id) {
                    mob.passenger_id = None;
                    dismount_pos = Some((mob.x, mob.y + mob.size().1 as f64, mob.z));
                }
            }
            if let Some(cart) = self.entities.get_minecart_mut(vid) {
                if cart.passenger_id == Some(player_id) {
                    cart.passenger_id = None;
                    // Entity.java:1195 — `this.vehicle.boundingBox.b + this.vehicle.width` = `(cart.y - 0.35) + 0.70`
                    dismount_pos = Some((cart.x, cart.y + 0.35, cart.z));
                }
            }
            if let Some((dx, dy, dz)) = dismount_pos {
                if let Some(player) = self.entities.get_player_mut(player_id) {
                    player.x = dx;
                    player.y = dy;
                    player.stance = dy;
                    player.z = dz;
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
        let active_dimension = self.active_dimension;
        let mut ready_breaks = Vec::new();
        for player in self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
        {
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
        // BlockTNT.postBreak (`BlockTNT.java:41-60`): ignite when metadata bit 0 is set, holding lighter, or `!tnt_require_lighter`
        if old_block == blocks::TNT {
            if !self.config.mechanics.tnt_require_lighter
                || (old_meta & 1) != 0
                || held_item.is_some_and(|s| s.id == items::FLINT_AND_STEEL)
            {
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
            let mut drops = blocks::get_block_drops(old_block, harvest_meta, &mut rng, held_item);
            if old_block == blocks::LAPIS_ORE && self.config.mechanics.drop_lapis_as_b1_2 {
                drops = vec![ItemStack::new(items::INK_SACK, 1, 4)];
            } else if old_block == blocks::GLOWSTONE && self.config.mechanics.glowstone_pre_1_6_6 {
                drops = vec![ItemStack::new(items::GLOWSTONE_DUST, 1, 0)];
            } else if old_block == blocks::LEAVES
                && !self.config.mechanics.drop_saplings_of_leaf_type
            {
                for st in &mut drops {
                    st.damage = 0;
                }
            }
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
                if self.world.is_hell && self.config.mechanics.nether_bed_explode {
                    // Java `BlockBed.interact` (`BlockBed.java:35-53`): the bed
                    // is removed and a power-5 explosion is created at the
                    // (possibly second) head block.
                    let directions = [(0, 1), (-1, 0), (0, -1), (1, 0)];
                    let mut bx = x;
                    let mut bz = z;
                    let mut data = self.world.get_data(x, y, z);
                    if (data & 8) == 0 {
                        let (dx, dz) = directions[(data & 3) as usize];
                        bx += dx;
                        bz += dz;
                        if self.world.get_block(bx, y, bz) != blocks::BED {
                            return true;
                        }
                        data = self.world.get_data(bx, y, bz);
                    }
                    self.world.java_set_type_id(bx, y, bz, blocks::AIR);
                    let (dx, dz) = directions[(data & 3) as usize];
                    bx += dx;
                    bz += dz;
                    if self.world.get_block(bx, y, bz) == blocks::BED {
                        self.world.java_set_type_id(bx, y, bz, blocks::AIR);
                    }
                    self.create_explosion(
                        bx as f64 + 0.5,
                        y as f64 + 0.5,
                        bz as f64 + 0.5,
                        5.0,
                        true,
                    );
                    return true;
                }
                let meta = self.world.get_data(x, y, z);
                // Java `EntityHuman.a(i, j, k)`: `worldProvider.c` is true in
                // the Nether, so sleeping there is never possible.
                if self.world.is_hell {
                    return true;
                }
                let (hx, hz) = if (meta & 8) == 0 {
                    let dir = (meta & 3) as usize;
                    let (dx, dz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][dir];
                    if self.world.get_block(x + dx, y, z + dz) != blocks::BED {
                        return true;
                    }
                    (x + dx, z + dz)
                } else {
                    (x, z)
                };
                let occupied = self
                    .entities
                    .iter_players()
                    .any(|p| !p.dead && p.sleeping && p.bed_pos == Some((hx, y, hz)));
                if occupied {
                    self.send(
                        id,
                        Clientbound::Chat(Chat {
                            message: "This bed is occupied".to_string(),
                        }),
                    );
                    return true;
                }

                let mut sleep_pkt = None;
                let mut only_at_night = false;
                let set_spawn = self.config.mechanics.beds_set_spawnpoint;
                let time_of_day = self.world.world_time().rem_euclid(24000);
                let is_night = (12541..=23458).contains(&time_of_day) || self.world.is_thundering();
                if let Some(player) = self.entities.get_by_conn_mut(id) {
                    if (player.x - hx as f64).abs() > 3.5
                        || (player.y - y as f64).abs() > 2.5
                        || (player.z - hz as f64).abs() > 3.5
                    {
                        return true;
                    }
                    if set_spawn {
                        player.spawn_bed = Some((hx, y, hz));
                    }
                    if is_night {
                        player.sleeping = true;
                        player.sleep_ticks = 0;
                        player.bed_pos = Some((hx, y, hz));
                        sleep_pkt = Some(Sleep {
                            entity_id: player.entity_id,
                            in_bed_code: 0,
                            bed_x: hx,
                            bed_y: y as u8,
                            bed_z: hz,
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
                    if clicked_block == blocks::GRASS
                        && self.config.mechanics.tile_grass_drop_seeds
                        && self.world.tick_random.next_i32_bound(8) == 0
                    {
                        self.spawn_dropped_item_random(
                            cx as f64,
                            (cy + 1) as f64,
                            cz as f64,
                            ItemStack::new(items::SEEDS, 1, 0),
                            10,
                        );
                    }
                    self.world.java_set_type_id(cx, cy, cz, blocks::FARMLAND);
                    self.send_block_change_to_observers(cx, cy, cz);
                    durability_cost = 1;
                }
            }
            // Seeds: plant crops (59) on top of farmland (60)
            items::SEEDS => {
                let above_id = self.world.get_block(cx, cy + 1, cz);
                if face == 1
                    && clicked_block == blocks::FARMLAND
                    && (above_id == blocks::AIR || self.config.mechanics.seeds_replace_blocks)
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
                    let dir = (((yaw + 180.0) * 4.0 / 360.0 - 0.5).floor() as i32 & 3) as usize;
                    let (dx, dz) = [(0, -1), (1, 0), (0, 1), (-1, 0)][dir];
                    let left_solid = i32::from(self.world.is_solid_normal_at(tx - dx, ty, tz - dz))
                        + i32::from(self.world.is_solid_normal_at(tx - dx, ty + 1, tz - dz));
                    let right_solid =
                        i32::from(self.world.is_solid_normal_at(tx + dx, ty, tz + dz))
                            + i32::from(self.world.is_solid_normal_at(tx + dx, ty + 1, tz + dz));
                    let left_door = self.world.get_block(tx - dx, ty, tz - dz) == door_id
                        || self.world.get_block(tx - dx, ty + 1, tz - dz) == door_id;
                    let right_door = self.world.get_block(tx + dx, ty, tz + dz) == door_id
                        || self.world.get_block(tx + dx, ty + 1, tz + dz) == door_id;
                    let flip = (left_door && !right_door) || (right_solid > left_solid);
                    let meta = if flip {
                        (((dir as i32 - 1) & 3) + 4) as u8
                    } else {
                        dir as u8
                    };
                    self.world
                        .java_set_type_id_and_data(tx, ty, tz, door_id, meta);
                    self.world
                        .java_set_type_id_and_data(tx, ty + 1, tz, door_id, meta + 8);
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
                        2 => 0,
                        4 => 1,
                        3 => 2,
                        _ => 3,
                    };
                    let mut valid_titles = Vec::new();
                    for &title in PAINTING_TITLES {
                        if self.painting_can_survive(cx, cy, cz, dir, title, None) {
                            valid_titles.push(title);
                        }
                    }
                    if !valid_titles.is_empty() {
                        let idx = (self.current_tick as usize + (cx.unsigned_abs() as usize))
                            % valid_titles.len();
                        let title = valid_titles[idx].to_string();
                        self.spawn_painting_at(title, cx, cy, cz, dir);
                        consumed = true;
                    }
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
                    if stack.id == items::WATER_BUCKET && self.world.is_hell {
                        // ItemBucket: water evaporates in the Nether (fizz +
                        // smoke), no fluid is placed, the bucket is returned.
                        self.broadcast(Clientbound::LevelSound(LevelSound {
                            effect_id: 1004,
                            x: tx,
                            y: ty as u8,
                            z: tz,
                            data: 0,
                        }));
                        replacement_item = Some(ItemStack::new(items::BUCKET, 1, 0));
                    } else {
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
                } else if !(!self.config.mechanics.allow_blocks_at_y_127
                    && ty == 127
                    && blocks::material(block_id).is_buildable())
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
        } else if stack.is_food()
            || matches!(
                stack.id,
                items::BOW
                    | items::SNOW_BALL
                    | items::EGG
                    | items::FISHING_ROD
                    | items::MILK_BUCKET
            )
        {
            // ItemInWorldManager.java:241-243 — if block.interact and placeItem both returned false, fall through to useItem
            self.use_item_right_click_air(id, stack.id);
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
                    if stack.id == items::WATER_BUCKET && self.world.is_hell {
                        self.broadcast(Clientbound::LevelSound(LevelSound {
                            effect_id: 1004,
                            x: tx,
                            y: ty as u8,
                            z: tz,
                            data: 0,
                        }));
                        new_stack = Some(Some(ItemStack::new(items::BUCKET, 1, 0)));
                    } else {
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
            // ItemFishingRod.java & EntityFish.java:337-410 — cast or reel in FishingHook
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
                .map(|pr| {
                    (
                        pr.entity_id,
                        pr.x,
                        pr.y,
                        pr.z,
                        pr.in_water_or_catching(),
                        pr.in_ground,
                        pr.hooked_entity,
                    )
                });
            if let Some((hook_id, hx, hy, hz, catching, in_ground, hooked_entity)) = existing_hook {
                if self.entities.remove_projectile(hook_id).is_some() {
                    for out in self.tracker.untrack_projectile(hook_id) {
                        self.send_tracked(out.recipient_id, out.packet);
                    }
                }
                let mut rod_damage = 0i16;
                if let Some(target_eid) = hooked_entity {
                    let dx = px - hx;
                    let dy = py - hy;
                    let dz = pz - hz;
                    let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(0.1);
                    let vx = dx * 0.1;
                    let vy = dy * 0.1 + dist.sqrt() * 0.08;
                    let vz = dz * 0.1;
                    if let Some(m) = self.entities.get_mob_mut(target_eid) {
                        m.mot_x += vx;
                        m.mot_y += vy;
                        m.mot_z += vz;
                        m.velocity_changed = true;
                    } else if let Some(tp) = self.entities.get_player_mut(target_eid) {
                        tp.mot_x += vx;
                        tp.mot_y += vy;
                        tp.mot_z += vz;
                        tp.velocity_changed = true;
                        let tconn = tp.conn_id;
                        let vel = Clientbound::EntityVelocity(EntityVelocity {
                            entity_id: target_eid,
                            mot_x: (tp.mot_x.clamp(-3.9, 3.9) * 8000.0) as i16,
                            mot_y: (tp.mot_y.clamp(-3.9, 3.9) * 8000.0) as i16,
                            mot_z: (tp.mot_z.clamp(-3.9, 3.9) * 8000.0) as i16,
                        });
                        self.send(tconn, vel.clone());
                        self.broadcast(vel);
                    }
                    rod_damage = 3;
                } else if catching {
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
                    let active_dimension = self.active_dimension;
                    let players: Vec<_> = self
                        .entities
                        .iter_players()
                        .filter(|p| p.dimension == active_dimension)
                        .cloned()
                        .collect();
                    if let Some(dropped) = self.entities.get_dropped_item(item_id) {
                        for out in self.tracker.track_item_and_scan(dropped, &players) {
                            self.send_tracked(out.recipient_id, out.packet);
                        }
                    }
                    rod_damage = 1;
                } else if in_ground {
                    rod_damage = 2;
                }
                if rod_damage > 0 {
                    let mut damaged = stack;
                    let broken = damaged.damage_item(rod_damage);
                    new_stack = Some(if broken { None } else { Some(damaged) });
                }
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
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(boat) = self.entities.get_boat(boat_id) {
            let packets = self.tracker.track_boat_and_scan(boat, &players);
            for outbound in packets {
                self.send_tracked(outbound.recipient_id, outbound.packet);
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
        let active_dimension = self.active_dimension;
        let boat_ids: Vec<i32> = self
            .entities
            .iter_boats()
            .filter(|b| self.entities.dimension_of(b.entity_id) == active_dimension)
            .map(|b| b.entity_id)
            .collect();
        if boat_ids.is_empty() {
            return;
        }
        let mut crashed_boats = Vec::new();
        let players_pos: Vec<(i32, f64, f64, f64, bool)> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
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

            if horiz_collided
                && pre_move_speed > 0.15
                && self.config.mechanics.boats_break_on_collision
            {
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
                self.send_tracked(outbound.recipient_id, outbound.packet);
            }
            if self.config.mechanics.boats_drop_boat_not_wood {
                self.spawn_dropped_item_random(bx, by, bz, ItemStack::new(items::BOAT, 1, 0), 10);
            } else {
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
    }

    pub fn spawn_mob_at(&mut self, mob_type: MobType, x: f64, y: f64, z: f64) -> i32 {
        let mob_id = self
            .entities
            .spawn_mob(mob_type, x, y, z, &mut self.entity_random);
        if mob_type == MobType::Sheep
            && (!self.config.spawning.spawn_sheep_shades_of_black
                || !self.config.spawning.spawn_brown_and_pink_sheep)
        {
            if let Some(m) = self.entities.get_mob_mut(mob_id) {
                // The colour of the sheep is drawn with the random of the
                // world (`SpawnerCreature`), not with the one of the entities.
                let mut rng = std::mem::replace(&mut self.world.world_random, JavaRandom::new(0));
                m.sheep_color = crate::entity::mob::Mob::natural_sheep_color_ex(
                    &mut rng,
                    self.config.spawning.spawn_sheep_shades_of_black,
                    self.config.spawning.spawn_brown_and_pink_sheep,
                );
                self.world.world_random = rng;
            }
        }
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(mob) = self.entities.get_mob(mob_id) {
            for out in self.tracker.track_mob_and_scan(mob, &players) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
        mob_id
    }

    fn tick_mob_spawner(&mut self) {
        let active_dimension = self.active_dimension;
        let player_positions: Vec<(f64, f64, f64)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead && p.dimension == active_dimension)
            .map(|p| (p.x, p.y, p.z))
            .collect();
        if player_positions.is_empty() {
            return;
        }
        let (monsters, animals, water) = self.entities.count_mob_categories(active_dimension);
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

    /// Java `Entity.m_()` / `EntityLiving.m_()` — environmental damage (void, suffocation, drowning, cactus, fire, lava) for players.
    fn tick_player_environmental_damage(&mut self) {
        let active_dimension = self.active_dimension;
        let player_ids: Vec<i32> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .map(|p| p.entity_id)
            .collect();
        let mut damages: Vec<(i32, i16, &'static str)> = Vec::new();

        for pid in player_ids {
            let Some((px, py, pz, dead, sleeping, cur_fire)) = self
                .entities
                .get_player(pid)
                .map(|p| (p.x, p.y, p.z, p.dead, p.sleeping, p.fire_ticks))
            else {
                continue;
            };
            if dead {
                continue;
            }

            // 1. Void damage (`Entity.java:292-294`, `EntityLiving.java:861-872`)
            if py < -64.0 {
                damages.push((pid, 4, "fell out of the world"));
            }

            // 2. Head-level checks: in-wall suffocation (`EntityLiving.java:121-130`) & drowning (`EntityLiving.java:138-166`)
            let head_x = px.floor() as i32;
            let head_y = (py + 1.62).floor() as i32;
            let head_z = pz.floor() as i32;
            if (0..128).contains(&head_y) {
                let head_block = self.world.get_block(head_x, head_y, head_z);
                if !sleeping && blocks::is_opaque_cube(head_block) {
                    damages.push((pid, 1, "suffocated in a wall"));
                }
                if blocks::is_water(head_block) {
                    let drowned = if let Some(p) = self.entities.get_player_mut(pid) {
                        p.air_ticks -= 1;
                        if p.air_ticks <= -20 {
                            p.air_ticks = 0;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if drowned {
                        damages.push((pid, 2, "drowned"));
                    }
                } else if let Some(p) = self.entities.get_player_mut(pid) {
                    p.air_ticks = 300;
                }
            }

            let min_x = (px - 0.36).floor() as i32;
            let max_x = (px + 0.36).floor() as i32;
            let min_y = py.floor() as i32;
            let max_y = (py + 1.79).floor() as i32;
            let min_z = (pz - 0.36).floor() as i32;
            let max_z = (pz + 0.36).floor() as i32;

            let mut in_water = false;
            let mut in_lava = false;
            let mut in_fire = false;
            let mut in_cactus = false;

            for bx in min_x..=max_x {
                for by in min_y..=max_y {
                    for bz in min_z..=max_z {
                        let bid = self.world.get_block(bx, by, bz);
                        if bid == blocks::CACTUS {
                            in_cactus = true;
                        }
                        if (px - 0.3).floor() as i32 <= bx
                            && bx <= (px + 0.3).floor() as i32
                            && (pz - 0.3).floor() as i32 <= bz
                            && bz <= (pz + 0.3).floor() as i32
                        {
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
            }

            if in_cactus {
                damages.push((pid, 1, "was pricked to death"));
            }

            let in_rain = self.world.is_being_rained_on(
                px.floor() as i32,
                py.floor() as i32,
                pz.floor() as i32,
            );

            if in_water || in_rain {
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
            let dealt = self.damage_player_direct(pid, dmg, reason);
            // Poseidon `settings.fix-drowning-push-down.enabled`: vanilla
            // drowning damage sets `velocityChanged`, which pushes the client
            // down. The fix preserves the pre-damage flag.
            if dealt && reason == "drowned" && !self.config.mechanics.fix_drowning_push_down {
                if let Some(player) = self.entities.get_player_mut(pid) {
                    player.velocity_changed = true;
                }
            }
        }
    }

    /// Java `TileEntityMobSpawner.g_()` (`TileEntityMobSpawner.java:31-123`) — tick all loaded mob spawner tile entities.
    fn tick_mob_spawners(&mut self) {
        let active_dimension = self.active_dimension;
        let player_positions: Vec<(f64, f64, f64)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead && p.dimension == active_dimension)
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
            if self.config.spawning.mob_spawner_area_limit_enabled {
                let rad_blocks = (self.config.spawning.mob_spawner_chunk_radius.max(1) * 16) as f64;
                let area_count = self
                    .entities
                    .iter_mobs()
                    .filter(|m| {
                        !m.dead
                            && m.mob_type == mob_type
                            && (m.x - (sx as f64 + 0.5)).abs() <= rad_blocks
                            && (m.z - (sz as f64 + 0.5)).abs() <= rad_blocks
                    })
                    .count();
                if area_count >= self.config.spawning.mob_spawner_area_limit {
                    delay = (200 + rng.next_i32_bound(600)) as i16;
                    delay_updates.push((sx, sy, sz, delay));
                    continue;
                }
            }
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
                            2 + self.entity_random.next_i32_bound(3)
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
            MobType::Squid if self.config.mechanics.allow_milking_squids => {
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
                            let success = self.entity_random.next_i32_bound(3) == 0;
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

        if self.config.mechanics.sheep_drop_wool_on_punch {
            let mut punched_wool = None;
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                if mob.mob_type == MobType::Sheep && !mob.sheep_sheared && damage > 0 {
                    mob.sheep_sheared = true;
                    let count = 1 + self.entity_random.next_i32_bound(3);
                    punched_wool = Some((mob.x, mob.y + 0.5, mob.z, mob.sheep_color, count));
                }
            }
            if let Some((wx, wy, wz, color, count)) = punched_wool {
                for _ in 0..count {
                    self.spawn_dropped_item_random(
                        wx,
                        wy,
                        wz,
                        ItemStack::new(blocks::WOOL as i16, 1, color as i16),
                        10,
                    );
                }
            }
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

        let attacker_is_player = attacker.is_some_and(|(aid, _, _)| {
            self.entities.get_player(aid).is_some() || killed_by_skeleton_arrow
        });

        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            if mob.dead || damage < 0 || (damage == 0 && attacker.is_none()) {
                return false;
            }
            let mut adj_dmg = damage;
            if mob.mob_type == MobType::Wolf && attacker.is_some() && !attacker_is_player {
                adj_dmg = (adj_dmg + 1) / 2;
            }
            let eff_dmg = if mob.no_damage_ticks > 10 {
                if adj_dmg <= mob.last_damage {
                    return false;
                }
                let diff = adj_dmg - mob.last_damage;
                mob.last_damage = adj_dmg;
                diff
            } else {
                mob.last_damage = adj_dmg;
                mob.no_damage_ticks = 20;
                // The sound of the hit (`EntityLiving.damageEntity`): the
                // reference draws two numbers for the pitch and the volume.
                let _ = self.entity_random.next_f32();
                let _ = self.entity_random.next_f32();
                adj_dmg
            };
            mob.health -= eff_dmg;

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
                    mob.pigzombie_anger = 400 + self.entity_random.next_i32_bound(400);
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
                // The counter of the death (`EntityLiving.deathTicks`): the
                // corpse leaves the world when it passes twenty.
                mob.death_ticks = 0;
                died = true;
                let mut drops =
                    mob.get_death_drops(&mut self.entity_random, killed_by_skeleton_arrow);
                if mob.mob_type == MobType::Pig
                    && !self.config.mechanics.burning_pig_drop_cooked_meat
                {
                    for st in &mut drops {
                        if st.id == items::GRILLED_PORK {
                            st.id = items::PORK;
                        }
                    }
                }
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
                    pz_mob.pigzombie_anger = 400 + self.entity_random.next_i32_bound(400);
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
                // The mob is not removed from the world right away: the
                // corpse lives twenty more ticks (`EntityLiving.R()`), the
                // mob leaves the world when the counter of the death passes
                // twenty.
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
        let active_dimension = self.active_dimension;
        // The world removes the dead entities at the beginning of its tick
        // (`World.tickEntities`), so the mob that died in the explosion of a
        // creeper leaves the world now.
        let removed: Vec<i32> = self
            .entities
            .iter_mobs()
            .filter(|m| m.removed)
            .map(|m| m.entity_id)
            .collect();
        for mob_id in removed {
            let _ = self.entities.remove_mob(mob_id);
            for out in self.tracker.untrack_mob(mob_id) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
        // The corpses of the dead mobs leave the world at the end of the tick
        // in which they die (`World.tickEntities` removes the dead entities
        // after the update of the entities).
        let corpses: Vec<i32> = self
            .entities
            .iter_mobs()
            .filter(|m| m.dead && m.death_ticks > 20)
            .map(|m| m.entity_id)
            .collect();
        for mob_id in corpses {
            let _ = self.entities.remove_mob(mob_id);
        }
        // The mobs are updated in the order of their entity id, the reference
        // walks its entity list in the order of the insertion. The order
        // matters: the mobs share one random stream.
        let mob_ids: Vec<i32> = self
            .entities
            .iter_mobs()
            .filter(|m| self.entities.dimension_of(m.entity_id) == active_dimension)
            .map(|m| m.entity_id)
            .collect();
        if mob_ids.is_empty() {
            return;
        }
        let sky_sub = world_sky_subtracted(self.world.world_time());
        let is_daytime = sky_sub < 4;
        let players_snapshot: Vec<(i32, String, f64, f64, f64, bool)> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .map(|p| (p.entity_id, p.username.clone(), p.x, p.y, p.z, p.dead))
            .collect();

        let mut despawn_list = Vec::new();
        let mut eggs_to_drop = Vec::new();
        let mut melee_hits_player = Vec::new();
        let mut melee_hits_mob = Vec::new();
        let mut arrows_to_shoot = Vec::new();
        let mut fireballs_to_shoot = Vec::new();
        let mut status_packets = Vec::new();
        // The collision boxes of the other entities and the scratch buffer of
        // the collision boxes of the blocks (`World.getEntities`).
        let mut entity_boxes: Vec<Aabb> = Vec::new();
        let mut collision_scratch: Vec<Aabb> = Vec::new();

        for mob_id in mob_ids {
            let Some((mtype, mx, my, mz, can_desp, length, width, step_offset)) =
                self.entities.get_mob(mob_id).map(|m| {
                    let (length, width) = m.size();
                    (
                        m.mob_type,
                        m.x,
                        m.y,
                        m.z,
                        m.can_despawn(),
                        length,
                        width,
                        m.step_offset,
                    )
                })
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

            // The update of the mob before the AI (`Entity.R()` and
            // `EntityLiving.R()`): the splash in the water, the damage of the
            // fire and the lava and the idle sound of the mob draw from the
            // shared random of the entities.
            //
            // The corpse of a dead mob leaves the world at the end of the
            // tick (`World.tickEntities`), so the mob is still updated.
            let left_the_world = self.mob_update_phase(mob_id);
            if self.entities.get_mob(mob_id).is_none() {
                continue;
            }

            // The corpse of a dead mob is not despawned, it leaves the world
            // when the counter of the death passes twenty. A dead mob has no
            // artificial intelligence (`EntityLiving.v()`), it only falls.
            let corpse = self.entities.get_mob(mob_id).is_some_and(|m| m.dead);
            if can_desp && !corpse {
                // The despawn of the mob (`EntityLiving.U()`) is checked when
                // the mob walks without a path (see the update of the mob
                // below).
            }

            // The counter of the ticks without a target grows faster in the
            // light (`EntityMonster.v()`).
            if !corpse && mtype.is_monster() {
                let light = self
                    .entities
                    .get_mob(mob_id)
                    .map(|mob| crate::entity::mob::mob_light_at(&self.world, mob, sky_sub))
                    .unwrap_or(0.0);
                if light > 0.5 {
                    if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                        mob.no_despawn_ticks += 2;
                    }
                }
            }

            // Daylight combustion for Zombie and Skeleton (`EntityZombie.v()`,
            // `EntitySkeleton.v()`): the reference draws a number every tick
            // the mob sees the sun.
            if is_daytime && matches!(mtype, MobType::Zombie | MobType::Skeleton) {
                // The light of the mob (`Entity.c(float)`): the curve of the
                // light at the feet of the mob.
                let light_y = (my + (width as f64) * 0.66).floor() as i32;
                let light = light_curve(chunk_light_at(
                    &self.world,
                    mx.floor() as i32,
                    light_y,
                    mz.floor() as i32,
                    sky_sub,
                ));
                // The reference checks that the block of the mob is above the
                // terrain (`World.isChunkLoaded`), only then it draws the
                // number of the combustion.
                let above_terrain = self.world.java_can_see_sky(
                    mx.floor() as i32,
                    my.floor() as i32,
                    mz.floor() as i32,
                );
                if light > 0.5 && above_terrain {
                    let burn = self.entity_random.next_f32() * 30.0 < (light - 0.4) * 2.0;
                    if burn {
                        if let Some(m) = self.entities.get_mob_mut(mob_id) {
                            m.fire_ticks = 300;
                        }
                    }
                }
            }

            // Resolve current target position (player or sheep/mob)
            let cur_target_id = self.entities.get_mob(mob_id).and_then(|m| m.target_id);
            let mut target_pos: Option<(i32, bool, f64, f64, f64)> = None;
            // The height of the eyes of the target (`EntityLiving.t()`).
            let mut target_eye_height = PLAYER_EYE_HEIGHT;
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
                        // The eyes of the target (`EntityLiving.t()`).
                        target_eye_height = tmob.eye_height();
                    }
                }
            }

            // Acquire new target if none
            if target_pos.is_none() && !corpse {
                match mtype {
                    MobType::Zombie | MobType::Skeleton | MobType::Creeper | MobType::Giant => {
                        if let Some((pid, px, py, pz, dsq)) = closest_player {
                            if dsq <= 16.0 * 16.0 {
                                target_pos = Some((pid, true, px, py, pz));
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
                            // The wolf hunts a sheep (`EntityWolf.c_()`): the
                            // reference draws a number of the world, not of
                            // the entities.
                            let hunt_sheep = {
                                let mut rng = std::mem::replace(
                                    &mut self.world.world_random,
                                    JavaRandom::new(0),
                                );
                                let hunt = rng.next_i32_bound(100) == 0;
                                self.world.world_random = rng;
                                hunt
                            };
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
                        .is_some_and(|_| self.entity_random.next_i32_bound(100) == 0)
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

            let has_los = target_pos.is_some_and(|(_, _, tx, ty, tz)| {
                self.world
                    .ray_trace((mx, my + 1.0, mz), (tx, ty + 1.0, tz), false)
                    .is_none()
            });

            // The attack of the mob (`EntityCreature.c_()`): the melee, the
            // fuse of the creeper and the bow of the skeleton. The reference
            // attacks before the path of the mob, the attack of the creeper
            // and of the skeleton sets the flag of the mob that sees its
            // target (`EntityLiving.e`).
            //
            // The explosion of the creeper happens inside the attack, before
            // the movement of the mob: the mob falls into the hole of the
            // explosion in the same tick.
            let mut creeper_explosion: Option<(f64, f64, f64, f32)> = None;
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                // The flag of the mob that sees its target
                // (`EntityCreature.c_()`): it is reset at the beginning of
                // the artificial intelligence.
                mob.sees_target = mob.reference_sees_target();
                if let Some((tid, is_player, tx, ty, tz)) = target_pos {
                    let dx = tx - mob.x;
                    let dy = ty - mob.y;
                    let dz = tz - mob.z;
                    let dist = (dx * dx + dy * dy + dz * dz).sqrt();

                    match mob.mob_type {
                        MobType::Creeper => {
                            // The fuse of the creeper (`EntityCreeper.a`):
                            // it burns while the target is near.
                            if (mob.creeper_state > 0 && dist < 7.0)
                                || (mob.creeper_state <= 0 && dist < 3.0)
                            {
                                mob.creeper_state = 1;
                                mob.creeper_fuse += 1;
                                // The mob sees its target
                                // (`EntityCreeper.a`).
                                mob.sees_target = true;
                                if mob.creeper_fuse >= 30 {
                                    let radius: f32 = if mob.creeper_powered { 6.0 } else { 3.0 };
                                    // The creeper dies in the explosion
                                    // (`EntityCreeper.a`): its health does not
                                    // change, the world removes the entity at
                                    // the beginning of its next tick
                                    // (`World.tickEntities`).
                                    mob.removed = true;
                                    creeper_explosion = Some((mob.x, mob.y, mob.z, radius));
                                }
                            } else {
                                mob.creeper_state = -1;
                                if mob.creeper_fuse > 0 {
                                    mob.creeper_fuse -= 1;
                                }
                            }
                        }
                        MobType::Skeleton => {
                            // The bow of the skeleton (`EntitySkeleton.a`):
                            // the mob shoots only when it sees its target.
                            if dist < 10.0 && has_los {
                                if mob.attack_ticks == 0 {
                                    // The arrow of the reference: the
                                    // constructor of the arrow and its
                                    // direction draw three normal values
                                    // each (`EntityArrow.a`).
                                    let spread: Vec<f64> = (0..6)
                                        .map(|_| self.entity_random.next_gaussian())
                                        .collect();
                                    arrows_to_shoot.push((
                                        mob_id,
                                        mob.x,
                                        mob.y + 1.0,
                                        mob.z,
                                        dx,
                                        (ty + 1.4) - (mob.y + 1.2),
                                        dz,
                                        spread,
                                    ));
                                    mob.attack_ticks = 30;
                                }
                                // The yaw of the mob is the direction of the
                                // target (`EntitySkeleton.a`).
                                mob.yaw = (crate::util::fdlibm::atan2(dz, dx) * 180.0
                                    / crate::entity::mob::JAVA_PI_DOUBLE)
                                    as f32
                                    - 90.0;
                                // The mob sees its target
                                // (`EntitySkeleton.a`).
                                mob.sees_target = true;
                            }
                        }
                        MobType::Spider | MobType::Wolf => {
                            if has_los
                                && dist > 2.0
                                && dist < 6.0
                                && mob.on_ground
                                && self.entity_random.next_i32_bound(10) == 0
                            {
                                let h = (dx * dx + dz * dz).sqrt().max(0.01);
                                mob.mot_x = dx / h * 0.4 + mob.mot_x * 0.2;
                                mob.mot_z = dz / h * 0.4 + mob.mot_z * 0.2;
                                mob.mot_y = 0.4;
                            } else if has_los
                                && dist < 2.0
                                && dy.abs() < 2.0
                                && mob.attack_ticks <= 0
                            {
                                mob.attack_ticks = 20;
                                let dmg =
                                    mob.mob_type.attack_damage(mob.slime_size, mob.wolf_tamed);
                                if is_player {
                                    melee_hits_player.push((tid, dmg, mob.x, mob.z));
                                } else {
                                    melee_hits_mob.push((tid, dmg, mob_id, mob.x, mob.z));
                                }
                            }
                        }
                        _ => {
                            let dmg = mob.mob_type.attack_damage(mob.slime_size, mob.wolf_tamed);
                            if has_los
                                && dmg > 0
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
            }
            // The explosion of the creeper (`EntityCreeper.a`): the blocks of
            // the blast are destroyed before the movement of the mob.
            if let Some((ex, ey, ez, radius)) = creeper_explosion {
                let allow_blocks = self.config.mechanics.creeper_block_damage;
                self.create_explosion_ex(ex, ey, ez, radius, false, allow_blocks, Some(mob_id));
            }

            // Compute A* path when needed
            let mut new_path: Option<Vec<(i32, i32, i32)>> = None;
            let mut teleport_to_owner: Option<(f64, f64, f64)> = None;
            if corpse {
                // A dead mob has no artificial intelligence
                // (`EntityLiving.v()`): the input of the movement is zero.
            } else if let Some((ox, oy, oz)) = owner_pos {
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
                    let should_repath = self.entities.get_mob_mut(mob_id).is_some_and(|m| {
                        m.path.is_empty() || self.entity_random.next_i32_bound(20) == 0
                    });
                    if should_repath {
                        new_path = Some(find_path(
                            &self.world,
                            mx,
                            my + step_offset as f64,
                            mz,
                            length,
                            width,
                            ox,
                            oy,
                            oz,
                            16.0,
                        ));
                    }
                }
            } else if !matches!(mtype, MobType::Ghast | MobType::Slime | MobType::Squid) {
                // The path of the mob (`EntityCreature.c_()`).
                //
                // The mob walks to its target or wanders around. The
                // reference draws the number of the walk even when the mob
                // has a target, so the order of the draws matters.
                let mut path_to_target = false;
                // The path to the target is computed as soon as the mob finds
                // its target (`EntityCreature.c_()`).
                if cur_target_id.is_none() {
                    if let Some((tid, is_player, tx, ty, tz)) = target_pos {
                        new_path = Some(find_path(
                            &self.world,
                            mx,
                            my + step_offset as f64,
                            mz,
                            length,
                            width,
                            tx,
                            self.target_box_bottom(tid, is_player, ty),
                            tz,
                            16.0,
                        ));
                    }
                }
                // The path of the mob (`this.pathEntity` of the reference).
                let has_path = new_path.is_some()
                    || self
                        .entities
                        .get_mob(mob_id)
                        .is_some_and(|m| !m.path.is_empty());
                // The flag of the mob that sees its target
                // (`EntityLiving.e`).
                let sees_target = self.entities.get_mob(mob_id).is_some_and(|m| m.sees_target);
                // The mob that attacked its target does not walk to it
                // again (`EntityCreature.c_()`): the flag of the mob that
                // sees its target blocks the path and the walk.
                if !sees_target {
                    if let Some((tid, is_player, tx, ty, tz)) = target_pos {
                        // The path to the target: it is computed when the path is
                        // missing or when the roll of the reference hits.
                        let should_repath = !has_path || self.entity_random.next_i32_bound(20) == 0;
                        if should_repath {
                            new_path = Some(find_path(
                                &self.world,
                                mx,
                                my + step_offset as f64,
                                mz,
                                length,
                                width,
                                tx,
                                self.target_box_bottom(tid, is_player, ty),
                                tz,
                                16.0,
                            ));
                            path_to_target = true;
                        }
                    }
                    if !path_to_target {
                        // The walk of the mob (`EntityCreature.B()`).
                        let has_path = new_path.is_some()
                            || self
                                .entities
                                .get_mob(mob_id)
                                .is_some_and(|m| !m.path.is_empty());
                        let wander = self.entities.get_mob_mut(mob_id).is_some_and(|m| {
                            !m.wolf_sitting
                                && ((!has_path && self.entity_random.next_i32_bound(80) == 0)
                                    || self.entity_random.next_i32_bound(80) == 0)
                        });
                        if wander {
                            // The goal of the walk: the best of ten candidates.
                            let mut wander_goal = None;
                            if let Some(m) = self.entities.get_mob_mut(mob_id) {
                                let mut best_score = -99999.0f32;
                                for _ in 0..10 {
                                    let rx =
                                        (m.x + (self.entity_random.next_i32_bound(13) - 6) as f64)
                                            .floor() as i32;
                                    let ry =
                                        (m.y + (self.entity_random.next_i32_bound(7) - 3) as f64)
                                            .floor() as i32;
                                    let rz =
                                        (m.z + (self.entity_random.next_i32_bound(13) - 6) as f64)
                                            .floor() as i32;
                                    // The score of a block (`EntityAnimal.a` and
                                    // `EntityMonster.a`): the light of the block
                                    // goes through the curve of the reference.
                                    let light = light_curve(chunk_light_at(
                                        &self.world,
                                        rx,
                                        ry,
                                        rz,
                                        sky_sub,
                                    ));
                                    let score = if m.mob_type.is_animal() {
                                        if self.world.get_block(rx, ry - 1, rz) == blocks::GRASS {
                                            10.0
                                        } else {
                                            light - 0.5
                                        }
                                    } else {
                                        0.5 - light
                                    };
                                    if score > best_score {
                                        best_score = score;
                                        wander_goal = Some((rx, ry, rz));
                                    }
                                }
                            }
                            if let Some((gx, gy, gz)) = wander_goal {
                                new_path = Some(find_path(
                                    &self.world,
                                    mx,
                                    my + step_offset as f64,
                                    mz,
                                    length,
                                    width,
                                    gx as f64 + 0.5,
                                    gy as f64 + 0.5,
                                    gz as f64 + 0.5,
                                    10.0,
                                ));
                            }
                        }
                    }
                }
            }

            let in_rain = self.world.is_being_rained_on(
                mx.floor() as i32,
                my.floor() as i32,
                mz.floor() as i32,
            );

            // The collision boxes of the other entities (`World.getEntities`).
            entity_boxes.clear();
            self.collect_entity_boxes(mob_id, &mut entity_boxes);

            // Now update the mob state and physics
            let mut rider_sync: Option<(i32, f64, f64, f64)> = None;
            let mut stepped_soil: Option<(i32, i32, i32)> = None;
            // The position of the mob before the movement (the soil under the
            // mob and the fall of the mob).
            let mut pre_y = 0.0;
            let mut pre_mot_y = 0.0;
            // The state of the mob before the movement and the fall damage
            // (`EntityLiving.a(float, float)`).
            let mut was_on_ground = false;
            let mut fall_damage = 0.0f32;
            // The damage of the fall and of the cactus, they are applied
            // after the movement of the mob.
            let mut fall_hit: i16 = 0;
            let mut cactus_hit = false;
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                pre_y = mob.y;
                pre_mot_y = mob.mot_y;
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

                // The void of the mob is handled in the update of the mob
                // (`Entity.R()`).

                // Wolf water shaking (`EntityWolf.v()`, `m_()`)
                if mob.mob_type == MobType::Wolf {
                    if mob.in_water || in_rain {
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
                if corpse {
                    // A dead mob has no artificial intelligence
                    // (`EntityLiving.v()`): the input of the movement is zero
                    // and the mob does not jump.
                    mob.move_strafe = 0.0;
                    mob.move_forward = 0.0;
                    mob.jump_flag = false;
                } else {
                    match mob.mob_type {
                        MobType::Squid => {
                            // The direction of the swim (`EntitySquid.c_()`): the
                            // squid picks a new direction now and then, the
                            // reference draws the number every tick.
                            if self.entity_random.next_i32_bound(50) == 0
                                || !mob.in_water
                                || (mob.squid_swim_vec.0 == 0.0
                                    && mob.squid_swim_vec.1 == 0.0
                                    && mob.squid_swim_vec.2 == 0.0)
                            {
                                let angle =
                                    self.entity_random.next_f32() * std::f32::consts::PI * 2.0;
                                mob.squid_swim_vec = (
                                    crate::util::math::cos(angle) * 0.2,
                                    -0.1 + self.entity_random.next_f32() * 0.2,
                                    crate::util::math::sin(angle) * 0.2,
                                );
                            }
                        }
                        MobType::Slime => {
                            // The slime looks at a nearby player (`EntitySlime.c_()`).
                            let mut player_nearby = false;
                            if let Some((pid, px, py, pz, dsq)) = closest_player {
                                if dsq <= 16.0 * 16.0 {
                                    player_nearby = true;
                                    look_at(mob, px, py, pz, PLAYER_EYE_HEIGHT, 10.0, 20.0);
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
                            // The hop of the slime (`EntitySlime.c_()`): the timer
                            // of the reference is decremented on the ground and
                            // the slime hops when it reaches zero.
                            if mob.on_ground {
                                let timer = mob.slime_jump_delay;
                                mob.slime_jump_delay -= 1;
                                if timer <= 0 {
                                    mob.slime_jump_delay =
                                        self.entity_random.next_i32_bound(20) + 10;
                                    if player_nearby {
                                        mob.slime_jump_delay /= 3;
                                    }
                                    mob.jump_flag = true;
                                    if mob.slime_size > 1 {
                                        // The sound of the hop: two numbers.
                                        let _ = self.entity_random.next_f32();
                                        let _ = self.entity_random.next_f32();
                                    }
                                    // The direction of the hop.
                                    mob.move_strafe = 1.0 - self.entity_random.next_f32() * 2.0;
                                    mob.move_forward = mob.slime_size as f32;
                                } else {
                                    mob.jump_flag = false;
                                    mob.move_strafe = 0.0;
                                    mob.move_forward = 0.0;
                                }
                            } else {
                                mob.jump_flag = false;
                            }
                        }
                        MobType::Ghast => {
                            // The flight of the ghast (`EntityGhast.c_()`): the
                            // ghast flies to a waypoint and picks a new one when
                            // it reaches it.
                            let dx = mob.ghast_waypoint.0 - mob.x;
                            let dy = mob.ghast_waypoint.1 - mob.y;
                            let dz = mob.ghast_waypoint.2 - mob.z;
                            // `MathHelper.a(double)`: the square root is a single
                            // precision value.
                            let dist = ((dx * dx + dy * dy + dz * dz).sqrt() as f32) as f64;
                            if dist < 1.0 || dist > 60.0 {
                                mob.ghast_waypoint = (
                                    mob.x
                                        + ((self.entity_random.next_f32() * 2.0 - 1.0) * 16.0)
                                            as f64,
                                    mob.y
                                        + ((self.entity_random.next_f32() * 2.0 - 1.0) * 16.0)
                                            as f64,
                                    mob.z
                                        + ((self.entity_random.next_f32() * 2.0 - 1.0) * 16.0)
                                            as f64,
                                );
                            } else {
                                let cooldown = mob.ghast_course_cooldown;
                                mob.ghast_course_cooldown -= 1;
                                if cooldown <= 0 {
                                    mob.ghast_course_cooldown +=
                                        self.entity_random.next_i32_bound(5) + 2;
                                    // The way of the ghast has to be free
                                    // (`EntityGhast.a(double, double, double,
                                    // double)`).
                                    if ghast_path_is_free(mob, &self.world, dist) {
                                        mob.mot_x += dx / dist * 0.1;
                                        mob.mot_y += dy / dist * 0.1;
                                        mob.mot_z += dz / dist * 0.1;
                                    } else {
                                        mob.ghast_waypoint = (mob.x, mob.y, mob.z);
                                    }
                                }
                            }

                            if let Some((_, _, tx, ty, tz)) = target_pos {
                                let tdx = tx - mob.x;
                                let tdy = (ty + 1.0) - (mob.y + 2.0);
                                let tdz = tz - mob.z;
                                if tdx * tdx + tdy * tdy + tdz * tdz < 64.0 * 64.0 && has_los {
                                    mob.yaw = -(crate::util::fdlibm::atan2(tdx, tdz) as f32)
                                        * 180.0
                                        / crate::entity::mob::JAVA_PI;
                                    mob.look_yaw = mob.yaw;
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
                                // The ghast looks into the direction of the
                                // flight (`EntityGhast.c_()`).
                                let yaw = -(crate::util::fdlibm::atan2(mob.mot_x, mob.mot_z)
                                    as f32)
                                    * 180.0
                                    / crate::entity::mob::JAVA_PI;
                                mob.yaw = yaw;
                                mob.look_yaw = yaw;
                                if mob.ghast_charge > 0 {
                                    mob.ghast_charge -= 1;
                                }
                            }
                        }
                        _ => {
                            // The path following (`EntityCreature.c_()`).
                            //
                            // The mob walks to the next point of the path, turns at
                            // most 30 degrees and jumps when the point is higher
                            // or when it is blocked without a path.
                            //
                            // The pitch of a creature is reset before it looks at
                            // its target (`EntityCreature.c_()`).
                            mob.pitch = 0.0;
                            let mut jump = false;
                            if !mob.wolf_sitting && !mob.wolf_shaking {
                                if !mob.path.is_empty()
                                    && self.entity_random.next_i32_bound(100) != 0
                                {
                                    // The jump flag is reset (`EntityCreature.c_()`).
                                    mob.jump_flag = false;
                                    // The next point of the path (`PathEntity.a`).
                                    let half = ((length + 1.0) as i32) as f64 * 0.5;
                                    let mut point = mob.path.get(mob.path_index).copied();
                                    let threshold = (length * 2.0) as f64;
                                    while let Some((wx, _wy, wz)) = point {
                                        let dx = wx as f64 + half - mob.x;
                                        let dz = wz as f64 + half - mob.z;
                                        if dx * dx + dz * dz >= threshold * threshold {
                                            break;
                                        }
                                        mob.path_index += 1;
                                        if mob.path_index >= mob.path.len() {
                                            mob.path.clear();
                                            mob.path_index = 0;
                                            point = None;
                                            break;
                                        }
                                        point = mob.path.get(mob.path_index).copied();
                                    }
                                    if let Some((wx, wy, wz)) = point {
                                        let dx = wx as f64 + half - mob.x;
                                        let dz = wz as f64 + half - mob.z;
                                        // The height of the mob
                                        // (`EntityCreature.c_()`): the reference
                                        // takes the floor of the bottom of the box
                                        // plus a half.
                                        let level = (mob.y + 0.5).floor();
                                        let dy = wy as f64 - level;
                                        // The reference uses its own
                                        // `TrigMath.atan2` here.
                                        let target_yaw = (crate::util::math::trig::atan2(dz, dx)
                                            * 180.0
                                            / crate::entity::mob::JAVA_PI_DOUBLE)
                                            as f32
                                            - 90.0;
                                        let mut diff = target_yaw - mob.yaw;
                                        while diff < -180.0 {
                                            diff += 360.0;
                                        }
                                        while diff >= 180.0 {
                                            diff -= 360.0;
                                        }
                                        mob.yaw += diff.clamp(-30.0, 30.0);
                                        // The forward input of the mob (`EntityLiving.aA`).
                                        mob.move_forward =
                                            mob.mob_type.move_speed(mob.target_id.is_some());
                                        // The mob that sees its target looks
                                        // straight at it and walks sideways
                                        // (`EntityCreature.c_()`).
                                        if mob.sees_target {
                                            if let Some((_, _, tx, _, tz)) = target_pos {
                                                let d4 = tx - mob.x;
                                                let d5 = tz - mob.z;
                                                let f4 = mob.yaw;
                                                // The reference uses the
                                                // `Math.atan2` of the JDK.
                                                mob.yaw = (crate::util::fdlibm::atan2(d5, d4)
                                                    * 180.0
                                                    / crate::entity::mob::JAVA_PI_DOUBLE)
                                                    as f32
                                                    - 90.0;
                                                let f3 = (f4 - mob.yaw + 90.0)
                                                    * std::f32::consts::PI
                                                    / 180.0;
                                                mob.move_strafe =
                                                    -crate::util::math::sin(f3) * mob.move_forward;
                                                mob.move_forward *= crate::util::math::cos(f3);
                                            }
                                        }
                                        if dy > 0.0 {
                                            jump = true;
                                        }
                                    }
                                    // The mob looks at its target (`EntityCreature.c_()`).
                                    if let Some((_, _, tx, ty, tz)) = target_pos {
                                        look_at(mob, tx, ty, tz, target_eye_height, 30.0, 30.0);
                                    }
                                    // The jump when the mob is blocked and has no path.
                                    if mob.position_changed && mob.path.is_empty() {
                                        jump = true;
                                    }
                                    // The jump in the water or the lava
                                    // (`EntityCreature.c_()`); the reference draws
                                    // the number only while it follows a path.
                                    if self.entity_random.next_f32() < 0.8
                                        && (mob.in_water || mob.in_lava)
                                    {
                                        jump = true;
                                    }
                                } else {
                                    // The random look around (`EntityLiving.c_()`).
                                    mob.path.clear();
                                    mob.path_index = 0;
                                    // The counter of the ticks without a target
                                    // (`EntityLiving.ay`).
                                    mob.no_despawn_ticks += 1;
                                    // The despawn of the mob (`EntityLiving.U()`).
                                    if can_desp {
                                        if let Some((_, _, _, _, dsq)) = closest_player {
                                            if dsq > 128.0 * 128.0 {
                                                despawn_list.push(mob_id);
                                            } else if mob.no_despawn_ticks > 600
                                                && self.entity_random.next_i32_bound(800) == 0
                                            {
                                                if dsq < 32.0 * 32.0 {
                                                    mob.no_despawn_ticks = 0;
                                                } else {
                                                    despawn_list.push(mob_id);
                                                }
                                            }
                                        }
                                    }
                                    random_look(mob, &mut self.entity_random, &players_snapshot);
                                }
                            }
                            if jump {
                                mob.jump_flag = true;
                            }
                        }
                    }
                }

                // The jump (`EntityLiving.v()`): the mob swims up in the water
                // and jumps on the ground.
                if mob.jump_flag {
                    if mob.in_water || mob.in_lava {
                        // The constant of the reference is the single precision
                        // value of `0.04` (`EntityLiving.v()`).
                        mob.mot_y += 0.039_999_999_105_930_33;
                    } else if mob.on_ground {
                        // `EntityLiving.O()`: the constant of the reference
                        // is the single precision value.
                        mob.mot_y = 0.419_999_986_886_978_15;
                    }
                }

                // The movement input is damped (`EntityLiving.v()`).
                mob.move_strafe *= 0.98;
                mob.move_forward *= 0.98;
                mob.yaw_drift *= 0.9;

                // The movement of the mob (`EntityLiving.a(float, float)`).
                was_on_ground = mob.on_ground;
                fall_damage = mob.apply_movement(
                    &self.world,
                    &entity_boxes,
                    &mut collision_scratch,
                    self.config.mechanics.spiders_climb_walls,
                    mob.mob_type == MobType::Ghast,
                );
            }

            // The fire of the mob (`Entity.move()`): the damage of the fire,
            // the counter of the fire and the sound of the fizz. The
            // reference does it inside the movement of the entity, so it is
            // between the movement of the mob and the rest of its update.
            self.mob_fire_phase(mob_id);

            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                // The swim of the squid (`EntitySquid.v()`): the phase of the
                // swim and the velocity of the squid, they are set after the
                // movement of the mob.
                if mob.mob_type == MobType::Squid {
                    mob.squid_phase += mob.squid_phase_speed;
                    if mob.squid_phase > 6.2831855 {
                        mob.squid_phase -= 6.2831855;
                        if self.entity_random.next_i32_bound(10) == 0 {
                            mob.squid_phase_speed =
                                1.0 / (self.entity_random.next_f32() + 1.0) * 0.2;
                        }
                    }
                    // The squid has its own check for the water
                    // (`EntitySquid.ad()`).
                    let squid_water =
                        crate::entity::physics::squid_in_water(&self.world, &mob.aabb());
                    if squid_water {
                        if mob.squid_phase < std::f32::consts::PI {
                            let f = mob.squid_phase / std::f32::consts::PI;
                            mob.squid_tentacle_angle =
                                crate::util::math::sin(f * f * std::f32::consts::PI)
                                    * std::f32::consts::PI
                                    * 0.25;
                            if f > 0.75 {
                                mob.squid_speed = 1.0;
                                mob.squid_squash = 1.0;
                            } else {
                                mob.squid_squash *= 0.8;
                            }
                        } else {
                            mob.squid_tentacle_angle = 0.0;
                            mob.squid_speed *= 0.9;
                            mob.squid_squash *= 0.99;
                        }
                        if !mob.dead {
                            mob.mot_x = (mob.squid_swim_vec.0 * mob.squid_speed) as f64;
                            mob.mot_y = (mob.squid_swim_vec.1 * mob.squid_speed) as f64;
                            mob.mot_z = (mob.squid_swim_vec.2 * mob.squid_speed) as f64;
                        }
                        // The yaw of the swim (`EntityLiving.K`).
                        mob.look_yaw +=
                            (-(crate::util::fdlibm::atan2(mob.mot_x, mob.mot_z) as f32) * 180.0
                                / crate::entity::mob::JAVA_PI
                                - mob.look_yaw)
                                * 0.1;
                        mob.yaw = mob.look_yaw;
                    } else {
                        mob.squid_tentacle_angle = crate::util::math::sin(mob.squid_phase).abs()
                            * std::f32::consts::PI
                            * 0.25;
                        if !mob.dead {
                            mob.mot_x = 0.0;
                            mob.mot_y -= 0.08;
                            // The constant of the reference is the single
                            // precision value of `0.98`.
                            mob.mot_y *= 0.980_000_019_073_486_3;
                            mob.mot_z = 0.0;
                        }
                    }
                }

                // The chicken (`EntityChicken.v()`): the flap of the wings and
                // the timer of the egg. The reference does it after the
                // movement of the mob.
                if mob.mob_type == MobType::Chicken {
                    mob.chicken_wing = mob.chicken_wing_tip;
                    mob.chicken_wing_tip += if mob.on_ground { -0.3 } else { 1.2 };
                    mob.chicken_wing_tip = mob.chicken_wing_tip.clamp(0.0, 1.0);
                    if !mob.on_ground && mob.chicken_flap < 1.0 {
                        mob.chicken_flap = 1.0;
                    }
                    mob.chicken_flap *= 0.9;
                    if !mob.on_ground && mob.mot_y < 0.0 {
                        mob.mot_y *= 0.6;
                    }
                    mob.chicken_wing += mob.chicken_flap * 2.0;
                    mob.chicken_egg_timer -= 1;
                    if mob.chicken_egg_timer <= 0 {
                        // The sound of the chicken: the pitch and the volume
                        // are two numbers.
                        let _ = self.entity_random.next_f32();
                        let _ = self.entity_random.next_f32();
                        eggs_to_drop.push((mob.x, mob.y, mob.z));
                        mob.chicken_egg_timer = self.entity_random.next_i32_bound(6000) + 6000;
                    }
                }

                // The landing of the slime (`EntitySlime.m_()`): the particles
                // of the splash and the sound are drawn after the movement of
                // the mob.
                if mob.mob_type == MobType::Slime && mob.on_ground && !was_on_ground {
                    for _ in 0..(mob.slime_size * 8) {
                        let _ = self.entity_random.next_f32();
                        let _ = self.entity_random.next_f32();
                    }
                    if mob.slime_size > 2 {
                        let _ = self.entity_random.next_f32();
                        let _ = self.entity_random.next_f32();
                    }
                }

                // The look of the mob (`EntityLiving.m_()`): the yaw the mob
                // looks at follows the direction of the movement.
                {
                    let dx = mob.x - mob.last_x;
                    let dz = mob.z - mob.last_z;
                    let moved = crate::util::math::math_helper_a(dx * dx + dz * dz);
                    let mut look = mob.look_yaw;
                    if moved > 0.05 {
                        // The reference uses its own `TrigMath.atan2` here and
                        // the whole expression is a single precision one.
                        look = (crate::util::math::trig::atan2(dz, dx) as f32) * 180.0
                            / crate::entity::mob::JAVA_PI
                            - 90.0;
                    }
                    let mut diff = look - mob.look_yaw;
                    while diff < -180.0 {
                        diff += 360.0;
                    }
                    while diff >= 180.0 {
                        diff -= 360.0;
                    }
                    mob.look_yaw += diff * 0.3;
                    let mut diff = mob.yaw - mob.look_yaw;
                    while diff < -180.0 {
                        diff += 360.0;
                    }
                    while diff >= 180.0 {
                        diff -= 360.0;
                    }
                    diff = diff.clamp(-75.0, 75.0);
                    mob.look_yaw = mob.yaw - diff;
                    if diff * diff > 2500.0 {
                        mob.look_yaw += diff * 0.2;
                    }
                }

                // The fall damage of the mob (`EntityLiving.a(float)`): the
                // reference hits the mob once with all the damage.
                if fall_damage > 3.0 {
                    fall_hit = ((fall_damage - 3.0).ceil() as i32).max(0) as i16;
                }
                // The cactus (`BlockCactus.a`): the mob inside a cactus takes
                // one point of damage. The reference applies it in the
                // collision of the movement, after the fall damage.
                let bx = mob.x.floor() as i32;
                let by = mob.y.floor() as i32;
                let bz = mob.z.floor() as i32;
                if self.world.get_block(bx, by, bz) == blocks::CACTUS {
                    cactus_hit = true;
                }
            }

            // The fall damage and the cactus of the mob (`Entity.move()`):
            // the reference applies them in the collision of the movement.
            if fall_hit > 0 {
                let _ = self.damage_mob(mob_id, fall_hit, None, false);
            }
            if cactus_hit {
                let _ = self.damage_mob(mob_id, 1, None, false);
            }

            if left_the_world {
                // The corpse of the mob left the world (`Entity.die()` and
                // `World.tickEntities`): the client stops to see it, the
                // entity itself is removed at the beginning of the next tick.
                for out in self.tracker.untrack_mob(mob_id) {
                    self.send_tracked(out.recipient_id, out.packet);
                }
            }

            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                // The soil under the mob is trampled (`BlockFarmland`).
                let below_y = (pre_y + pre_mot_y).floor() as i32;
                let below =
                    self.world
                        .get_block(mob.x.floor() as i32, below_y, mob.z.floor() as i32);
                if below == blocks::FARMLAND
                    && (mob.mob_type != MobType::Spider
                        || self.config.mechanics.spiders_trample_crops)
                {
                    stepped_soil = Some((mob.x.floor() as i32, below_y, mob.z.floor() as i32));
                }
                // The rider of the mob follows it.
                if let Some(pid) = mob.passenger_id {
                    rider_sync = Some((pid, mob.x, mob.y + mob.mounted_y_offset(), mob.z));
                }
            }

            if let Some((sx, sy, sz)) = stepped_soil {
                if !self.config.mechanics.trample_farmland_above_fence
                    || sy == 0
                    || self.world.get_block(sx, sy - 1, sz) != blocks::FENCE
                {
                    self.world.trample_farmland(sx, sy, sz);
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
                self.send_tracked(out.recipient_id, out.packet);
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

        for (pid, dmg, ax, az) in melee_hits_player {
            self.damage_player_from_mob(pid, dmg, ax, az);
        }

        for (target_mob_id, dmg, att_id, ax, az) in melee_hits_mob {
            let _ = self.damage_mob(target_mob_id, dmg, Some((att_id, ax, az)), false);
        }

        for (skel_id, sx, sy, sz, dx, dy, dz, spread) in arrows_to_shoot {
            let h = (dx * dx + dz * dz).sqrt() * 0.2;
            let len = (dx * dx + (dy + h).powi(2) + dz * dz).sqrt().max(0.001);
            // The spread of the arrow (`EntityArrow.a`): three normal values
            // with the factor of the spread of the arrow.
            let vx = (dx / len + spread[3] * 0.007_499_999_832_361_937 * 12.0) * 0.6;
            let vy = ((dy + h) / len + spread[4] * 0.007_499_999_832_361_937 * 12.0) * 0.6;
            let vz = (dz / len + spread[5] * 0.007_499_999_832_361_937 * 12.0) * 0.6;
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
            if self.config.mechanics.skeleton_shooting_sound_fix {
                self.broadcast(Clientbound::LevelSound(LevelSound {
                    effect_id: 1002,
                    x: sx.floor() as i32,
                    y: sy.floor().clamp(0.0, 127.0) as u8,
                    z: sz.floor() as i32,
                    data: 0,
                }));
            }
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
    }

    /// The collision boxes of the other entities (`World.b(Entity,
    /// AxisAlignedBB)`).
    fn collect_entity_boxes(&mut self, mob_id: i32, out: &mut Vec<Aabb>) {
        out.clear();
        for other in self.entities.iter_mobs() {
            if other.entity_id == mob_id || other.dead {
                continue;
            }
            out.push(other.aabb());
        }
        for player in self.entities.iter_players() {
            out.push(player.bounding_box());
        }
    }

    /// Java `Entity.R()` and `EntityLiving.R()`: the update of the mob before
    /// the AI of the mob.
    ///
    /// The splash in the water, the damage of the fire and the lava and the
    /// idle sound of the mob draw from the shared random of the entities. The
    /// order of the draws is the order of the reference: the splash, the
    /// damage of the fire, the damage of the lava, the idle sound.
    /// The bottom of the box of the target (`Entity.boundingBox.b`).
    ///
    /// The goal of the path of a mob is the bottom of the box of its target,
    /// not its position.
    fn target_box_bottom(&self, target_id: i32, is_player: bool, target_y: f64) -> f64 {
        if is_player {
            self.entities
                .get_player(target_id)
                .map(|player| player.box_bottom())
                .unwrap_or(target_y)
        } else {
            self.entities
                .get_mob(target_id)
                .map(|mob| mob.y + mob.step_offset as f64)
                .unwrap_or(target_y)
        }
    }

    /// The fire of the mob (`Entity.move()`).
    ///
    /// The reference checks the blocks of the box of the entity at the end
    /// of the movement: a fire or a lava block in the box damages the mob and
    /// sets the counter of the fire, the sound of the fizz is played when the
    /// entity leaves the fire.
    fn mob_fire_phase(&mut self, mob_id: i32) {
        let mut burn = false;
        let mut fizz = false;
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            let box_ = mob.aabb();
            // `Entity.ac()`: the entity is in the water or the position is
            // one where the mobs can live.
            let burning = mob.was_in_water
                || self.world.can_spawn_at(
                    mob.x.floor() as i32,
                    mob.y.floor() as i32,
                    mob.z.floor() as i32,
                );
            if self
                .world
                .has_fire_or_lava_in(&box_.shrunk(0.001, 0.001, 0.001))
            {
                burn = true;
                if !burning {
                    mob.fire_ticks = 300;
                }
            } else if mob.fire_ticks <= 0 {
                mob.fire_ticks = -1;
            }
            if burning && mob.fire_ticks > 0 {
                fizz = true;
                mob.fire_ticks = -1;
            }
        }
        if burn {
            // `Entity.burn(1)`: the damage of the fire.
            self.damage_mob(mob_id, 1, None, false);
        }
        if fizz {
            // The sound of the fizz (`Entity.move()`): the pitch and the
            // volume are two numbers.
            let _ = self.entity_random.next_f32();
            let _ = self.entity_random.next_f32();
        }
    }

    /// The update of the mob before the AI of the mob.
    ///
    /// Java `Entity.R()` and `EntityLiving.R()`: the splash in the water, the
    /// damage of the fire and the lava and the idle sound of the mob draw
    /// from the shared random of the entities. The order of the draws is the
    /// order of the reference: the splash, the damage of the fire, the damage
    /// of the lava, the idle sound.
    ///
    /// The function answers whether the mob left the world: the corpse of a
    /// dead mob lives twenty ticks (`EntityLiving.R()`).
    fn mob_update_phase(&mut self, mob_id: i32) -> bool {
        let Some(mob) = self.entities.get_mob(mob_id) else {
            return false;
        };
        let (x, _y, z) = (mob.x, mob.y, mob.z);
        let aabb = mob.aabb();
        let in_water = crate::entity::physics::in_water(&self.world, &aabb);
        let in_lava = crate::entity::physics::in_lava(&self.world, &aabb);
        let size = mob.size();

        // The last position of the mob (`Entity.m_()`), the reference uses it
        // for the rotation of the head.
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            mob.last_x = x;
            mob.last_z = z;
        }

        // The splash of the mob in the water (`Entity.R()`): the sound and the
        // particles of the splash are drawn when the mob enters the water.
        if in_water {
            let mut splash = false;
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                splash = !mob.was_in_water && !mob.just_created;
                mob.fall_distance = 0.0;
                mob.was_in_water = true;
                mob.fire_ticks = 0;
            }
            if splash {
                // The sound of the splash: two numbers.
                let _ = self.entity_random.next_f32();
                let _ = self.entity_random.next_f32();
                // The bubbles of the splash: three numbers each.
                let particles = 1.0f32 + size.0 * 20.0f32;
                let mut index = 0;
                while (index as f32) < particles {
                    let _ = self.entity_random.next_f32();
                    let _ = self.entity_random.next_f32();
                    let _ = self.entity_random.next_f32();
                    index += 1;
                }
                // The splashes: two numbers each.
                let mut index = 0;
                while (index as f32) < particles {
                    let _ = self.entity_random.next_f32();
                    let _ = self.entity_random.next_f32();
                    index += 1;
                }
            }
        } else if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            mob.was_in_water = false;
        }
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            mob.in_water = in_water;
            mob.in_lava = in_lava;
        }

        // The fire (`Entity.R()`): the mob burns every twentieth tick.
        let mut fire_damage = false;
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            if mob.fire_ticks > 0 {
                if mob.fire_proof() {
                    mob.fire_ticks -= 4;
                    if mob.fire_ticks < 0 {
                        mob.fire_ticks = 0;
                    }
                } else {
                    fire_damage = mob.fire_ticks % 20 == 0;
                    mob.fire_ticks -= 1;
                }
            }
        }
        if fire_damage {
            self.damage_mob(mob_id, 1, None, false);
        }

        // The lava (`Entity.ab()`): four points of damage every tick the mob
        // is in the lava, the mob catches fire.
        if in_lava {
            let mut burning = false;
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                burning = !mob.fire_proof();
            }
            if burning {
                // The reference damages the mob first and sets the counter of
                // the fire afterwards (`Entity.ab()`).
                self.damage_mob(mob_id, 4, None, false);
                if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                    mob.fire_ticks = 600;
                }
            }
        }

        // The void (`Entity.R()`): the mob below the world takes four points
        // of damage (`EntityLiving.Y()`).
        if self.entities.get_mob(mob_id).is_some_and(|m| m.y < -64.0) {
            self.damage_mob(mob_id, 4, None, false);
        }

        // The idle sound of the mob (`EntityLiving.R()`): the reference draws a
        // random number every tick of every living entity.
        {
            let threshold = self
                .entities
                .get_mob(mob_id)
                .map(|mob| mob.idle_sound_ticks)
                .unwrap_or(0);
            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                mob.idle_sound_ticks += 1;
            }
            if self.entity_random.next_i32_bound(1000) < threshold {
                if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                    // The counter of the sound is set to the interval of the
                    // mob (`EntityLiving.R()`).
                    mob.idle_sound_ticks = -mob.idle_sound_interval();
                }
                // The sound of the mob (`EntityLiving.Q()`): the pitch and the
                // volume are two numbers.
                let name = self
                    .entities
                    .get_mob(mob_id)
                    .map(|mob| mob.idle_sound_name(&mut self.entity_random))
                    .unwrap_or(None);
                if name.is_some() {
                    let _ = self.entity_random.next_f32();
                    let _ = self.entity_random.next_f32();
                }
            }
        }

        // The suffocation of the mob (`EntityLiving.R()`): the head of the
        // mob is inside a block, the mob takes one point of damage.
        {
            let head = self.entities.get_mob(mob_id).map(|m| {
                (
                    m.x.floor() as i32,
                    (m.y + m.eye_height()).floor() as i32,
                    m.z.floor() as i32,
                )
            });
            if let Some((hx, hy, hz)) = head {
                if (0..128).contains(&hy) {
                    let head_block = self.world.get_block(hx, hy, hz);
                    if blocks::is_opaque_cube(head_block) {
                        self.damage_mob(mob_id, 1, None, false);
                    }
                }
            }
        }

        // The fire of a mob that is immune to it (`EntityLiving.R()`).
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            if mob.fire_proof() {
                mob.fire_ticks = 0;
            }
        }

        // The drowning of the mob (`EntityLiving.R()`): the air of the mob
        // runs out, the mob makes eight bubbles and takes two points of
        // damage.
        {
            let head = self.entities.get_mob(mob_id).map(|m| {
                (
                    m.x.floor() as i32,
                    (m.y + m.eye_height()).floor() as i32,
                    m.z.floor() as i32,
                    m.mob_type == MobType::Squid,
                )
            });
            if let Some((hx, hy, hz, breathes)) = head {
                if (0..128).contains(&hy) && !breathes {
                    let head_block = self.world.get_block(hx, hy, hz);
                    if blocks::is_water(head_block) {
                        let mut drown = false;
                        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                            mob.air_ticks -= 1;
                            if mob.air_ticks == -20 {
                                mob.air_ticks = 0;
                                drown = true;
                            }
                        }
                        if drown {
                            // The bubbles of the drowning: eight times three
                            // pairs of numbers.
                            for _ in 0..8 {
                                for _ in 0..3 {
                                    let _ = self.entity_random.next_f32();
                                    let _ = self.entity_random.next_f32();
                                }
                            }
                            self.damage_mob(mob_id, 2, None, false);
                            if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                                mob.fire_ticks = 0;
                            }
                        }
                    } else if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                        mob.air_ticks = 300;
                    }
                } else if let Some(mob) = self.entities.get_mob_mut(mob_id) {
                    mob.air_ticks = 300;
                }
            }
        }

        // The counters of the mob (`EntityLiving.R()`).
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            if mob.no_damage_ticks > 0 {
                mob.no_damage_ticks -= 1;
            }
            if mob.attack_ticks > 0 {
                mob.attack_ticks -= 1;
            }
            if mob.pigzombie_anger > 0 {
                mob.pigzombie_anger -= 1;
            }
            mob.just_created = false;
        }

        // The death of the mob (`EntityLiving.R()`): the counter of the death
        // is increased and the mob leaves the world when it passes twenty.
        // The particles of the death are drawn then.
        let mut left_the_world = false;
        if let Some(mob) = self.entities.get_mob_mut(mob_id) {
            if mob.health <= 0 {
                mob.death_ticks += 1;
                if mob.death_ticks > 20 {
                    left_the_world = true;
                }
            }
        }
        if left_the_world {
            // The particles of the death (`EntityLiving.R()`): twenty times
            // three normal values and one number.
            for _ in 0..20 {
                let _ = self.entity_random.next_gaussian();
                let _ = self.entity_random.next_gaussian();
                let _ = self.entity_random.next_gaussian();
                let _ = self.entity_random.next_f32();
            }
            return true;
        }
        false
    }

    fn damage_player_from_mob(
        &mut self,
        player_id: i32,
        damage: i16,
        attacker_x: f64,
        attacker_z: f64,
    ) {
        let mut hit = None;
        let mut woke_player = false;
        let mut armor_sync = None;
        if let Some(p) = self.entities.get_player_mut(player_id) {
            if p.dead || damage < 0 {
                return;
            }
            let (dealt, died) = if damage == 0 {
                if p.no_damage_ticks > 10 {
                    return;
                }
                p.no_damage_ticks = 20;
                p.last_damage = 0;
                (0, false)
            } else {
                let (d, died_flag) = p.take_damage(damage);
                if d <= 0 && p.no_damage_ticks > 10 && damage <= p.last_damage {
                    return;
                }
                (d, died_flag)
            };
            let _ = dealt;
            if p.sleeping {
                p.sleeping = false;
                p.sleep_ticks = 0;
                p.bed_pos = None;
                woke_player = true;
            }
            if damage > 0 {
                armor_sync = Some((
                    p.conn_id,
                    [
                        InventoryPlayer::to_slot_item(p.inventory.armor[3]),
                        InventoryPlayer::to_slot_item(p.inventory.armor[2]),
                        InventoryPlayer::to_slot_item(p.inventory.armor[1]),
                        InventoryPlayer::to_slot_item(p.inventory.armor[0]),
                    ],
                ));
            }
            p.apply_knockback(attacker_x, attacker_z);
            let keep_inv = self.config.mechanics.keep_inventory_on_death;
            let drops = if died && !keep_inv {
                let px = p.x;
                let py = p.y;
                let pz = p.z;
                let mut list: Vec<(f64, f64, f64, ItemStack)> = p
                    .inventory
                    .drop_all_on_death()
                    .into_iter()
                    .map(|it| (px, py, pz, it))
                    .collect();
                if let Some(OpenContainer::Workbench { craft_grid, .. }) = p.open_container.take() {
                    for it in craft_grid.into_iter().flatten() {
                        list.push((px, py, pz, it));
                    }
                }
                list
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

        if woke_player {
            self.broadcast(Clientbound::ArmAnimation(ArmAnimation {
                entity_id: player_id,
                animate: 3,
            }));
        }
        if let Some((cid, slots)) = armor_sync {
            for (i, item) in slots.into_iter().enumerate() {
                self.send(
                    cid,
                    Clientbound::SetSlot(SetSlot {
                        window_id: 0,
                        slot: 5 + i as i16,
                        item,
                    }),
                );
            }
        }

        if let Some((conn_id, username, health, died, mx, my, mz, drops)) = hit {
            // Alert tamed wolves owned by player (`EntityHuman.java:455-495`)
            let nearest_attacker_mob = self
                .entities
                .iter_mobs()
                .filter(|m| {
                    !m.dead && (m.x - attacker_x).abs() < 2.0 && (m.z - attacker_z).abs() < 2.0
                })
                .map(|m| m.entity_id)
                .next();
            if let Some(att_mob_id) = nearest_attacker_mob {
                for w in self.entities.iter_mobs_mut() {
                    if w.mob_type == MobType::Wolf
                        && w.wolf_tamed
                        && w.entity_id != att_mob_id
                        && w.wolf_owner.eq_ignore_ascii_case(&username)
                    {
                        w.wolf_sitting = false;
                        w.target_id = Some(att_mob_id);
                    }
                }
            }

            if self.config.mechanics.player_knockback_fix {
                let vel = Clientbound::EntityVelocity(EntityVelocity {
                    entity_id: player_id,
                    mot_x: mx,
                    mot_y: my,
                    mot_z: mz,
                });
                self.send(conn_id, vel.clone());
                self.broadcast(vel);
            }
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
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(proj) = self.entities.get_projectile(pid) {
            for out in self.tracker.track_projectile_and_scan(proj, &players) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
    }

    fn tick_projectiles(&mut self) {
        let active_dimension = self.active_dimension;
        let proj_ids: Vec<i32> = self
            .entities
            .iter_projectiles()
            .filter(|p| self.entities.dimension_of(p.entity_id) == active_dimension)
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

            if proj_snapshot.kind == ProjectileKind::FishingHook {
                let valid_owner = proj_snapshot
                    .shooter_id
                    .and_then(|sid| self.entities.get_player(sid))
                    .is_some_and(|p| {
                        !p.dead
                            && p.inventory
                                .item_in_hand()
                                .is_some_and(|s| s.id == items::FISHING_ROD)
                            && ((p.x - proj_snapshot.x).powi(2)
                                + (p.y - proj_snapshot.y).powi(2)
                                + (p.z - proj_snapshot.z).powi(2))
                                <= 1024.0
                    });
                if !valid_owner {
                    to_remove.push(pid);
                    continue;
                }
                if let Some(target_eid) = proj_snapshot.hooked_entity {
                    let hooked_pos = self
                        .entities
                        .get_mob(target_eid)
                        .filter(|m| !m.dead)
                        .map(|m| (m.x, m.y + (m.size().1 as f64) * 0.8, m.z))
                        .or_else(|| {
                            self.entities
                                .get_player(target_eid)
                                .filter(|p| !p.dead)
                                .map(|p| (p.x, p.y + 1.4, p.z))
                        });
                    if let Some((hx, hy, hz)) = hooked_pos {
                        if let Some(p) = self.entities.get_projectile_mut(pid) {
                            p.x = hx;
                            p.y = hy;
                            p.z = hz;
                            p.mot_x = 0.0;
                            p.mot_y = 0.0;
                            p.mot_z = 0.0;
                        }
                        continue;
                    } else if let Some(p) = self.entities.get_projectile_mut(pid) {
                        p.hooked_entity = None;
                    }
                }
            }

            if proj_snapshot.in_ground {
                let gbx = proj_snapshot.x.floor() as i32;
                let gby = proj_snapshot.y.floor() as i32;
                let gbz = proj_snapshot.z.floor() as i32;
                if (0..128).contains(&gby) && !blocks::is_solid(self.world.get_block(gbx, gby, gbz))
                {
                    if let Some(p) = self.entities.get_projectile_mut(pid) {
                        p.in_ground = false;
                        p.ticks_in_ground = 0;
                        p.ticks_in_air = 0;
                    }
                } else {
                    if proj_snapshot.kind == ProjectileKind::Arrow && proj_snapshot.from_player {
                        // Check if a player walks over the stuck arrow to pick it up
                        let allow_others = self.config.mechanics.arrows_pickup_by_others;
                        let collector = self.entities.iter_players().find_map(|p| {
                            if !p.dead
                                && p.dimension == self.active_dimension
                                && (allow_others || proj_snapshot.shooter_id == Some(p.entity_id))
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
                if proj_snapshot.kind == ProjectileKind::FishingHook {
                    if let Some(p) = self.entities.get_projectile_mut(pid) {
                        p.hooked_entity = Some(mob_id);
                    }
                } else {
                    to_remove.push(pid);
                    continue;
                }
            }

            let hit_player = self.entities.iter_players().find_map(|p| {
                if p.dead
                    || p.dimension != self.active_dimension
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
                if !self.config.server.pvp && proj_snapshot.from_player {
                    // PvP disabled: player-fired arrows/snowballs/rods don't hit other players
                } else {
                    let dmg = match proj_snapshot.kind {
                        ProjectileKind::Arrow => 4,
                        ProjectileKind::Fireball => 5,
                        _ => 0,
                    };
                    player_hits.push((player_id, dmg, proj_snapshot.x, proj_snapshot.z));
                    if proj_snapshot.kind == ProjectileKind::Egg {
                        egg_hatches.push((next_x, next_y, next_z));
                    } else if proj_snapshot.kind == ProjectileKind::Fireball {
                        fireball_explosions.push((next_x, next_y, next_z));
                    }
                    if proj_snapshot.kind == ProjectileKind::FishingHook {
                        if let Some(p) = self.entities.get_projectile_mut(pid) {
                            p.hooked_entity = Some(player_id);
                        }
                    } else {
                        to_remove.push(pid);
                        continue;
                    }
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
                self.send_tracked(out.recipient_id, out.packet);
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
            let allow_blocks = self.config.mechanics.ghast_block_damage;
            self.create_explosion_ex(fx, fy, fz, 1.0, true, allow_blocks, None);
        }
    }

    fn spawn_primed_tnt_at(&mut self, x: f64, y: f64, z: f64, fuse: i32) {
        let tid = self.entities.spawn_primed_tnt(x, y, z, fuse);
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(tnt) = self.entities.get_primed_tnt(tid) {
            for out in self.tracker.track_tnt_and_scan(tnt, &players) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
    }

    fn tick_primed_tnts(&mut self) {
        let active_dimension = self.active_dimension;
        let tnt_ids: Vec<i32> = self
            .entities
            .iter_primed_tnts()
            .filter(|t| self.entities.dimension_of(t.entity_id) == active_dimension)
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
                self.send_tracked(out.recipient_id, out.packet);
            }
            let allow_blocks = self.config.mechanics.tnt_block_damage;
            self.create_explosion_ex(tx, ty, tz, 4.0, false, allow_blocks, None);
        }
    }

    fn spawn_minecart_at(&mut self, cart_type: u8, x: f64, y: f64, z: f64) {
        let cid = self.entities.spawn_minecart(cart_type, x, y, z);
        if let Some(cart) = self.entities.get_minecart_mut(cid) {
            cart.max_speed = self.config.mechanics.minecart_max_speed;
            cart.slow_when_empty = self.config.mechanics.minecart_slow_when_empty;
        }
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(cart) = self.entities.get_minecart(cid) {
            for out in self.tracker.track_minecart_and_scan(cart, &players) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
    }

    fn handle_interact_minecart(&mut self, conn_id: ConnId, player_id: i32, cart_id: i32) {
        let Some((ctype, cur_rider, cargo)) = self
            .entities
            .get_minecart(cart_id)
            .filter(|c| !c.dead)
            .map(|c| (c.cart_type, c.passenger_id, c.cargo))
        else {
            return;
        };
        if ctype == 0 {
            if cur_rider == Some(player_id) {
                self.dismount_player(player_id);
            } else if !cur_rider.is_some_and(|rid| self.entities.get_player(rid).is_some()) {
                if let Some(mob_id) = cur_rider {
                    if let Some(m) = self.entities.get_mob_mut(mob_id) {
                        m.vehicle_id = None;
                    }
                    self.broadcast(Clientbound::AttachEntity(AttachEntity {
                        entity_id: mob_id,
                        vehicle_id: -1,
                    }));
                }
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
        } else if ctype == 1 {
            // StorageMinecart (`EntityMinecart.java:862-865` + Poseidon dupe fix `NetServerHandler.java:1037-1039`)
            let prevent_vehicle_open = self.config.fixes.prevent_vehicle_chest_open;
            let open_info = if let Some(player) = self.entities.get_by_conn_mut(conn_id) {
                if prevent_vehicle_open && player.vehicle_id.is_some() {
                    return;
                }
                let win_id = player.next_window_id();
                let container = OpenContainer::MinecartChest {
                    window_id: win_id,
                    entity_id: cart_id,
                    slots: cargo,
                };
                let win_items = player.inventory.container_window_items(&container);
                player.open_container = Some(container);
                Some((win_id, win_items))
            } else {
                None
            };
            if let Some((win_id, win_items)) = open_info {
                self.send(
                    conn_id,
                    Clientbound::OpenWindow(OpenWindow {
                        window_id: win_id,
                        inventory_type: 0,
                        title: "Minecart".to_string(),
                        slots_count: 27,
                    }),
                );
                self.send(
                    conn_id,
                    Clientbound::WindowItems(WindowItems {
                        window_id: win_id,
                        items: win_items,
                    }),
                );
            }
        } else if ctype == 2 {
            // Powered minecart (`EntityMinecart.java:866-879`):
            // 1. If holding COAL (263), consume 1 and add 1200 fuel.
            // 2. Always set push vector `f = locX - player.locX; g = locZ - player.locZ`.
            let mut slot_update = None;
            let mut consumed_coal = false;
            let mut player_xz = None;
            if let Some(player) = self.entities.get_by_conn_mut(conn_id) {
                player_xz = Some((player.x, player.z));
                if let Some(mut stack) = player.inventory.item_in_hand() {
                    if stack.id == items::COAL {
                        stack.count = stack.count.saturating_sub(1);
                        player.inventory.set_item_in_hand(if stack.count > 0 {
                            Some(stack)
                        } else {
                            None
                        });
                        let slot_idx = 36 + player.inventory.held_slot as i16;
                        let item = InventoryPlayer::to_slot_item(player.inventory.item_in_hand());
                        slot_update = Some((slot_idx, item));
                        consumed_coal = true;
                    }
                }
            }
            if let Some((px, pz)) = player_xz {
                if let Some(c) = self.entities.get_minecart_mut(cart_id) {
                    if consumed_coal {
                        c.fuel += 1200;
                    }
                    c.push_x = c.x - px;
                    c.push_z = c.z - pz;
                }
            }
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

    fn damage_minecart(&mut self, cart_id: i32, raw_damage: i32) {
        let mut destroyed = None;
        if let Some(cart) = self.entities.get_minecart_mut(cart_id) {
            if cart.dead {
                return;
            }
            cart.rock_direction = -cart.rock_direction;
            cart.time_since_hit = 10;
            cart.damage += raw_damage * 10;
            if cart.damage > 40 {
                cart.dead = true;
                destroyed = Some((
                    cart.cart_type,
                    cart.x,
                    cart.y,
                    cart.z,
                    cart.passenger_id,
                    cart.cargo,
                ));
            }
        }
        if let Some((ctype, cx, cy, cz, passenger, cargo)) = destroyed {
            if let Some(pid) = passenger {
                if self.entities.get_player(pid).is_some() {
                    self.dismount_player(pid);
                } else if let Some(m) = self.entities.get_mob_mut(pid) {
                    m.vehicle_id = None;
                    self.broadcast(Clientbound::AttachEntity(AttachEntity {
                        entity_id: pid,
                        vehicle_id: -1,
                    }));
                }
            }
            let _ = self.entities.remove_minecart(cart_id);
            for out in self.tracker.untrack_minecart(cart_id) {
                self.send_tracked(out.recipient_id, out.packet);
            }
            self.spawn_dropped_item_random(cx, cy, cz, ItemStack::new(items::MINECART, 1, 0), 10);
            if ctype == 1 {
                // EntityMinecart.java:134-160 — split each stack into random sub-stacks of 10..=30
                for mut item in cargo.into_iter().flatten() {
                    while item.count > 0 {
                        let split = (self.world.tick_random.next_i32_bound(21) + 10)
                            .min(item.count as i32) as u8;
                        item.count -= split;
                        self.spawn_dropped_item_random(
                            cx,
                            cy,
                            cz,
                            ItemStack::new(item.id, split, item.damage),
                            10,
                        );
                    }
                }
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

    #[allow(clippy::type_complexity)]
    fn tick_minecarts(&mut self) {
        let active_dimension = self.active_dimension;
        self.world.active_detector_rails.clear();
        let cart_ids: Vec<i32> = self
            .entities
            .iter_minecarts()
            .filter(|c| self.entities.dimension_of(c.entity_id) == active_dimension)
            .map(|c| c.entity_id)
            .collect();
        if cart_ids.is_empty() {
            return;
        }

        let mut void_carts = Vec::new();
        let mut passenger_fall_damages = Vec::new();

        for &cid in &cart_ids {
            // EntityMinecart.java:576-579 — clear dead passenger before/after tick
            let dead_passenger = self
                .entities
                .get_minecart(cid)
                .and_then(|c| c.passenger_id)
                .is_some_and(|pid| {
                    let p_alive = self.entities.get_player(pid).is_some_and(|p| !p.dead);
                    let m_alive = self.entities.get_mob(pid).is_some_and(|m| !m.dead);
                    !p_alive && !m_alive
                });

            let mut rider_sync = None;
            if let Some(cart) = self.entities.get_minecart_mut(cid) {
                if dead_passenger {
                    cart.passenger_id = None;
                }
                let fall_dmg = self.world.tick_single_minecart(cart);
                if cart.dead {
                    void_carts.push((cid, cart.passenger_id));
                    continue;
                }
                if let Some(pid) = cart.passenger_id {
                    if let Some(dmg) = fall_dmg {
                        passenger_fall_damages.push((pid, dmg));
                    }
                    // Entity.f() + EntityMinecart.m() (`Entity.java:1154`, `EntityMinecart.java:90-92`)
                    rider_sync = Some((pid, cart.x, cart.y + cart.mounted_y_offset(), cart.z));
                }
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
                } else if let Some(rmob) = self.entities.get_mob_mut(pid) {
                    rmob.x = rx;
                    rmob.y = ry;
                    rmob.z = rz;
                }
                if let Some(cid) = conn_to_update {
                    self.update_player_view_chunks(cid);
                }
            }
        }

        // EntityMinecart.collide (`EntityMinecart.java:564-575, 703-794`, `EntityLiving.java:710-720`)
        let player_boxes: Vec<(i32, f64, f64, f64, Option<i32>)> = self
            .entities
            .iter_players()
            .filter(|p| !p.dead && p.dimension == self.active_dimension)
            .map(|p| (p.entity_id, p.x, p.y, p.z, p.vehicle_id))
            .collect();

        let mob_boxes: Vec<(i32, f64, f64, f64, f32, f32, Option<i32>, Option<i32>)> = self
            .entities
            .iter_mobs()
            .filter(|m| !m.dead && self.entities.dimension_of(m.entity_id) == self.active_dimension)
            .map(|m| {
                let (w, h) = m.size();
                (
                    m.entity_id,
                    m.x,
                    m.y,
                    m.z,
                    w,
                    h,
                    m.vehicle_id,
                    m.passenger_id,
                )
            })
            .collect();

        let mut mob_mounts = Vec::new();
        let mut mob_pushes = Vec::new();

        for i in 0..cart_ids.len() {
            let cid = cart_ids[i];
            let Some((cx, cy, cz, c_mx, c_mz, c_type, c_pass, bb)) = self
                .entities
                .get_minecart(cid)
                .filter(|c| !c.dead)
                .map(|c| {
                    (
                        c.x,
                        c.y,
                        c.z,
                        c.mot_x,
                        c.mot_z,
                        c.cart_type,
                        c.passenger_id,
                        c.bounding_box(),
                    )
                })
            else {
                continue;
            };
            let _ = cy;
            let exp_a = bb.0 - 0.20000000298023224;
            let exp_b = bb.1;
            let exp_c = bb.2 - 0.20000000298023224;
            let exp_d = bb.3 + 0.20000000298023224;
            let exp_e = bb.4;
            let exp_f = bb.5 + 0.20000000298023224;

            // 1. Collide with players (`EntityLiving.java:717` -> `EntityMinecart.collide`)
            for &(pid, px, py, pz, p_veh) in &player_boxes {
                if Some(pid) == c_pass || p_veh == Some(cid) {
                    continue;
                }
                let overlaps = px + 0.3 > exp_a
                    && px - 0.3 < exp_d
                    && py + 1.8 > exp_b
                    && py < exp_e
                    && pz + 0.3 > exp_c
                    && pz - 0.3 < exp_f;
                if overlaps {
                    let mut d0 = px - cx;
                    let mut d1 = pz - cz;
                    let mut d2 = d0 * d0 + d1 * d1;
                    if d2 >= 9.999999747378752e-5 {
                        d2 = (d2.sqrt() as f32) as f64;
                        d0 /= d2;
                        d1 /= d2;
                        let d3 = (1.0 / d2).min(1.0);
                        d0 = d0 * d3 * 0.10000000149011612 * 0.5;
                        d1 = d1 * d3 * 0.10000000149011612 * 0.5;
                        if let Some(c) = self.entities.get_minecart_mut(cid) {
                            c.mot_x -= d0;
                            c.mot_z -= d1;
                        }
                    }
                }
            }

            // 2. Collide with mobs (`EntityMinecart.java:717-790`)
            let mut cur_pass = c_pass;
            for &(mid, mx, my, mz, mw, mh, m_veh, m_pass) in &mob_boxes {
                if Some(mid) == cur_pass || m_veh.is_some() || m_pass.is_some() {
                    continue;
                }
                let hw = (mw / 2.0) as f64;
                let overlaps = mx + hw > exp_a
                    && mx - hw < exp_d
                    && my + (mh as f64) > exp_b
                    && my < exp_e
                    && mz + hw > exp_c
                    && mz - hw < exp_f;
                if overlaps {
                    if c_type == 0 && c_mx * c_mx + c_mz * c_mz > 0.01 && cur_pass.is_none() {
                        cur_pass = Some(mid);
                        mob_mounts.push((cid, mid));
                    }
                    let mut d0 = mx - cx;
                    let mut d1 = mz - cz;
                    let mut d2 = d0 * d0 + d1 * d1;
                    if d2 >= 9.999999747378752e-5 {
                        d2 = (d2.sqrt() as f32) as f64;
                        d0 /= d2;
                        d1 /= d2;
                        let d3 = (1.0 / d2).min(1.0);
                        d0 = d0 * d3 * 0.10000000149011612 * 0.5;
                        d1 = d1 * d3 * 0.10000000149011612 * 0.5;
                        if let Some(c) = self.entities.get_minecart_mut(cid) {
                            c.mot_x -= d0;
                            c.mot_z -= d1;
                        }
                        mob_pushes.push((mid, d0 / 4.0, d1 / 4.0));
                    }
                }
            }

            // 3. Collide with other minecarts (`EntityMinecart.java:570-572, 752-786`)
            for &other_id in &cart_ids[(i + 1)..] {
                let Some((ox, oz, o_last_x, o_mx, o_mz, o_type, obb)) = self
                    .entities
                    .get_minecart(other_id)
                    .filter(|c| !c.dead)
                    .map(|c| {
                        (
                            c.x,
                            c.z,
                            c.last_x,
                            c.mot_x,
                            c.mot_z,
                            c.cart_type,
                            c.bounding_box(),
                        )
                    })
                else {
                    continue;
                };
                let overlaps = obb.3 > exp_a
                    && obb.0 < exp_d
                    && obb.4 > exp_b
                    && obb.1 < exp_e
                    && obb.5 > exp_c
                    && obb.2 < exp_f;
                if !overlaps {
                    continue;
                }
                let (cur_mx, cur_mz) = self
                    .entities
                    .get_minecart(cid)
                    .map(|c| (c.mot_x, c.mot_z))
                    .unwrap_or((c_mx, c_mz));
                let mut d0 = ox - cx;
                let mut d1 = oz - cz;
                let mut d2 = d0 * d0 + d1 * d1;
                if d2 < 9.999999747378752e-5 {
                    continue;
                }
                d2 = (d2.sqrt() as f32) as f64;
                d0 /= d2;
                d1 /= d2;
                let d3 = (1.0 / d2).min(1.0);
                d0 = d0 * d3 * 0.10000000149011612 * 0.5;
                d1 = d1 * d3 * 0.10000000149011612 * 0.5;

                if !self.config.mechanics.allow_minecart_boosters {
                    let d4 = ox - cx;
                    let d5 = oz - cz;
                    let d6 = d4 * o_mz + d5 * o_last_x;
                    if d6 * d6 > 5.0 {
                        continue;
                    }
                }
                let mut d7 = o_mx + cur_mx;
                let mut d8 = o_mz + cur_mz;

                let (new_c_mx, new_c_mz, new_o_mx, new_o_mz) = if o_type == 2 && c_type != 2 {
                    (
                        cur_mx * 0.20000000298023224 + o_mx - d0,
                        cur_mz * 0.20000000298023224 + o_mz - d1,
                        o_mx * 0.699999988079071,
                        o_mz * 0.699999988079071,
                    )
                } else if o_type != 2 && c_type == 2 {
                    (
                        cur_mx * 0.699999988079071,
                        cur_mz * 0.699999988079071,
                        o_mx * 0.20000000298023224 + cur_mx + d0,
                        o_mz * 0.20000000298023224 + cur_mz + d1,
                    )
                } else {
                    d7 /= 2.0;
                    d8 /= 2.0;
                    (
                        cur_mx * 0.20000000298023224 + d7 - d0,
                        cur_mz * 0.20000000298023224 + d8 - d1,
                        o_mx * 0.20000000298023224 + d7 + d0,
                        o_mz * 0.20000000298023224 + d8 + d1,
                    )
                };

                if let Some(c) = self.entities.get_minecart_mut(cid) {
                    c.mot_x = new_c_mx;
                    c.mot_z = new_c_mz;
                }
                if let Some(o) = self.entities.get_minecart_mut(other_id) {
                    o.mot_x = new_o_mx;
                    o.mot_z = new_o_mz;
                }
            }
        }

        for (cid, mid) in mob_mounts {
            if let Some(c) = self.entities.get_minecart_mut(cid) {
                if c.passenger_id.is_none() {
                    c.passenger_id = Some(mid);
                }
            }
            if let Some(m) = self.entities.get_mob_mut(mid) {
                m.vehicle_id = Some(cid);
            }
            self.broadcast(Clientbound::AttachEntity(AttachEntity {
                entity_id: mid,
                vehicle_id: cid,
            }));
        }

        for (mid, vx, vz) in mob_pushes {
            if let Some(m) = self.entities.get_mob_mut(mid) {
                m.mot_x += vx;
                m.mot_z += vz;
            }
        }

        for (pid, dmg) in passenger_fall_damages {
            if self.entities.get_player(pid).is_some() {
                self.damage_player_direct(pid, dmg, "fell from a high place");
            } else {
                let _ = self.damage_mob(pid, dmg, None, false);
            }
        }

        for (cid, passenger) in void_carts {
            if let Some(pid) = passenger {
                if self.entities.get_player(pid).is_some() {
                    self.dismount_player(pid);
                } else if let Some(m) = self.entities.get_mob_mut(pid) {
                    m.vehicle_id = None;
                }
            }
            let _ = self.entities.remove_minecart(cid);
            for out in self.tracker.untrack_minecart(cid) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
    }

    fn painting_can_survive(
        &self,
        x: i32,
        y: i32,
        z: i32,
        direction: i32,
        title: &str,
        ignore_id: Option<i32>,
    ) -> bool {
        let (aabb, wall_blocks) = Painting::bounds_and_wall_blocks(x, y, z, direction, title);
        for (wx, wy, wz) in wall_blocks {
            if !(0..128).contains(&wy) {
                return false;
            }
            if !blocks::material(self.world.get_block(wx, wy, wz)).is_buildable() {
                return false;
            }
        }
        for other in self.entities.iter_paintings() {
            if other.dead || Some(other.entity_id) == ignore_id {
                continue;
            }
            let (oa, _) = Painting::bounds_and_wall_blocks(
                other.x,
                other.y,
                other.z,
                other.direction,
                &other.title,
            );
            let overlaps = aabb.0 < oa.3
                && aabb.3 > oa.0
                && aabb.1 < oa.4
                && aabb.4 > oa.1
                && aabb.2 < oa.5
                && aabb.5 > oa.2;
            if overlaps {
                return false;
            }
        }
        true
    }

    fn tick_paintings(&mut self) {
        let active_dimension = self.active_dimension;
        let mut broken = Vec::new();
        for p in self.entities.iter_paintings() {
            if p.dead || self.entities.dimension_of(p.entity_id) != active_dimension {
                continue;
            }
            if !self.world.is_chunk_loaded(p.chunk_pos()) {
                continue;
            }
            if !self.painting_can_survive(p.x, p.y, p.z, p.direction, &p.title, Some(p.entity_id)) {
                broken.push((p.entity_id, p.x, p.y, p.z));
            }
        }
        for (pid, x, y, z) in broken {
            if self.entities.remove_painting(pid).is_some() {
                for outbound in self.tracker.untrack_painting(pid) {
                    self.send_tracked(outbound.recipient_id, outbound.packet);
                }
                self.spawn_dropped_item_random(
                    x as f64 + 0.5,
                    y as f64 + 0.5,
                    z as f64 + 0.5,
                    ItemStack::new(items::PAINTING, 1, 0),
                    10,
                );
            }
        }
    }

    fn spawn_painting_at(&mut self, title: String, x: i32, y: i32, z: i32, direction: i32) {
        let pid = self.entities.spawn_painting(title, x, y, z, direction);
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(p) = self.entities.get_painting(pid) {
            for out in self.tracker.track_painting_and_scan(p, &players) {
                self.send_tracked(out.recipient_id, out.packet);
            }
        }
    }

    fn tick_pressure_plates_and_beds(&mut self) {
        let active_dimension = self.active_dimension;
        let mut plate_positions = Vec::new();
        for p in self.entities.iter_players() {
            if p.dead || p.dimension != active_dimension {
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
            if m.dead || self.entities.dimension_of(m.entity_id) != active_dimension {
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
            if it.dead || self.entities.dimension_of(it.entity_id) != active_dimension {
                continue;
            }
            let bx = it.x.floor() as i32;
            let by = it.y.floor() as i32;
            let bz = it.z.floor() as i32;
            if self.world.get_block(bx, by, bz) == blocks::WOOD_PLATE {
                plate_positions.push((bx, by, bz, blocks::WOOD_PLATE));
            }
        }
        for pr in self.entities.iter_projectiles() {
            if pr.dead || self.entities.dimension_of(pr.entity_id) != active_dimension {
                continue;
            }
            let bx = pr.x.floor() as i32;
            let by = pr.y.floor() as i32;
            let bz = pr.z.floor() as i32;
            if self.world.get_block(bx, by, bz) == blocks::WOOD_PLATE {
                plate_positions.push((bx, by, bz, blocks::WOOD_PLATE));
            }
        }
        for bt in self.entities.iter_boats() {
            if self.entities.dimension_of(bt.entity_id) != active_dimension {
                continue;
            }
            let bx = bt.x.floor() as i32;
            let by = bt.y.floor() as i32;
            let bz = bt.z.floor() as i32;
            if self.world.get_block(bx, by, bz) == blocks::WOOD_PLATE {
                plate_positions.push((bx, by, bz, blocks::WOOD_PLATE));
            }
        }
        for ct in self.entities.iter_minecarts() {
            if ct.dead || self.entities.dimension_of(ct.entity_id) != active_dimension {
                continue;
            }
            let bx = ct.x.floor() as i32;
            let by = (ct.y - 0.35).floor() as i32;
            let bz = ct.z.floor() as i32;
            let bid = self.world.get_block(bx, by, bz);
            if bid == blocks::WOOD_PLATE
                || (bid == blocks::STONE_PLATE && ct.passenger_id.is_some())
            {
                plate_positions.push((bx, by, bz, bid));
            }
        }

        self.world.active_pressure_plates.clear();
        for (px, py, pz, pid) in plate_positions {
            self.world.active_pressure_plates.insert((px, py, pz));
            self.world.set_pressure_plate_state(px, py, pz, pid, true);
        }

        // Beds only exist in the overworld; skip the sleep/wake logic while
        // the Nether world is the active one.
        if self.active_dimension != 0 {
            return;
        }

        // Wake players whose bed block was destroyed (`EntityHuman.java:88-93`)
        let mut broken_bed_wakes = Vec::new();
        for p in self.entities.iter_players() {
            if !p.dead && p.sleeping {
                if let Some((bx, by, bz)) = p.bed_pos {
                    if self.world.get_block(bx, by, bz) != blocks::BED {
                        broken_bed_wakes.push(p.entity_id);
                    }
                }
            }
        }
        for eid in broken_bed_wakes {
            if let Some(p) = self.entities.get_player_mut(eid) {
                p.sleeping = false;
                p.sleep_ticks = 0;
                p.bed_pos = None;
            }
            self.broadcast(Clientbound::ArmAnimation(ArmAnimation {
                entity_id: eid,
                animate: 3,
            }));
        }

        // Bed sleep check (`World.everyoneDeeplySleeping()`)
        for p in self.entities.iter_players_mut() {
            if !p.dead && p.sleeping && p.sleep_ticks < 100 {
                p.sleep_ticks += 1;
            }
        }
        let mut overworld_players = 0usize;
        let mut all_sleeping = true;
        for p in self.entities.iter_players().filter(|p| p.dimension == 0) {
            overworld_players += 1;
            if p.dead || !p.sleeping || p.sleep_ticks < 1 {
                all_sleeping = false;
            }
        }
        let all_sleeping = overworld_players > 0 && all_sleeping;
        if all_sleeping {
            let cur_time = self.world.world_time();
            let next_morning = (cur_time + 24000) - (cur_time + 24000).rem_euclid(24000);
            self.world.live_time = next_morning;
            self.world.level_data.borrow_mut().time = next_morning;
            if self.world.reset_weather() {
                self.broadcast(Clientbound::Bed(Bed { reason: 2 }));
            }

            let mut woke_players = Vec::new();
            for p in self.entities.iter_players_mut() {
                p.sleeping = false;
                p.sleep_ticks = 0;
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

    pub fn strike_lightning(&mut self, x: i32, y: i32, z: i32) {
        self.world.strike_lightning_at(x, y, z);
        self.process_world_events();
    }

    fn handle_lightning_strike(&mut self, x: i32, y: i32, z: i32) {
        let bolt_id = self.entities.allocate_id();
        self.broadcast(Clientbound::Weather(Weather {
            entity_id: bolt_id,
            weather_type: 1,
            x: x * 32,
            y: y * 32,
            z: z * 32,
        }));

        // EntityWeatherStorm.java:111-120 — affect entities in ±3.0 box (`y - 3.0..=y + 9.0`)
        let fx = x as f64;
        let fy = y as f64;
        let fz = z as f64;

        let hit_players: Vec<i32> = self
            .entities
            .iter_players()
            .filter(|p| {
                !p.dead
                    && p.dimension == self.active_dimension
                    && (p.x - fx).abs() <= 3.0
                    && p.y >= fy - 3.0
                    && p.y <= fy + 9.0
                    && (p.z - fz).abs() <= 3.0
            })
            .map(|p| p.entity_id)
            .collect();
        for pid in hit_players {
            if let Some(p) = self.entities.get_player_mut(pid) {
                p.fire_ticks = p.fire_ticks.max(160);
            }
            self.damage_player_direct(pid, 5, "was struck by lightning");
        }

        let hit_mobs: Vec<(i32, MobType, f64, f64, f64, f32, f32)> = self
            .entities
            .iter_mobs()
            .filter(|m| {
                !m.dead
                    && (m.x - fx).abs() <= 3.0
                    && m.y >= fy - 3.0
                    && m.y <= fy + 9.0
                    && (m.z - fz).abs() <= 3.0
            })
            .map(|m| (m.entity_id, m.mob_type, m.x, m.y, m.z, m.yaw, m.pitch))
            .collect();

        for (mid, mtype, mx, my, mz, myaw, mpitch) in hit_mobs {
            match mtype {
                MobType::Creeper => {
                    // EntityCreeper.java:151-163 — power the creeper!
                    if let Some(m) = self.entities.get_mob_mut(mid) {
                        m.creeper_powered = true;
                        m.fire_ticks = m.fire_ticks.max(160);
                    }
                    let _ = self.damage_mob(mid, 5, None, false);
                }
                MobType::Pig => {
                    // EntityPig.java:67-85 — transform Pig into PigZombie!
                    let _ = self.entities.remove_mob(mid);
                    for out in self.tracker.untrack_mob(mid) {
                        self.send_tracked(out.recipient_id, out.packet);
                    }
                    let pz_id = self.spawn_mob_at(MobType::PigZombie, mx, my, mz);
                    if let Some(pz) = self.entities.get_mob_mut(pz_id) {
                        pz.yaw = myaw;
                        pz.pitch = mpitch;
                    }
                }
                _ => {
                    if let Some(m) = self.entities.get_mob_mut(mid) {
                        if !m.fire_proof() {
                            m.fire_ticks = m.fire_ticks.max(160);
                        }
                    }
                    let _ = self.damage_mob(mid, 5, None, false);
                }
            }
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
                    let active_dimension = self.active_dimension;
                    let players: Vec<_> = self
                        .entities
                        .iter_players()
                        .filter(|p| p.dimension == active_dimension)
                        .cloned()
                        .collect();
                    if let Some(dropped) = self.entities.get_dropped_item(item_id) {
                        for out in self.tracker.track_item_and_scan(dropped, &players) {
                            self.send_tracked(out.recipient_id, out.packet);
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
                WorldEvent::LightningStrike { x, y, z } => {
                    self.handle_lightning_strike(x, y, z);
                }
            }
        }
    }

    /// Java `Explosion.java` — 16x16x16 raycast block destruction, entity damage/knockback, and `Packet60Explosion`.
    pub fn create_explosion(&mut self, ex: f64, ey: f64, ez: f64, radius: f32, set_fire: bool) {
        self.create_explosion_ex(ex, ey, ez, radius, set_fire, true, None);
    }

    /// The explosion of the reference (`Explosion.java`).
    ///
    /// The source of the explosion is not damaged by it: the list of the
    /// entities of the blast skips the source (`World.b(Entity,
    /// AxisAlignedBB)`).
    #[allow(clippy::too_many_arguments)]
    pub fn create_explosion_ex(
        &mut self,
        ex: f64,
        ey: f64,
        ez: f64,
        radius: f32,
        set_fire: bool,
        allow_block_damage: bool,
        source_mob: Option<i32>,
    ) {
        let mut destroyed_blocks = HashSet::new();
        let mut rng = std::mem::replace(&mut self.world.tick_random, JavaRandom::new(0));
        let step_size = if self.config.mechanics.optimized_explosions {
            2
        } else {
            1
        };

        if allow_block_damage {
            for i in (0..16).step_by(step_size) {
                for j in (0..16).step_by(step_size) {
                    for k in (0..16).step_by(step_size) {
                        if i == 0
                            || i >= 16 - step_size
                            || j == 0
                            || j >= 16 - step_size
                            || k == 0
                            || k >= 16 - step_size
                        {
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
        }

        // Entity damage within `radius * 2.0` with block exposure raycasting (`Explosion.java:127-150`)
        let blast_size = (radius * 2.0) as f64;
        let player_ids: Vec<i32> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == self.active_dimension)
            .map(|p| p.entity_id)
            .collect();
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
                    Some((d, p.x, p.y, p.z))
                } else {
                    None
                }
            });
            if let Some((d, px, py, pz)) = dist_info {
                let r1 = self
                    .world
                    .ray_trace((ex, ey, ez), (px, py + 0.2, pz), false)
                    .is_none();
                let r2 = self
                    .world
                    .ray_trace((ex, ey, ez), (px, py + 1.5, pz), false)
                    .is_none();
                let exposure = (i32::from(r1) + i32::from(r2)) as f64 / 2.0;
                if exposure > 0.0 {
                    let d7 = d / blast_size;
                    let impact = (1.0 - d7) * exposure;
                    let dmg =
                        (((impact * impact + impact) / 2.0 * 8.0 * blast_size + 1.0) as i16).max(1);
                    let prev_kb = self.config.mechanics.player_knockback_fix;
                    self.config.mechanics.player_knockback_fix =
                        self.config.mechanics.send_explosion_velocity;
                    self.damage_player_from_mob(pid, dmg, ex, ez);
                    self.config.mechanics.player_knockback_fix = prev_kb;
                }
            }
        }

        let mob_ids: Vec<i32> = self.entities.iter_mobs().map(|m| m.entity_id).collect();
        for mid in mob_ids {
            if source_mob == Some(mid) {
                continue;
            }
            let dist_info = self.entities.get_mob(mid).and_then(|m| {
                if m.dead {
                    return None;
                }
                let dx = m.x - ex;
                let dy = m.y - ey;
                let dz = m.z - ez;
                let d = (dx * dx + dy * dy + dz * dz).sqrt();
                if d <= blast_size {
                    Some((d, m.x, m.y, m.z, m.eye_height()))
                } else {
                    None
                }
            });
            if let Some((d, mx, my, mz, eye_h)) = dist_info {
                let r1 = self
                    .world
                    .ray_trace((ex, ey, ez), (mx, my + 0.2, mz), false)
                    .is_none();
                let r2 = self
                    .world
                    .ray_trace((ex, ey, ez), (mx, my + eye_h, mz), false)
                    .is_none();
                let exposure = (i32::from(r1) + i32::from(r2)) as f64 / 2.0;
                if exposure > 0.0 {
                    let impact = (1.0 - (d / blast_size)) * exposure;
                    let dmg =
                        (((impact * impact + impact) / 2.0 * 8.0 * blast_size + 1.0) as i16).max(1);
                    let _ = self.damage_mob(mid, dmg, Some((0, ex, ez)), false);
                }
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
            if p.dimension == self.active_dimension && p.sent_chunks.contains(&cpos) {
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
            OpenContainer::MinecartChest {
                entity_id, slots, ..
            } => {
                if let Some(cart) = self.entities.get_minecart_mut(*entity_id) {
                    cart.cargo = *slots;
                }
            }
            OpenContainer::Workbench { .. } => {}
        }
    }

    #[allow(clippy::type_complexity)]
    fn sync_open_containers(&mut self) {
        let active_dimension = self.active_dimension;
        let mut updates = Vec::new();
        let mut force_closes = Vec::new();
        let cart_snapshots: HashMap<i32, (f64, f64, f64, [Option<ItemStack>; 27])> = self
            .entities
            .iter_minecarts()
            .filter(|c| {
                !c.dead
                    && c.cart_type == 1
                    && self.entities.dimension_of(c.entity_id) == active_dimension
            })
            .map(|c| (c.entity_id, (c.x, c.y, c.z, c.cargo)))
            .collect();

        for player in self
            .entities
            .iter_players_mut()
            .filter(|p| p.dimension == active_dimension)
        {
            let (px, py, pz) = (player.x, player.y, player.z);
            let max_dist_sq = if self.config.fixes.container_interaction_check_enabled {
                let d = (self.config.fixes.container_interaction_max_distance.max(1) as f64) + 1.5;
                d * d
            } else {
                64.0
            };
            match &mut player.open_container {
                Some(OpenContainer::Workbench {
                    window_id, x, y, z, ..
                }) => {
                    let valid = self.world.get_block(*x, *y, *z) == blocks::WORKBENCH
                        && (px - (*x as f64 + 0.5)).powi(2)
                            + (py - (*y as f64 + 0.5)).powi(2)
                            + (pz - (*z as f64 + 0.5)).powi(2)
                            <= max_dist_sq;
                    if !valid {
                        if self.config.fixes.container_interaction_log {
                            log::warn(&format!(
                                "{} exceeded container interaction distance at ({}, {}, {})",
                                player.username, x, y, z
                            ));
                        }
                        force_closes.push((player.conn_id, *window_id));
                    }
                }
                Some(OpenContainer::Furnace {
                    window_id,
                    x,
                    y,
                    z,
                    slots,
                    cook_time,
                    burn_time,
                    ticks_for_current_fuel,
                }) => {
                    let dist_ok = (px - (*x as f64 + 0.5)).powi(2)
                        + (py - (*y as f64 + 0.5)).powi(2)
                        + (pz - (*z as f64 + 0.5)).powi(2)
                        <= max_dist_sq;
                    if let Some(TileEntity {
                        kind:
                            TileEntityKind::Furnace {
                                burn_time: te_burn,
                                ticks_for_current_fuel: te_fuel,
                                cook_time: te_cook,
                                items: te_items,
                            },
                        ..
                    }) = self.world.get_tile_entity(*x, *y, *z).filter(|_| dist_ok)
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
                    } else {
                        force_closes.push((player.conn_id, *window_id));
                    }
                }
                Some(OpenContainer::Chest {
                    window_id,
                    positions,
                    slots,
                }) => {
                    let mut valid = !positions.is_empty();
                    let mut slot_changes = Vec::new();
                    for (idx, &(cx, cy, cz)) in positions.iter().enumerate() {
                        let dist_ok = (px - (cx as f64 + 0.5)).powi(2)
                            + (py - (cy as f64 + 0.5)).powi(2)
                            + (pz - (cz as f64 + 0.5)).powi(2)
                            <= max_dist_sq;
                        if let Some(TileEntity {
                            kind: TileEntityKind::Chest { items: te_items },
                            ..
                        }) = self.world.get_tile_entity(cx, cy, cz).filter(|_| dist_ok)
                        {
                            let start = idx * 27;
                            for s in 0..27 {
                                if start + s < slots.len() && slots[start + s] != te_items[s] {
                                    slots[start + s] = te_items[s];
                                    slot_changes.push((
                                        (start + s) as i16,
                                        InventoryPlayer::to_slot_item(te_items[s]),
                                    ));
                                }
                            }
                        } else {
                            valid = false;
                            break;
                        }
                    }
                    if !valid {
                        force_closes.push((player.conn_id, *window_id));
                    } else if !slot_changes.is_empty() {
                        updates.push((player.conn_id, *window_id, Vec::new(), slot_changes));
                    }
                }
                Some(OpenContainer::Dispenser {
                    window_id,
                    x,
                    y,
                    z,
                    slots,
                }) => {
                    let dist_ok = (px - (*x as f64 + 0.5)).powi(2)
                        + (py - (*y as f64 + 0.5)).powi(2)
                        + (pz - (*z as f64 + 0.5)).powi(2)
                        <= max_dist_sq;
                    if let Some(TileEntity {
                        kind: TileEntityKind::Dispenser { items: te_items },
                        ..
                    }) = self.world.get_tile_entity(*x, *y, *z).filter(|_| dist_ok)
                    {
                        let mut slot_changes = Vec::new();
                        for i in 0..9 {
                            if slots[i] != te_items[i] {
                                slots[i] = te_items[i];
                                slot_changes
                                    .push((i as i16, InventoryPlayer::to_slot_item(te_items[i])));
                            }
                        }
                        if !slot_changes.is_empty() {
                            updates.push((player.conn_id, *window_id, Vec::new(), slot_changes));
                        }
                    } else {
                        force_closes.push((player.conn_id, *window_id));
                    }
                }
                Some(OpenContainer::MinecartChest {
                    window_id,
                    entity_id,
                    slots,
                }) => {
                    if let Some(&(cx, cy, cz, ref cargo)) = cart_snapshots.get(entity_id) {
                        let dist_ok = (px - cx).powi(2) + (py - cy).powi(2) + (pz - cz).powi(2)
                            <= max_dist_sq;
                        if dist_ok {
                            let mut slot_changes = Vec::new();
                            for i in 0..27 {
                                if slots[i] != cargo[i] {
                                    slots[i] = cargo[i];
                                    slot_changes
                                        .push((i as i16, InventoryPlayer::to_slot_item(cargo[i])));
                                }
                            }
                            if !slot_changes.is_empty() {
                                updates.push((
                                    player.conn_id,
                                    *window_id,
                                    Vec::new(),
                                    slot_changes,
                                ));
                            }
                        } else {
                            force_closes.push((player.conn_id, *window_id));
                        }
                    } else {
                        force_closes.push((player.conn_id, *window_id));
                    }
                }
                None => {}
            }
        }

        for (conn_id, win_id) in force_closes {
            self.close_player_container_and_drop_transient(conn_id, true);
            self.send(
                conn_id,
                Clientbound::CloseWindow(CloseWindow { window_id: win_id }),
            );
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

    /// Java `ItemWorldMap.a` + `WorldMapHumanTracker.a` (`ItemWorldMap.java:40-210`, `WorldMapHumanTracker.java:35-101`):
    /// stream `Packet131` (`MapData`) scanlines and player cursor icons for players holding `Item.MAP`.
    fn tick_held_maps(&mut self, ticks: u64) {
        let holders: Vec<(ConnId, i16, f64, f64, f32)> = self
            .entities
            .iter_players()
            .filter_map(|p| {
                if p.dead || p.dimension != self.active_dimension {
                    return None;
                }
                let stack = p.inventory.item_in_hand()?;
                if stack.id == items::MAP {
                    Some((p.conn_id, stack.damage, p.x, p.z, p.yaw))
                } else {
                    None
                }
            })
            .collect();
        if holders.is_empty() {
            return;
        }

        // Java `ItemWorldMap`: the map centre belongs to the world the map was
        // created in (`worldmap.map = world.worldProvider.dimension`), so the
        // Nether uses the Nether spawn as its centre.
        let (center_x, _, center_z) = self.world.spawn_point();
        let scale = 3i32; // 1 << 3 = 8 blocks per pixel
        let step = 1i32 << scale;

        for (conn_id, map_id, px, pz, yaw) in holders {
            if ticks % 5 == 0 {
                let rel_x = (((px - center_x as f64) / (step as f64)) * 2.0)
                    .round()
                    .clamp(-127.0, 127.0) as i8 as u8;
                let rel_z = (((pz - center_z as f64) / (step as f64)) * 2.0)
                    .round()
                    .clamp(-127.0, 127.0) as i8 as u8;
                let dir = (((yaw * 16.0 / 360.0).round() as i32) & 15) as u8;
                let data = vec![1u8, dir << 4, rel_x, rel_z];
                self.send(
                    conn_id,
                    Clientbound::MapData(MapData {
                        item_type: items::MAP,
                        item_id: map_id,
                        data,
                    }),
                );
            } else {
                let col = ((ticks * 11) % 128) as i32;
                let mut data = Vec::with_capacity(131);
                data.push(0u8);
                data.push(col as u8);
                data.push(0u8);
                let mut prev_h = 64i32;
                for row in 0..128i32 {
                    let wx = center_x + (col - 64) * step;
                    let wz = center_z + (row - 64) * step;
                    let wy = self.world.top_solid_or_liquid_block(wx, wz);
                    let (color_idx, shade) = if wy > 1 {
                        let bid = self.world.get_block(wx, wy - 1, wz);
                        let c_idx = blocks::material(bid).map_color_index();
                        let s = if wy > prev_h {
                            2u8
                        } else if wy < prev_h {
                            0u8
                        } else {
                            1u8
                        };
                        prev_h = wy;
                        (c_idx, s)
                    } else {
                        (0u8, 1u8)
                    };
                    data.push(color_idx * 4 + shade);
                }
                self.send(
                    conn_id,
                    Clientbound::MapData(MapData {
                        item_type: items::MAP,
                        item_id: map_id,
                        data,
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
        // TileEntitySign.isEditableBy (`NetServerHandler.java:1151-1164`)
        let can_edit = self.entities.get_by_conn(id).is_some_and(|p| {
            p.editing_sign == Some((x, y, z))
                || ((p.x - x as f64).powi(2) + (p.y - y as f64).powi(2) + (p.z - z as f64).powi(2)
                    <= 64.0
                    && p.editing_sign.is_none())
        });
        if !can_edit {
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
            if p.dimension == self.active_dimension && p.sent_chunks.contains(&cpos) {
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
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(dropped) = self.entities.get_dropped_item(item_id) {
            let packets = self.tracker.track_item_and_scan(dropped, &players);
            for outbound in packets {
                self.send_tracked(outbound.recipient_id, outbound.packet);
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
        let active_dimension = self.active_dimension;
        let players: Vec<_> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .cloned()
            .collect();
        if let Some(dropped) = self.entities.get_dropped_item(item_id) {
            let packets = self.tracker.track_item_and_scan(dropped, &players);
            for outbound in packets {
                self.send_tracked(outbound.recipient_id, outbound.packet);
            }
        }
    }

    fn tick_dropped_items(&mut self) {
        let active_dimension = self.active_dimension;
        let item_ids: Vec<i32> = self
            .entities
            .iter_dropped_items()
            .filter(|i| self.entities.dimension_of(i.entity_id) == active_dimension)
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
                let cur_id = self.world.get_block(bx, by, bz);

                if blocks::is_lava(cur_id) {
                    item.mot_y = 0.2;
                    item.health -= 4;
                } else if cur_id == blocks::FIRE || cur_id == blocks::CACTUS {
                    item.health -= 1;
                } else if blocks::is_opaque_cube(cur_id) {
                    // Entity.g(x, y, z) (`Entity.java:1277-1353`): push item out of opaque block
                    if !blocks::is_opaque_cube(self.world.get_block(bx, by + 1, bz)) {
                        item.mot_y = 0.2;
                    } else if !blocks::is_opaque_cube(self.world.get_block(bx - 1, by, bz)) {
                        item.mot_x = -0.2;
                    } else if !blocks::is_opaque_cube(self.world.get_block(bx + 1, by, bz)) {
                        item.mot_x = 0.2;
                    } else if !blocks::is_opaque_cube(self.world.get_block(bx, by, bz - 1)) {
                        item.mot_z = -0.2;
                    } else if !blocks::is_opaque_cube(self.world.get_block(bx, by, bz + 1)) {
                        item.mot_z = 0.2;
                    }
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
                if p.dead || p.dimension != self.active_dimension {
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
                self.send_tracked(outbound.recipient_id, outbound.packet);
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
                        let should_send = self.entities.get_by_conn(conn_id).is_some_and(|p| {
                            p.dimension == self.active_dimension && p.sent_chunks.contains(&pos)
                        });
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
                    if self.entities.get_by_conn(conn_id).is_some_and(|p| {
                        p.dimension == self.active_dimension && p.sent_chunks.contains(&pos)
                    }) {
                        self.send(conn_id, pkt.clone());
                    }
                }
            }
        }
    }

    fn find_safe_bed_respawn(&self, bx: i32, by: i32, bz: i32) -> Option<(i32, i32, i32)> {
        if self.world.get_block(bx, by, bz) != blocks::BED {
            return None;
        }
        let meta = self.world.get_data(bx, by, bz);
        let dir = (meta & 3) as usize;
        let (dx, dz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][dir];
        let (fx, fz) = if (meta & 8) != 0 {
            (bx - dx, bz - dz)
        } else {
            (bx + dx, bz + dz)
        };
        let min_x = bx.min(fx) - 1;
        let max_x = bx.max(fx) + 1;
        let min_z = bz.min(fz) - 1;
        let max_z = bz.max(fz) + 1;
        for rx in min_x..=max_x {
            for rz in min_z..=max_z {
                if by > 0
                    && by < 126
                    && self.world.is_solid_normal_at(rx, by - 1, rz)
                    && self.world.get_block(rx, by, rz) == blocks::AIR
                    && self.world.get_block(rx, by + 1, rz) == blocks::AIR
                {
                    return Some((rx, by, rz));
                }
            }
        }
        None
    }

    /// Java `BlockPortal.a(World, i, j, k, Entity)`: does the player's bounding
    /// box intersect a portal block?
    fn player_touches_portal(&self, x: f64, y: f64, z: f64) -> bool {
        let min_x = (x - 0.3).floor() as i32;
        let max_x = (x + 0.3).floor() as i32;
        let min_y = y.floor() as i32;
        let max_y = (y + 1.8).floor() as i32;
        let min_z = (z - 0.3).floor() as i32;
        let max_z = (z + 0.3).floor() as i32;
        for bx in min_x..=max_x {
            for by in min_y..=max_y {
                for bz in min_z..=max_z {
                    if self.world.get_block(bx, by, bz) == blocks::PORTAL {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Java `EntityPlayer.m_()` portal section (`F`, `D`, `E`) combined with
    /// `EntityHuman.P()`: 80 ticks in a portal (0.0125/tick), then
    /// `ServerConfigurationManager.f`.
    fn tick_player_portals(&mut self) {
        let ids: Vec<i32> = self.entities.iter_players().map(|p| p.entity_id).collect();
        for entity_id in ids {
            let Some(player) = self.entities.get_player(entity_id) else {
                continue;
            };
            if player.dead {
                continue;
            }
            let (dimension, x, y, z, conn_id) = (
                player.dimension,
                player.x,
                player.y,
                player.z,
                player.conn_id,
            );
            let touching = self.with_dimension(dimension, |s| s.player_touches_portal(x, y, z));

            let mut travel = false;
            let mut close_container = false;
            let mut close_window = None;
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                if touching {
                    // Java `EntityHuman.P()`: while the cooldown is active it
                    // is refreshed to 10 every tick the player still stands in
                    // a portal, so re-entry only happens after leaving it.
                    if player.portal_cooldown > 0 {
                        player.portal_cooldown = 10;
                    } else {
                        player.in_portal = true;
                    }
                    if let Some(container) = &player.open_container {
                        close_container = true;
                        close_window = Some(container.window_id());
                    }
                }
                if player.in_portal {
                    if player.vehicle_id.is_none() {
                        player.portal_time += 0.0125;
                        if player.portal_time >= 1.0 {
                            player.portal_time = 1.0;
                            player.portal_cooldown = 10;
                            travel = true;
                        }
                    }
                } else if player.portal_time > 0.0 {
                    player.portal_time = (player.portal_time - 0.05).max(0.0);
                }
                player.in_portal = false;
                if player.portal_cooldown > 0 {
                    player.portal_cooldown -= 1;
                }
            }
            if close_container {
                self.with_dimension(dimension, |s| {
                    s.close_player_container_and_drop_transient(conn_id, true)
                });
                if let Some(window_id) = close_window {
                    self.send(conn_id, Clientbound::CloseWindow(CloseWindow { window_id }));
                }
            }
            if travel {
                self.travel_player(entity_id);
            }
        }
    }

    /// Java `ServerConfigurationManager.f(EntityPlayer)` for the vanilla
    /// (plugin-free) path: scale by 1/8 or 8, find/create the destination
    /// portal, then `moveToWorld` with Poseidon's double `Packet9Respawn`.
    fn travel_player(&mut self, entity_id: i32) {
        let Some(player) = self.entities.get_player(entity_id) else {
            return;
        };
        let from_dimension = player.dimension;
        if !matches!(from_dimension, 0 | -1) {
            return;
        }
        let to_dimension = if from_dimension == -1 { 0 } else { -1 };
        if to_dimension == -1 && self.nether.is_none() {
            return;
        }
        let ratio = if from_dimension == -1 { 8.0 } else { 0.125 };
        let (x, y, z, yaw, pitch, conn_id) = (
            player.x,
            player.y,
            player.z,
            player.yaw,
            player.pitch,
            player.conn_id,
        );
        let target_x = x * ratio;
        let target_z = z * ratio;

        let (dest_x, dest_y, dest_z) = self.with_dimension(to_dimension, |s| {
            s.world.ensure_portal_search_chunks(target_x, target_z);
            s.world.find_or_create_portal(target_x, y, target_z)
        });

        // Release the player's chunk view in the source world.
        self.with_dimension(from_dimension, |s| {
            if s.world.live_is_active() {
                s.world.live_player_quit(conn_id);
            }
        });

        // Untrack before moving so old-dimension observers destroy the entity.
        let untrack = self.tracker.untrack_player(entity_id);
        for outbound in untrack {
            if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                self.send(target.conn_id, outbound.packet);
            }
        }

        let old_chunk = {
            let player = self.entities.get_player(entity_id).unwrap();
            player.chunk_pos()
        };
        if let Some(player) = self.entities.get_player_mut(entity_id) {
            player.dimension = to_dimension;
            player.x = dest_x;
            player.y = dest_y;
            player.z = dest_z;
            player.prev_x = dest_x;
            player.prev_y = dest_y;
            player.prev_z = dest_z;
            player.stance = dest_y;
            player.yaw = yaw;
            player.pitch = pitch;
            player.mot_x = 0.0;
            player.mot_y = 0.0;
            player.mot_z = 0.0;
            player.velocity_changed = false;
            player.portal_time = 1.0;
            player.in_portal = false;
            player.open_container = None;
            player.editing_sign = None;
            player.digging = None;
            player.pending_dig = None;
            player.sleeping = false;
            player.last_eval_x = dest_x;
            player.last_eval_z = dest_z;
            player.sent_chunks.clear();
            player.pre_chunk_sent.clear();
            player.chunk_queue.clear();
        }
        let new_chunk = {
            let player = self.entities.get_player(entity_id).unwrap();
            player.chunk_pos()
        };
        self.entities
            .update_player_chunk(entity_id, old_chunk, new_chunk);

        // Poseidon: send the opposite dimension first, then the actual one.
        let opposite = if to_dimension >= 0 { -1 } else { 0 };
        self.send(
            conn_id,
            Clientbound::Respawn(Respawn {
                dimension: opposite,
            }),
        );
        self.send(
            conn_id,
            Clientbound::Respawn(Respawn {
                dimension: to_dimension as i8,
            }),
        );

        self.with_dimension(to_dimension, |s| {
            // `ServerConfigurationManager.a(EntityPlayer, WorldServer)`:
            // update the world clock and weather, not the bed/spawn compass.
            let world_time = s.world.world_time();
            s.send(
                conn_id,
                Clientbound::UpdateTime(UpdateTime { time: world_time }),
            );
            if to_dimension == 0 && s.world.is_raining() {
                s.send(conn_id, Clientbound::Bed(Bed { reason: 1 }));
            }
            s.stream_initial_chunks(conn_id);
        });

        if let Some(player) = self.entities.get_player(entity_id) {
            self.tracker.track_player(player);
        }
    }

    fn handle_respawn(&mut self, id: ConnId) {
        let Some(from_dimension) = self.entities.get_by_conn(id).map(|p| p.dimension) else {
            return;
        };
        if self.nether.is_some() {
            self.with_dimension(from_dimension, |s| {
                if s.world.live_is_active() {
                    s.world.live_player_quit(id);
                }
            });
        } else if self.world.live_is_active() {
            self.world.live_player_quit(id);
        }

        // Java `NetServerHandler.a(Packet9Respawn)` always calls
        // `moveToWorld(player, 0)`: death respawns in the overworld, using the
        // bed if it is still valid (`ServerConfigurationManager.java:253-278`).
        let saved_bed = self.entities.get_by_conn(id).and_then(|p| p.spawn_bed);
        let mut invalid_bed = false;
        let (spawn_x, spawn_y, spawn_z) = self.with_dimension(0, |s| {
            if let Some((bx, by, bz)) = saved_bed {
                if let Some(pos) = s.find_safe_bed_respawn(bx, by, bz) {
                    pos
                } else {
                    invalid_bed = true;
                    s.world.safe_spawn_point()
                }
            } else {
                s.world.safe_spawn_point()
            }
        });

        // Untrack player in EntityTracker before respawn (`ServerConfigurationManager.moveToWorld`)
        // so both the respawned player and all observers clear stale tracked_players state
        // and re-receive NamedEntitySpawn packets after respawn!
        if let Some(p) = self.entities.get_by_conn(id) {
            let eid = p.entity_id;
            self.with_dimension(from_dimension, |s| s.dismount_player(eid));
            let untrack = self.tracker.untrack_player(eid);
            for outbound in untrack {
                if let Some(target) = self.entities.get_player(outbound.recipient_id) {
                    if target.dimension == from_dimension {
                        self.send(target.conn_id, outbound.packet);
                    }
                }
            }
        }

        // Poseidon double-respawn client hack: send opposite dimension first,
        // then the actual destination dimension (always 0 here).
        self.send(id, Clientbound::Respawn(Respawn { dimension: -1 }));
        self.send(id, Clientbound::Respawn(Respawn { dimension: 0 }));
        if invalid_bed {
            self.send(id, Clientbound::Bed(Bed { reason: 0 }));
        }

        let mut respawned_eid = None;
        let spawn_yaw = self.config.world.spawn_yaw;
        let spawn_pitch = self.config.world.spawn_pitch;
        if let Some(player) = self.entities.get_by_conn_mut(id) {
            let old_chunk = player.chunk_pos();
            player.dimension = 0;
            player.portal_cooldown = 0;
            player.portal_time = 0.0;
            player.in_portal = false;
            player.respawn(spawn_x as f64 + 0.5, spawn_y as f64, spawn_z as f64 + 0.5);
            player.yaw = spawn_yaw;
            player.pitch = spawn_pitch;
            player.sent_chunks.clear();
            player.pre_chunk_sent.clear();
            player.chunk_queue.clear();
            let new_chunk = player.chunk_pos();
            let entity_id = player.entity_id;
            respawned_eid = Some(entity_id);
            self.entities
                .update_player_chunk(entity_id, old_chunk, new_chunk);
        }

        self.with_dimension(0, |s| s.stream_initial_chunks(id));

        if let Some(eid) = respawned_eid {
            if let Some(player) = self.entities.get_player(eid) {
                self.tracker.track_player(player);
            }
        }

        // `moveToWorld` updates the world clock/weather; the respawn point is
        // the overworld spawn (or the bed, when one is valid). The respawn
        // world is always the overworld.
        let (world_time, raining) =
            self.with_dimension(0, |s| (s.world.world_time(), s.world.is_raining()));
        self.send(
            id,
            Clientbound::SpawnPosition(SpawnPosition {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
            }),
        );
        self.send(id, Clientbound::UpdateTime(UpdateTime { time: world_time }));
        if raining {
            self.send(id, Clientbound::Bed(Bed { reason: 1 }));
        }
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
        let protocol_allowed = if self.config.network.allowed_protocols.is_empty() {
            login.protocol == PROTOCOL_VERSION
        } else {
            self.config
                .network
                .allowed_protocols
                .contains(&login.protocol)
        };
        if !protocol_allowed {
            let reason = if login.protocol > PROTOCOL_VERSION {
                "Outdated server! I'm still on Beta 1.7.3"
            } else {
                "Outdated client! Please use Beta 1.7.3"
            };
            self.kick(id, reason);
            return;
        }

        let raw_name = login.username.trim();
        let name_str = if !self.config.server.online_mode
            && self.config.server.cracked_username_prefix
            && !raw_name.starts_with('.')
        {
            format!(".{raw_name}")
        } else {
            raw_name.to_string()
        };
        let name = name_str.as_str();

        // Poseidon username validity check (`poseidon.yml`: length min..=max, `username_regex`)
        if self.config.server.check_username_validity {
            let min_len = self.config.server.username_min_length;
            let max_len = self.config.server.username_max_length;
            if name.len() < min_len {
                self.kick(
                    id,
                    &format!("Sorry, your username is too short. The minimum length is: {min_len}"),
                );
                return;
            }
            if name.len() > max_len {
                self.kick(
                    id,
                    &format!("Sorry, your username is too long. The maximum length is: {max_len}"),
                );
                return;
            }
            let pattern = &self.config.server.username_regex;
            if !Self::matches_username_pattern(name, pattern) {
                self.kick(
                    id,
                    &format!("Sorry, your username is invalid, allowed characters: {pattern}"),
                );
                return;
            }
        }

        // Poseidon proxy handler (`NetLoginHandler.java:122-169`, `Release2Beta.java`)
        // `login.dimension`: 25 ((byte)-999) or 26 = Release2Beta IP forwarding, 1 = Release2Beta, 2 = BungeeCord IP forwarding
        let is_bungee_forwarding = login.dimension == 2;
        let is_r2b_forwarding = login.dimension == 25 || login.dimension == 26;
        if self.config.network.bungeecord_mode && !is_bungee_forwarding {
            let msg = self.config.network.bungeecord_kick_message.clone();
            self.kick(id, &msg);
            return;
        }
        if is_r2b_forwarding || is_bungee_forwarding {
            if self.config.network.release2beta_ip_passthrough {
                let socket_ip = self
                    .connections
                    .get(&id)
                    .map(|c| c.address.ip().to_string())
                    .unwrap_or_default();
                if socket_ip.eq_ignore_ascii_case(&self.config.network.release2beta_proxy_ip) {
                    let forwarded_v4 = std::net::Ipv4Addr::from(login.map_seed as u32);
                    log::info(&format!(
                        "{name} has been detected using Release2Beta/BungeeCord, using forwarded IP: {forwarded_v4}"
                    ));
                    if let Some(conn) = self.connections.get_mut(&id) {
                        conn.address.set_ip(std::net::IpAddr::V4(forwarded_v4));
                    }
                } else {
                    self.kick(
                        id,
                        "\u{00A7}cThe Release2Beta server you are connecting through is unauthorized.",
                    );
                    return;
                }
            } else {
                self.kick(
                    id,
                    "\u{00A7}cIP Forwarding is disabled in Poseidon. Please disable in Release2Beta.",
                );
                return;
            }
        }

        let lower_name = name.to_lowercase();
        let peer_ip = self
            .connections
            .get(&id)
            .map(|c| c.address.ip().to_string())
            .unwrap_or_default();

        if self.banned_players.contains(&lower_name) {
            let msg = self.config.messages.kick_banned.clone();
            self.kick(id, &msg);
            return;
        }
        if self.banned_ips.contains(&peer_ip.to_lowercase()) {
            let msg = self.config.messages.kick_ip_banned.clone();
            self.kick(id, &msg);
            return;
        }
        if self.whitelist_enabled
            && !self.whitelist.contains(&lower_name)
            && !self.ops.contains(&lower_name)
        {
            let msg = self.config.messages.kick_not_whitelisted.clone();
            self.kick(id, &msg);
            return;
        }
        // A player whose session is already gone must neither block the name
        // nor count towards the player limit.
        self.purge_dead_players();
        if self.entities.player_count() >= self.config.server.max_players as usize {
            let msg = self.config.messages.kick_full.clone();
            self.kick(id, &msg);
            return;
        }

        // Prevent duplicate username login (`LoginProcessHandler.java:222`, `poseidon.yml`)
        let already_online = self
            .entities
            .iter_players()
            .any(|p| p.username.eq_ignore_ascii_case(name));
        if already_online {
            let msg = self.config.messages.kick_already_online.clone();
            self.kick(id, &msg);
            return;
        }

        let seed = self.world.seed();
        let (mut spawn_x, mut spawn_y, mut spawn_z) = self.world.safe_spawn_point();
        if self.config.world.randomize_spawn && self.config.world.randomize_spawn_radius > 0 {
            let r = self.config.world.randomize_spawn_radius;
            spawn_x += self.world.tick_random.next_i32_bound(r * 2) - r;
            spawn_z += self.world.tick_random.next_i32_bound(r * 2) - r;
            if self.config.world.teleport_to_highest_safe_block {
                let g = self.world.top_solid_or_liquid_block(spawn_x, spawn_z);
                if g > 0 {
                    spawn_y = g;
                }
            }
        }
        let player_x = spawn_x as f64 + 0.5;
        let player_y = spawn_y as f64;
        let player_z = spawn_z as f64 + 0.5;

        let entity_id =
            self.entities
                .insert_player(id, name.to_string(), player_x, player_y, player_z);
        if let Some(player) = self.entities.get_player_mut(entity_id) {
            player.yaw = self.config.world.spawn_yaw;
            player.pitch = self.config.world.spawn_pitch;
        }

        if self.config.world.save_enabled && self.config.server.save_playerdata {
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

        // Java `NetLoginHandler.finishLogin`: the login packet carries the
        // player's saved dimension, and every subsequent chunk/teleport packet
        // comes from that world's chunk provider.
        let saved_dimension = self
            .entities
            .get_player(entity_id)
            .map(|p| p.dimension)
            .unwrap_or(0);
        let mut login_dimension = saved_dimension;
        let mut reset_position = false;
        if !matches!(login_dimension, 0 | -1) {
            login_dimension = 0;
            reset_position = true;
        }
        if login_dimension == -1 && self.nether.is_none() {
            login_dimension = 0;
            reset_position = true;
        }
        if reset_position {
            // The saved dimension is unavailable; fall back to the overworld
            // spawn instead of leaving the player at stored Nether coordinates.
            let (spawn_x, spawn_y, spawn_z) = self.world.safe_spawn_point();
            let mut chunk_shift = None;
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let old_chunk = player.chunk_pos();
                player.x = spawn_x as f64 + 0.5;
                player.y = spawn_y as f64;
                player.z = spawn_z as f64 + 0.5;
                player.stance = player.y;
                player.prev_x = player.x;
                player.prev_y = player.y;
                player.prev_z = player.z;
                player.last_eval_x = player.x;
                player.last_eval_z = player.z;
                let new_chunk = player.chunk_pos();
                if old_chunk != new_chunk {
                    chunk_shift = Some((old_chunk, new_chunk));
                }
            }
            if let Some((old_chunk, new_chunk)) = chunk_shift {
                self.entities
                    .update_player_chunk(entity_id, old_chunk, new_chunk);
            }
        }
        if let Some(player) = self.entities.get_player_mut(entity_id) {
            player.dimension = login_dimension;
        }

        self.with_dimension(login_dimension, |s| {
            // `NetLoginHandler.b`: the login packet carries the player's world
            // seed and dimension, the spawn position packet the raw world
            // spawn (`World.getSpawn()`, not the "safe" block-adjusted one).
            let world_id = s.world.dimension as i8;
            s.send(
                id,
                Clientbound::Login(Login {
                    protocol: entity_id,
                    username: String::new(),
                    map_seed: seed,
                    dimension: world_id,
                }),
            );
            let (spawn_x, spawn_y, spawn_z) = s.world.spawn_point();
            s.send(
                id,
                Clientbound::SpawnPosition(SpawnPosition {
                    x: spawn_x,
                    y: spawn_y,
                    z: spawn_z,
                }),
            );
            let world_time = s.world.world_time();
            s.send(id, Clientbound::UpdateTime(UpdateTime { time: world_time }));

            // Stream initial spawn area chunks and send spawn teleport
            s.stream_initial_chunks(id);

            if login_dimension == 0 && s.world.live_is_active() && s.world.is_raining() {
                s.send(id, Clientbound::Bed(Bed { reason: 1 }));
            }
        });

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
        let join_msg = self.config.messages.join.replacen("%player%", name, 1);
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
        let dimensions: Vec<i32> = if self.nether.is_some() {
            vec![0, -1]
        } else {
            vec![0]
        };
        for dimension in dimensions {
            self.with_dimension(dimension, |s| s.stream_chunks_inner());
        }
    }

    fn stream_chunks_inner(&mut self) {
        let active_dimension = self.active_dimension;
        let max_per_tick = if self.world.live_is_active() { 16 } else { 2 };
        let chunks_per_tick = self.config.world.chunks_per_tick.clamp(1, max_per_tick);

        let player_ids: Vec<ConnId> = self
            .entities
            .iter_players()
            .filter(|p| p.dimension == active_dimension)
            .map(|p| p.conn_id)
            .collect();
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
        self.run_command(CommandSender::Player(id), command);
    }

    /// Runs a server command for `sender`.
    ///
    /// Java routes console commands and player commands through the same
    /// command map (`MinecraftServer.b()` → `server.dispatchCommand`); only
    /// the listener differs — the console answers on the terminal, a player
    /// receives chat packets.
    fn run_command(&mut self, sender: CommandSender, command: &str) {
        let username = match sender {
            CommandSender::Player(id) => self
                .connections
                .get(&id)
                .and_then(|connection| connection.username.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            CommandSender::Console => {
                let name = self.config.server.console_name.trim();
                if name.is_empty() {
                    "CONSOLE".to_string()
                } else {
                    name.to_string()
                }
            }
        };

        // The console accepts commands with and without the leading slash.
        let cmd_body = command.strip_prefix('/').unwrap_or(command);
        if !self.config.server.enable_tps_command
            && cmd_body
                .split_whitespace()
                .next()
                .is_some_and(|c| c.eq_ignore_ascii_case("tps"))
        {
            self.reply(sender, "Unknown command. Type \"help\" for help.");
            return;
        }
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
            self.reply(sender, "Unknown command. Type \"help\" for help.");
            return;
        }

        log::info(&format!("{username} issued server command: /{cmd_body}"));

        for action in result.actions {
            self.execute_command_action(action);
        }

        for msg in &result.sender_messages {
            self.reply(sender, msg);
        }

        for (target_name, msg) in &result.target_messages {
            if let Some(target_conn) = self
                .entities
                .iter_players()
                .find(|p| p.username.eq_ignore_ascii_case(target_name))
                .map(|p| p.conn_id)
            {
                if Some(target_conn) != sender.player_connection() {
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
            // The console sees the broadcasts it triggers as well.
            if sender == CommandSender::Console {
                self.reply(sender, msg);
            }
        }
    }

    /// Sends a command answer to whoever issued the command.
    fn reply(&self, sender: CommandSender, message: &str) {
        match sender {
            CommandSender::Player(id) => {
                self.send(
                    id,
                    Clientbound::Chat(Chat {
                        message: message.to_string(),
                    }),
                );
            }
            CommandSender::Console => {
                let text = crate::console::strip_formatting(message);
                (self.console_sink)(&text);
            }
        }
    }

    /// Drains the console command queue (`MinecraftServer.b()`).
    fn process_console_commands(&mut self) {
        let Some(rx) = &self.console_rx else {
            return;
        };
        let mut queued = Vec::new();
        while let Ok(line) = rx.try_recv() {
            queued.push(line);
        }
        for line in queued {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            self.run_command(CommandSender::Console, line);
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
                    .map(|p| (p.x, p.y, p.z, p.yaw, p.pitch, p.dimension));
                let victim_info = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&victim))
                    .map(|p| (p.conn_id, p.entity_id, p.dimension));
                if let (
                    Some((tx, ty, tz, tyaw, tpitch, target_dimension)),
                    Some((vconn, veid, victim_dimension)),
                ) = (target_pos, victim_info)
                {
                    // A teleport never changes the dimension: the victim keeps
                    // its own world and the coordinates are interpreted there.
                    if target_dimension != victim_dimension {
                        self.send(
                            vconn,
                            Clientbound::Chat(Chat {
                                message: "Cannot teleport across dimensions!".to_string(),
                            }),
                        );
                    } else {
                        self.with_dimension(victim_dimension, |s| {
                            s.dismount_player(veid);
                            if let Some(vp) = s.entities.get_player_mut(veid) {
                                let old_c = vp.chunk_pos();
                                vp.update_position(tx, ty, ty, tz, tyaw, tpitch, false);
                                vp.fall_distance = 0.0;
                                let new_c = vp.chunk_pos();
                                s.entities.update_player_chunk(veid, old_c, new_c);
                            }
                            s.send(
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
                            s.handle_movement(
                                vconn,
                                Some((tx, ty, ty, tz)),
                                Some((tyaw, tpitch)),
                                false,
                            );
                        });
                    }
                }
            }
            CommandAction::TeleportToCoords { victim, x, y, z } => {
                let victim_info = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&victim))
                    .map(|p| (p.conn_id, p.entity_id, p.yaw, p.pitch, p.dimension));
                if let Some((vconn, veid, vyaw, vpitch, victim_dimension)) = victim_info {
                    self.with_dimension(victim_dimension, |s| {
                        s.dismount_player(veid);
                        if let Some(vp) = s.entities.get_player_mut(veid) {
                            let old_c = vp.chunk_pos();
                            vp.update_position(x, y, y, z, vyaw, vpitch, false);
                            vp.fall_distance = 0.0;
                            let new_c = vp.chunk_pos();
                            s.entities.update_player_chunk(veid, old_c, new_c);
                        }
                        s.send(
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
                        s.handle_movement(vconn, Some((x, y, y, z)), Some((vyaw, vpitch)), false);
                    });
                }
            }
            CommandAction::SetTime(t) => {
                self.world.live_time = t;
                self.world.level_data.borrow_mut().time = t;
                if self.nether.is_some() {
                    self.with_dimension(-1, |s| {
                        s.world.live_time = t;
                        s.world.level_data.borrow_mut().time = t;
                    });
                }
                self.broadcast_all(Clientbound::UpdateTime(UpdateTime { time: t }));
            }
            CommandAction::AddTime(delta) => {
                let t = self.world.world_time().wrapping_add(delta);
                self.world.live_time = t;
                self.world.level_data.borrow_mut().time = t;
                if self.nether.is_some() {
                    self.with_dimension(-1, |s| {
                        s.world.live_time = t;
                        s.world.level_data.borrow_mut().time = t;
                    });
                }
                self.broadcast_all(Clientbound::UpdateTime(UpdateTime { time: t }));
            }
            CommandAction::Kill(target) => {
                let target_eid = self
                    .entities
                    .iter_players()
                    .find(|p| p.username.eq_ignore_ascii_case(&target))
                    .map(|p| p.entity_id);
                if let Some(eid) = target_eid {
                    // The death packets belong to the victim's dimension.
                    let victim_dimension = self
                        .entities
                        .get_player(eid)
                        .map(|p| p.dimension)
                        .unwrap_or(0);
                    self.with_dimension(victim_dimension, |s| {
                        s.damage_player_direct(eid, 1000, "died");
                    });
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
                    let msg = self.config.messages.kick_banned.clone();
                    self.kick(conn_id, &msg);
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
                let msg = self.config.messages.kick_ip_banned.clone();
                for cid in matching_conns {
                    self.kick(cid, &msg);
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
                if self.nether.is_some() {
                    self.with_dimension(-1, |s| {
                        let _ = s.world.save_all();
                    });
                }
            }
            CommandAction::SaveOn => {
                self.config.world.save_enabled = true;
            }
            CommandAction::SaveOff => {
                self.config.world.save_enabled = false;
            }
            CommandAction::Stop => {
                if self.config.world.save_enabled && self.config.server.save_playerdata {
                    for player in self.entities.iter_players() {
                        let _ = self.player_store.save_player(player);
                    }
                }
                let _ = self.world.save_all();
                if self.nether.is_some() {
                    self.with_dimension(-1, |s| {
                        let _ = s.world.save_all();
                    });
                }
                let conn_ids: Vec<ConnId> = self.connections.keys().copied().collect();
                let msg = self.config.messages.kick_shutdown.clone();
                for cid in conn_ids {
                    self.kick(cid, &msg);
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

    fn matches_username_pattern(name: &str, pattern: &str) -> bool {
        if let (Some(start), Some(end)) = (pattern.find('['), pattern.rfind(']')) {
            if start < end {
                let class = &pattern[(start + 1)..end];
                let chars: Vec<char> = class.chars().collect();
                return name.chars().all(|ch| {
                    let mut idx = 0;
                    while idx < chars.len() {
                        if idx + 2 < chars.len() && chars[idx + 1] == '-' {
                            if ch >= chars[idx] && ch <= chars[idx + 2] {
                                return true;
                            }
                            idx += 3;
                        } else {
                            if ch == chars[idx] {
                                return true;
                            }
                            idx += 1;
                        }
                    }
                    ch == '?' && class.contains('.')
                });
            }
        }
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '?')
    }

    fn damage_player_direct(&mut self, entity_id: i32, damage: i16, reason: &str) -> bool {
        let keep_inv = self.config.mechanics.keep_inventory_on_death;
        let (conn_id, username, health, died, drops) = {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let (dealt, died) = player.take_damage_direct(damage);
                if dealt > 0 {
                    let drops = if died && !keep_inv {
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
                    return false;
                }
            } else {
                return false;
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
        true
    }

    #[allow(dead_code)]
    fn damage_player(&mut self, entity_id: i32, damage: i16, reason: &str) {
        let keep_inv = self.config.mechanics.keep_inventory_on_death;
        let (conn_id, username, health, died, drops) = {
            if let Some(player) = self.entities.get_player_mut(entity_id) {
                let (dealt, died) = player.take_damage(damage);
                if dealt > 0 {
                    let drops = if died && !keep_inv {
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

    /// Send a world-scoped packet to players in the currently active
    /// dimension only (sounds, explosions, entity status, weather, ...).
    fn broadcast(&self, packet: Clientbound) {
        let active_dimension = self.active_dimension;
        for (&conn_id, connection) in &self.connections {
            if connection.state != ConnectionState::Playing {
                continue;
            }
            if self
                .entities
                .get_by_conn(conn_id)
                .is_some_and(|p| p.dimension == active_dimension)
            {
                let _ = connection.sender.send(Outbound::Packet(packet.clone()));
            }
        }
    }

    /// Send a packet to every logged-in player regardless of dimension
    /// (chat, time-of-day, keep-alive, ...).
    fn broadcast_all(&self, packet: Clientbound) {
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

    fn kick(&mut self, id: ConnId, reason: &str) {
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
        // The session ends now: the player must neither block the name nor
        // count towards the player limit until the disconnect event arrives.
        self.cleanup_player(id);
    }
}

/// The height of the eyes of a player (`EntityPlayer.t()`).
///
/// The constant of the reference is a single precision one.
const PLAYER_EYE_HEIGHT: f64 = 1.62f32 as f64;

/// Java `EntityLiving.b(float, float, float)`: the turn of the mob with the
/// limit of the turn.
fn clamped_turn(current: f32, target: f32, limit: f32) -> f32 {
    let mut diff = target - current;
    while diff < -180.0 {
        diff += 360.0;
    }
    while diff >= 180.0 {
        diff -= 360.0;
    }
    if diff > limit {
        diff = limit;
    }
    if diff < -limit {
        diff = -limit;
    }
    current + diff
}

/// Java `EntityLiving.a(Entity, float, float)`: the mob turns towards the
/// target.
///
/// The eyes of the target are given (`EntityLiving.t()` of the target), the
/// eyes of the mob are the height of its width.
fn look_at(
    mob: &mut crate::entity::mob::Mob,
    target_x: f64,
    target_y: f64,
    target_z: f64,
    target_eye_height: f64,
    yaw_limit: f32,
    pitch_limit: f32,
) {
    let dx = target_x - mob.x;
    let dz = target_z - mob.z;
    let dy = mob.y + mob.eye_height() - (target_y + target_eye_height);
    // `MathHelper.a(double)`: the square root is a single precision value.
    let distance = crate::util::math::math_helper_a(dx * dx + dz * dz) as f64;
    let target_yaw = (crate::util::fdlibm::atan2(dz, dx) * 180.0
        / crate::entity::mob::JAVA_PI_DOUBLE) as f32
        - 90.0;
    let target_pitch = (-(crate::util::fdlibm::atan2(dy, distance) * 180.0
        / crate::entity::mob::JAVA_PI_DOUBLE)) as f32;
    mob.pitch = -clamped_turn(mob.pitch, target_pitch, pitch_limit);
    mob.yaw = clamped_turn(mob.yaw, target_yaw, yaw_limit);
}

/// Java `EntityGhast.a(double, double, double, double)`: is the way of the
/// ghast free of blocks?
fn ghast_path_is_free(
    mob: &crate::entity::mob::Mob,
    world: &crate::world::world::World,
    dist: f64,
) -> bool {
    let step_x = (mob.ghast_waypoint.0 - mob.x) / dist;
    let step_y = (mob.ghast_waypoint.1 - mob.y) / dist;
    let step_z = (mob.ghast_waypoint.2 - mob.z) / dist;
    let mut box_ = mob.aabb();
    let mut index = 1;
    while (index as f64) < dist {
        box_.offset(step_x, step_y, step_z);
        let mut scratch = Vec::new();
        crate::entity::physics::collect_collision_boxes(world, &box_, &[], &mut scratch);
        if !scratch.is_empty() {
            return false;
        }
        index += 1;
    }
    true
}

/// Java `EntityLiving.c_()`: the mob looks around.
///
/// The reference draws two numbers every tick (the look at a player and the
/// drift of the yaw) and one more when the mob is in the water or in the lava.
fn random_look(
    mob: &mut crate::entity::mob::Mob,
    rng: &mut JavaRandom,
    players: &[(i32, String, f64, f64, f64, bool)],
) {
    // The movement input of the mob is reset (`EntityLiving.c_()`).
    mob.move_strafe = 0.0;
    mob.move_forward = 0.0;
    if rng.next_f32() < 0.02 {
        // The mob looks at a nearby player.
        let nearby = players.iter().find(|&&(_pid, _, px, py, pz, dead)| {
            !dead && (px - mob.x).powi(2) + (py - mob.y).powi(2) + (pz - mob.z).powi(2) <= 64.0
        });
        match nearby {
            Some(&(pid, _, _, _, _, _)) => {
                mob.look_target = Some(pid);
                mob.look_ticks = 10 + rng.next_i32_bound(20);
            }
            None => {
                mob.yaw_drift = (rng.next_f32() - 0.5) * 20.0;
            }
        }
    }
    if let Some(pid) = mob.look_target {
        // The mob turns towards the player (`EntityLiving.a(Entity, float,
        // float)`): the limit of the turn is 10 degrees and the limit of the
        // pitch is `EntityLiving.u()`.
        let mut drop = false;
        match players.iter().find(|&&(id, ..)| id == pid) {
            Some(&(_, _, px, py, pz, dead)) => {
                look_at(mob, px, py, pz, PLAYER_EYE_HEIGHT, 10.0, 40.0);
                // The mob stops looking at the player when the player is dead
                // or too far away (`EntityLiving.c_()`).
                if dead || (px - mob.x).powi(2) + (py - mob.y).powi(2) + (pz - mob.z).powi(2) > 64.0
                {
                    drop = true;
                }
            }
            None => drop = true,
        }
        // The counter of the ticks the mob looks at the player
        // (`EntityLiving.aF`): the counter is checked before it is
        // decremented.
        if mob.look_ticks <= 0 {
            drop = true;
        }
        mob.look_ticks -= 1;
        if drop {
            mob.look_target = None;
        }
    } else {
        if rng.next_f32() < 0.05 {
            mob.yaw_drift = (rng.next_f32() - 0.5) * 20.0;
        }
        mob.yaw += mob.yaw_drift;
        // The pitch of the mob is reset (`EntityLiving.aD`).
        mob.pitch = 0.0;
    }
    // The mob swims up in the water or in the lava (`EntityLiving.c_()`).
    if mob.in_water || mob.in_lava {
        mob.jump_flag = rng.next_f32() < 0.8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat floor of the scenario (`flatFloor` of the harness).
    fn scenario_flat_floor(world: &mut World, floor_y: i32, radius: i32) {
        for x in -radius..=radius {
            for z in -radius..=radius {
                for y in floor_y + 1..=floor_y + 14 {
                    world.set_block(x, y, z, blocks::AIR);
                }
                // The floor is grass: the animals score the blocks of their
                // walk with the grass below them (`EntityAnimal.a`).
                world.set_block(x, floor_y, z, blocks::GRASS);
            }
        }
    }

    /// The light of a scenario: the sky light is 15 above the floor of the
    /// scenario and 0 below it.
    ///
    /// The harness of the reference sets the sky light of the area of the
    /// scenario by hand, so the mobs score the blocks of their walk with the
    /// same light.
    fn scenario_sky_light(world: &mut World, floor_y: i32, radius: i32) {
        let min = -radius;
        let max = radius;
        for cx in (min >> 4)..=(max >> 4) {
            for cz in (min >> 4)..=(max >> 4) {
                let Some(chunk) = world.get_chunk_mut(ChunkPos::new(cx, cz)) else {
                    continue;
                };
                for x in 0..16usize {
                    for z in 0..16usize {
                        let wx = cx * 16 + x as i32;
                        let wz = cz * 16 + z as i32;
                        if !(min..=max).contains(&wx) || !(min..=max).contains(&wz) {
                            continue;
                        }
                        for y in 0..128usize {
                            let value = if y as i32 > floor_y { 15 } else { 0 };
                            chunk.set_sky_light(x, y, z, value);
                        }
                    }
                }
            }
        }
    }

    /// The server of a scenario: the terrain, the mob and the player target.
    fn mob_scenario_server(scenario: &str) -> (Server, i32) {
        let mut config = Config::default();
        // The seed of the reference harness (the shared random of the
        // entities is seeded with it).
        config.world.seed = 12345;
        config.world.save_enabled = false;
        config.world.view_distance = 2;
        config.server.allow_nether = false;
        // The scenario has exactly one mob, the natural spawning must not add
        // any other.
        config.spawning.spawn_monsters = false;
        config.spawning.spawn_animals = false;
        config.spawning.spawn_water_mobs = false;
        config.spawning.spawn_squids = false;
        config.spawning.spawn_wolves = false;
        config.spawning.spawn_slimes = false;

        let (_tx, rx) = mpsc::channel(8);
        let mut server = Server::new(config, rx, Arc::new(AtomicBool::new(true)));

        // The chunks of the scenario: they hold the terrain of the scenario
        // and the light. The harness of the reference generates the chunks
        // with the normal generator, so the port does the same.
        for cx in -2..=1 {
            for cz in -2..=1 {
                server.world.populate_chunk(ChunkPos::new(cx, cz));
            }
        }

        let (mob_type, player, floor_y, radius) = match scenario {
            "flat" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Pig, None, 63, 24)
            }
            "chase" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                for x in 4..=24 {
                    for z in -12..=12 {
                        server.world.set_block(x, 64, z, blocks::STONE);
                    }
                }
                (MobType::Zombie, Some((10.5, 65.0, 0.5)), 63, 24)
            }
            "water" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                for x in 2..=8 {
                    for z in -3..=3 {
                        server.world.set_block(x, 64, z, blocks::STATIONARY_WATER);
                        server.world.set_block(x, 63, z, blocks::STATIONARY_WATER);
                    }
                }
                (MobType::Zombie, Some((12.5, 64.0, 0.5)), 63, 24)
            }
            "wall" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                for y in 64..=67 {
                    for z in -6..=6 {
                        server.world.set_block(4, y, z, blocks::STONE);
                    }
                }
                (MobType::Spider, Some((10.5, 64.0, 0.5)), 63, 24)
            }
            "lava" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                for x in 2..=8 {
                    for z in -3..=3 {
                        server.world.set_block(x, 64, z, blocks::STATIONARY_LAVA);
                        server.world.set_block(x, 63, z, blocks::STATIONARY_LAVA);
                    }
                }
                (MobType::Zombie, Some((12.5, 64.0, 0.5)), 63, 24)
            }
            "slime" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Slime, None, 63, 24)
            }
            "squid" => {
                scenario_flat_floor(&mut server.world, 60, 12);
                for x in -8..=8 {
                    for z in -8..=8 {
                        for y in 61..=74 {
                            server.world.set_block(x, y, z, blocks::STATIONARY_WATER);
                        }
                    }
                }
                (MobType::Squid, None, 60, 12)
            }
            "ghast" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Ghast, None, 63, 24)
            }
            "creeper" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Creeper, Some((10.5, 64.0, 0.5)), 63, 24)
            }
            "skeleton" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Skeleton, Some((10.5, 64.0, 0.5)), 63, 24)
            }
            "chicken" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Chicken, None, 63, 24)
            }
            "cow" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Cow, None, 63, 24)
            }
            "sheep" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Sheep, None, 63, 24)
            }
            "wolf" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Wolf, Some((10.5, 64.0, 0.5)), 63, 24)
            }
            "pigzombie" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::PigZombie, Some((10.5, 64.0, 0.5)), 63, 24)
            }
            "giant" => {
                scenario_flat_floor(&mut server.world, 63, 24);
                (MobType::Giant, Some((10.5, 64.0, 0.5)), 63, 24)
            }
            other => panic!("unknown mob scenario {other}"),
        };

        // The light of the chunks of the scenario.
        for cx in -2..=1 {
            for cz in -2..=1 {
                if let Some(chunk) = server.world.get_chunk_mut(ChunkPos::new(cx, cz)) {
                    chunk.init_lighting();
                }
            }
        }
        while server.world.java_do_lighting() {}

        // The sky light of the scenario (`scenarioLight` of the harness).
        scenario_sky_light(&mut server.world, floor_y, radius);

        if let Some((px, py, pz)) = player {
            let entity_id = server
                .entities
                .insert_player(1, "Player".to_string(), px, py, pz);
            // The player of the reference harness is created without the
            // constructor, so its box is empty: the mob walks through it.
            if let Some(player) = server.entities.get_player_mut(entity_id) {
                player.box_size = (0.0, 0.0);
            }
        }
        let mob_id = server.spawn_mob_at(mob_type, 0.5, 64.0, 0.5);
        if scenario == "squid" {
            // The squid swims in the middle of the tank.
            if let Some(mob) = server.entities.get_mob_mut(mob_id) {
                mob.x = 0.5;
                mob.y = 66.0;
                mob.z = 0.5;
            }
        }
        if scenario == "ghast" {
            if let Some(mob) = server.entities.get_mob_mut(mob_id) {
                mob.y = 70.0;
            }
        }
        (server, mob_id)
    }

    /// Java `String.format("%.Nf", value)`.
    ///
    /// Java rounds half up, Rust rounds half to even, so the same value can
    /// be printed with different digits. The trace of the reference is
    /// compared as text, so the port has to round the way of Java.
    fn java_fixed(value: f64, decimals: usize) -> String {
        // The exact expansion of the value, with enough digits to see whether
        // the remainder after the last kept digit is zero.
        let exact = format!("{:.*}", decimals + 40, value);
        let (sign, digits) = match exact.strip_prefix('-') {
            Some(rest) => ("-", rest),
            None => ("", exact.as_str()),
        };
        let (integer, fraction) = match digits.split_once('.') {
            Some((integer, fraction)) => (integer, fraction),
            None => (digits, ""),
        };
        let mut kept: Vec<u8> = fraction.bytes().take(decimals).collect();
        // The rounding of Java is half up: the last digit is rounded up when
        // the remainder is a half or more.
        let round_up = fraction
            .as_bytes()
            .get(decimals)
            .is_some_and(|first| *first >= b'5');
        let mut integer = integer.to_string();
        if round_up {
            let mut index = kept.len();
            let mut carry = true;
            while carry && index > 0 {
                index -= 1;
                if kept[index] == b'9' {
                    kept[index] = b'0';
                } else {
                    kept[index] += 1;
                    carry = false;
                }
            }
            if carry {
                // The carry went through all the digits of the fraction.
                kept.insert(0, b'1');
                kept.truncate(decimals);
                integer = increment_decimal(&integer);
            }
        }
        let fraction: String = kept.iter().map(|digit| *digit as char).collect();
        format!("{sign}{integer}.{fraction}")
    }

    /// The decimal string plus one.
    fn increment_decimal(text: &str) -> String {
        let mut digits: Vec<u8> = text.bytes().collect();
        let mut index = digits.len();
        loop {
            if index == 0 {
                digits.insert(0, b'1');
                break;
            }
            index -= 1;
            if digits[index] == b'9' {
                digits[index] = b'0';
            } else {
                digits[index] += 1;
                break;
            }
        }
        String::from_utf8(digits).unwrap_or_else(|_| text.to_string())
    }

    /// The path of a mob, hashed the same way as the reference harness.
    fn scenario_path_hash(mob: &crate::entity::mob::Mob) -> u64 {
        // The reference has no path when the mob does not follow one.
        if mob.path.is_empty() {
            return 0;
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (index, point) in mob.path.iter().enumerate() {
            let mut value = ((point.0 as u64) << 40) | ((point.1 as u64) << 20) | (point.2 as u64);
            if index == mob.path_index {
                value ^= 0x5DEE_CE66D;
            }
            hash ^= value;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        hash
    }

    /// The trace of a scenario: the state of the mob after every tick.
    fn mob_scenario_trace(scenario: &str, ticks: usize) -> Vec<String> {
        let (mut server, mob_id) = mob_scenario_server(scenario);
        let mut lines: Vec<String> = Vec::with_capacity(ticks);
        for tick in 0..ticks {
            server.tick_mobs();
            let Some(mob) = server.entities.get_mob(mob_id) else {
                // The mob left the world: the harness of the reference keeps
                // the entity and prints its frozen state.
                if let Some(last) = lines.last() {
                    let rest = &last[last.find(' ').unwrap_or(0)..];
                    lines.push(format!("{tick}{rest}"));
                }
                continue;
            };
            // The target of the reference trace is the player (the id 1000 of
            // the harness) or nothing.
            let target = match mob.target_id {
                Some(id) if server.entities.get_player(id).is_some() => 1000,
                Some(_) => -2,
                None => -1,
            };
            lines.push(format!(
                "{} {} {} {} {} {} {} {} {} {} {} {} {} {:016x}",
                tick,
                java_fixed(mob.x, 9),
                java_fixed(mob.y, 9),
                java_fixed(mob.z, 9),
                java_fixed(mob.mot_x, 9),
                java_fixed(mob.mot_y, 9),
                java_fixed(mob.mot_z, 9),
                java_fixed(mob.yaw as f64, 7),
                java_fixed(mob.pitch as f64, 7),
                if mob.on_ground { 1 } else { 0 },
                java_fixed(mob.fall_distance as f64, 7),
                target,
                mob.path.len(),
                scenario_path_hash(mob),
            ));
        }
        lines
    }

    /// The fall damage of a mob is one hit of all the damage.
    ///
    /// Java `EntityLiving.a(float)`: the damage is
    /// `(int) Math.ceil(fall - 3.0F)` and the mob is hit once, so the window
    /// of the invulnerability of the reference does not swallow it.
    #[test]
    fn the_fall_damage_of_a_mob_is_one_hit() {
        let (mut server, mob_id) = mob_scenario_server("flat");
        // The mob falls from the height of ten blocks.
        if let Some(mob) = server.entities.get_mob_mut(mob_id) {
            mob.y = 74.0;
            mob.mot_y = 0.0;
            mob.fall_distance = 0.0;
        }
        for _ in 0..60 {
            server.tick_mobs();
        }
        let mob = server.entities.get_mob(mob_id).expect("the mob is alive");
        // The pig has ten points of health, `ceil(10 - 3)` of them are gone.
        assert_eq!(mob.health, 3);
    }

    /// The mob below the world takes four points of damage.
    ///
    /// Java `Entity.R()` calls `EntityLiving.Y()`, which is four points of
    /// damage.
    #[test]
    fn the_void_damage_of_a_mob_is_four() {
        let (mut server, mob_id) = mob_scenario_server("flat");
        if let Some(mob) = server.entities.get_mob_mut(mob_id) {
            mob.y = -70.0;
        }
        server.tick_mobs();
        let mob = server.entities.get_mob(mob_id).expect("the mob is alive");
        // The pig has ten points of health, four of them are gone.
        assert_eq!(mob.health, 6);
    }

    /// The drowning of a mob takes two points of damage.
    ///
    /// Java `EntityLiving.R()`: the air of the mob runs out, the mob makes
    /// eight bubbles and takes two points of damage.
    #[test]
    fn the_drowning_of_a_mob_hurts_it_twice() {
        let (mut server, mob_id) = mob_scenario_server("water");
        // The head of the mob is under the water.
        for y in 64..=66 {
            server.world.set_block(0, y, 0, blocks::STATIONARY_WATER);
        }
        if let Some(mob) = server.entities.get_mob_mut(mob_id) {
            mob.x = 0.5;
            mob.y = 64.0;
            mob.z = 0.5;
            // The air of the mob runs out at the next tick.
            mob.air_ticks = -19;
        }
        server.tick_mobs();
        let mob = server.entities.get_mob(mob_id).expect("the mob is alive");
        assert_eq!(mob.health, 18);
    }

    /// The suffocation of a mob takes one point of damage.
    ///
    /// Java `EntityLiving.R()`: the head of the mob is inside a block.
    #[test]
    fn the_suffocation_of_a_mob_hurts_it_once() {
        let (mut server, mob_id) = mob_scenario_server("flat");
        // The head of the mob is inside a block: the pig is 0.9 blocks high,
        // so the head of the mob is in the block of its feet.
        server.world.set_block(0, 64, 0, blocks::STONE);
        server.tick_mobs();
        let mob = server.entities.get_mob(mob_id).expect("the mob is alive");
        // The pig has ten points of health, one of them is gone.
        assert_eq!(mob.health, 9);
    }

    /// The traces of the port have to be the traces of the reference, tick by
    /// tick and without a tolerance.
    #[test]
    fn the_mob_scenarios_of_the_reference_match() {
        let golden = golden_mob_traces();
        let mut failures = Vec::new();
        for name in [
            "flat",
            "chase",
            "water",
            "wall",
            "lava",
            "slime",
            "squid",
            "ghast",
            "creeper",
            "skeleton",
            "chicken",
            "cow",
            "sheep",
            "wolf",
            "pigzombie",
            "giant",
        ] {
            let Some(expected) = golden.iter().find(|(scenario, _)| *scenario == name) else {
                failures.push(format!("{name}: the golden trace is missing"));
                continue;
            };
            let trace = mob_scenario_trace(name, expected.1.len());
            for (tick, (actual, expected)) in trace.iter().zip(expected.1.iter()).enumerate() {
                if actual != expected {
                    failures.push(format!(
                        "{name}: diverges at tick {tick}: {actual} != {expected}"
                    ));
                    break;
                }
            }
            if trace.len() != expected.1.len() {
                failures.push(format!(
                    "{name}: the trace has {} ticks, the reference has {}",
                    trace.len(),
                    expected.1.len()
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "the traces of the port differ from the reference:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    #[ignore = "temp debug"]
    fn debug_find_path() {
        let (server, mob_id) = mob_scenario_server("chase");
        let mob = server.entities.get_mob(mob_id).unwrap();
        let (length, width) = mob.size();
        let path = find_path(
            &server.world,
            mob.x,
            mob.y,
            mob.z,
            length,
            width,
            10.5,
            65.0,
            0.5,
            16.0,
        );
        println!("PATH {path:?}");
        for x in 0..12 {
            println!(
                "BLOCK {x} {} {}",
                server.world.get_block(x, 63, 0),
                server.world.get_block(x, 64, 0)
            );
        }
    }

    /// The golden traces of the reference (`tests/golden/poseidon_mob_traces.txt`).
    fn golden_mob_traces() -> Vec<(String, Vec<String>)> {
        let golden = include_str!("../tests/golden/poseidon_mob_traces.txt");
        let mut scenarios: Vec<(String, Vec<String>)> = Vec::new();
        let mut current: Option<(String, Vec<String>)> = None;
        for line in golden.lines() {
            if let Some(name) = line.strip_prefix("=== scenario ") {
                if let Some(entry) = current.take() {
                    scenarios.push(entry);
                }
                current = Some((name.trim_end_matches(" ===").to_string(), Vec::new()));
            } else if let Some(entry) = current.as_mut() {
                if !line.starts_with('#') && !line.trim().is_empty() {
                    entry.1.push(line.to_string());
                }
            }
        }
        if let Some(entry) = current {
            scenarios.push(entry);
        }
        scenarios
    }

    /// The draws of the shared random of the entities, tick by tick.
    ///
    /// The reference prints the same trace with the option
    /// `-Dflint.trace.rng=1` (`tools/poseidon_mobs/PoseidonMobTrace.java`).
    #[test]
    #[ignore = "temp debug"]
    fn dump_rng_trace() {
        let scenario = std::env::var("MOB_SCENARIO").unwrap_or_else(|_| "skeleton".to_string());
        let ticks: usize = std::env::var("MOB_TICKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let (mut server, _mob_id) = mob_scenario_server(&scenario);
        server.entity_random.trace = true;
        for tick in 0..ticks {
            server.entity_random.trace_values.clear();
            server.tick_mobs();
            println!(
                "RNG t={tick} n={} {:?}",
                server.entity_random.trace_values.len(),
                server.entity_random.trace_values
            );
        }
    }

    #[test]
    #[ignore = "temp debug"]
    fn mob_trace_dump() {
        let scenario = std::env::var("MOB_SCENARIO").unwrap_or_else(|_| "flat".to_string());
        let ticks: usize = std::env::var("MOB_TICKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let (mut server, mob_id) = mob_scenario_server(&scenario);
        for tick in 0..ticks {
            server.tick_mobs();
            if let Some(mob) = server.entities.get_mob(mob_id) {
                let mut sb = format!("TRACEPATH {tick} ");
                for (i, p) in mob.path.iter().enumerate() {
                    sb.push_str(&format!("[{},{},{}]", p.0, p.1, p.2));
                    if i == mob.path_index {
                        sb.push('*');
                    }
                }
                println!("{sb}");
                println!(
                    "IDLE {tick} a={} jump={} posch={} og={} moty={} step={} y={} path={} idx={}",
                    mob.idle_sound_ticks,
                    mob.jump_flag,
                    mob.position_changed,
                    mob.on_ground,
                    mob.mot_y,
                    mob.step_offset,
                    mob.y,
                    mob.path.len(),
                    mob.path_index
                );
            }
        }
    }

    #[test]
    #[ignore = "the long comparison with the traces of the reference"]
    fn the_mob_movement_matches_the_poseidon_reference() {
        let golden = golden_mob_traces();
        let mut failures = Vec::new();
        for (name, expected) in &golden {
            let trace = mob_scenario_trace(name, expected.len());
            for (tick, (actual, expected)) in trace.iter().zip(expected.iter()).enumerate() {
                if actual != expected {
                    failures.push(format!(
                        "{name}: diverges at tick {tick}: {actual} != {expected}"
                    ));
                    break;
                }
            }
            if trace.len() != expected.len() {
                failures.push(format!(
                    "{name}: the trace has {} ticks, the reference has {}",
                    trace.len(),
                    expected.len()
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "the traces of the port differ from the reference:\n{}",
            failures.join("\n")
        );
    }

    fn test_server() -> Server {
        let mut config = Config::default();
        config.world.save_enabled = false;
        config.world.view_distance = 2;
        config.server.allow_nether = true;
        let (_tx, rx) = mpsc::channel(8);
        Server::new(config, rx, Arc::new(AtomicBool::new(true)))
    }

    #[test]
    fn portal_travel_round_trips_between_overworld_and_nether() {
        let mut server = test_server();

        // Offline worlds need their chunks present before block writes/portal
        // scans touch them.
        for cx in -1..=1 {
            for cz in -1..=1 {
                server.world.ensure_chunk_base(ChunkPos::new(cx, cz));
                server
                    .nether
                    .as_mut()
                    .unwrap()
                    .ensure_chunk_base(ChunkPos::new(cx, cz));
            }
        }

        // 4x5 obsidian frame with a 2x3 interior at z = 0.
        for x in 0..=3 {
            for y in 63..=67 {
                let edge = x == 0 || x == 3 || y == 63 || y == 67;
                server.world.set_block(
                    x,
                    y,
                    0,
                    if edge {
                        blocks::OBSIDIAN
                    } else {
                        blocks::PORTAL
                    },
                );
            }
        }

        let eid = server
            .entities
            .insert_player(1, "Tester".to_string(), 2.0, 64.0, 0.5);
        {
            let player = server.entities.get_player_mut(eid).unwrap();
            player.dimension = 0;
            player.chunk_queue.clear();
        }

        server.travel_player(eid);
        {
            let player = server.entities.get_player(eid).unwrap();
            assert_eq!(player.dimension, -1);
        }

        // A destination portal must have been created near the scaled
        // coordinates (2 * 0.125, 0.5 * 0.125).
        let nether = server.nether.as_mut().unwrap();
        let mut found_portal = false;
        for x in -20..=20 {
            for z in -20..=20 {
                for y in 60..=80 {
                    if nether.get_block(x, y, z) == blocks::PORTAL {
                        found_portal = true;
                    }
                }
            }
        }
        assert!(found_portal, "nether portal was not created");

        server.travel_player(eid);
        {
            let player = server.entities.get_player(eid).unwrap();
            assert_eq!(player.dimension, 0);
            // Snapped back onto the bottom-center of the original portal.
            assert_eq!((player.x, player.y, player.z), (2.0, 64.5, 0.5));
        }
    }

    /// Build an obsidian-framed portal with a 2x3 interior at `z = 0` and a
    /// player standing in it.
    fn build_test_portal(server: &mut Server, eid: i32) {
        for x in 0..=3 {
            for y in 63..=67 {
                let edge = x == 0 || x == 3 || y == 63 || y == 67;
                server.world.set_block(
                    x,
                    y,
                    0,
                    if edge {
                        blocks::OBSIDIAN
                    } else {
                        blocks::PORTAL
                    },
                );
            }
        }
        if let Some(player) = server.entities.get_player_mut(eid) {
            player.dimension = 0;
            player.chunk_queue.clear();
            player.x = 2.0;
            player.y = 64.0;
            player.z = 0.5;
        }
    }

    #[test]
    fn portal_cooldown_blocks_immediate_re_entry() {
        let mut server = test_server();
        for cx in -1..=1 {
            for cz in -1..=1 {
                server.world.ensure_chunk_base(ChunkPos::new(cx, cz));
                server
                    .nether
                    .as_mut()
                    .unwrap()
                    .ensure_chunk_base(ChunkPos::new(cx, cz));
            }
        }

        let eid = server
            .entities
            .insert_player(1, "Tester".to_string(), 2.0, 64.0, 0.5);
        build_test_portal(&mut server, eid);

        // Java `EntityHuman.m_()`: the portal timer only advances while the
        // player stands in a portal. `F += 0.0125F` is a `float`, so the
        // accumulator only reaches 1.0 on the 81st tick.
        for _ in 0..80 {
            server.tick_player_portals();
        }
        {
            let player = server.entities.get_player(eid).unwrap();
            assert_eq!(
                player.dimension, 0,
                "travel must not happen before the portal timer fills"
            );
        }

        server.tick_player_portals();
        {
            let player = server.entities.get_player(eid).unwrap();
            assert_eq!(player.dimension, -1, "travel must happen when F >= 1.0");
        }
    }

    #[test]
    fn live_server_spawns_mobs_around_a_player() {
        let dir = std::env::temp_dir().join("flintcore_live_spawn_check");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default();
        config.world.save_enabled = true;
        config.world.level_name = dir.to_string_lossy().to_string();
        config.world.view_distance = 4;
        config.world.keep_spawn_in_memory = false;
        config.server.allow_nether = false;
        let (_tx, rx) = mpsc::channel(8);
        let mut server = Server::new(config, rx, Arc::new(AtomicBool::new(true)));
        server
            .entities
            .insert_player(1, "Tester".to_string(), 0.5, 64.0, 0.5);
        for _ in 0..1200 {
            server.tick(0);
        }
        let mobs: Vec<_> = server.entities.iter_mobs().map(|m| m.mob_type).collect();
        println!("LIVE: {} mobs: {:?}", mobs.len(), mobs);
        let animals = mobs.iter().filter(|m| m.is_animal()).count();
        let monsters = mobs.iter().filter(|m| m.is_monster()).count();
        println!("LIVE: animals={animals} monsters={monsters}");
        assert!(!mobs.is_empty(), "no mobs spawned in the live world");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server with a flat grass field and a player that stands at the
    /// world spawn.
    ///
    /// The whole area is flattened, so that the mobs always spawn on the
    /// surface at `y = 64` and the test does not depend on the terrain.
    fn spawn_test_server(spawning: crate::config::SpawningConfig) -> Server {
        let mut config = Config::default();
        config.world.save_enabled = false;
        config.world.view_distance = 2;
        config.server.allow_nether = false;
        config.spawning = spawning;
        let (_tx, rx) = mpsc::channel(8);
        let mut server = Server::new(config, rx, Arc::new(AtomicBool::new(true)));
        for cx in 0..=6 {
            for cz in 0..=6 {
                server.world.ensure_chunk_base(ChunkPos::new(cx, cz));
            }
        }
        for x in 0..112 {
            for z in 0..112 {
                server.world.set_block(x, 63, z, blocks::GRASS);
                server.world.set_block(x, 64, z, blocks::AIR);
                server.world.set_block(x, 65, z, blocks::AIR);
            }
        }
        // The sky light of the base chunks, so that the animals can spawn.
        for cx in 0..=6 {
            for cz in 0..=6 {
                if let Some(chunk) = server.world.get_chunk_mut(ChunkPos::new(cx, cz)) {
                    chunk.init_lighting();
                }
            }
        }
        while server.world.java_do_lighting() {}
        server
            .entities
            .insert_player(1, "Tester".to_string(), 0.0, 64.0, 0.0);
        server
    }

    #[test]
    fn the_spider_jockey_spawns_a_skeleton_rider() {
        let mut spawning = crate::config::SpawningConfig {
            chunk_sample_divisor: 1,
            ..Default::default()
        };
        spawning.mobs.get_mut("spider").unwrap().jockey_chance = 1;
        for name in [
            "zombie", "skeleton", "creeper", "slime", "sheep", "pig", "chicken", "cow", "wolf",
            "squid",
        ] {
            spawning.mobs.get_mut(name).unwrap().enabled = false;
        }
        let mut server = spawn_test_server(spawning);
        for _ in 0..50 {
            server.tick_mob_spawner();
        }

        // A spider carries a skeleton, like the Java `applyPostSpawnExtras`
        // does (`entityskeleton.mount(entityliving)`).
        let mut jockeys = 0;
        for mob in server.entities.iter_mobs() {
            if mob.mob_type != MobType::Spider {
                continue;
            }
            let Some(rider_id) = mob.passenger_id else {
                continue;
            };
            let rider = server.entities.get_mob(rider_id).unwrap();
            assert_eq!(rider.mob_type, MobType::Skeleton);
            assert_eq!(rider.vehicle_id, Some(mob.entity_id));
            jockeys += 1;
        }
        assert!(jockeys > 0, "the spider jockey did not spawn");
    }

    #[test]
    fn a_dead_session_does_not_block_the_name() {
        let mut server = test_server();

        // A player whose connection is gone: the writer was closed, so the
        // socket is dead even though the disconnect event is still queued.
        let (tx, rx) = mpsc::unbounded_channel::<Outbound>();
        drop(rx);
        let entity_id = server
            .entities
            .insert_player(1, "Ghost".to_string(), 0.0, 64.0, 0.0);
        server.connections.insert(
            1,
            Connection {
                sender: tx,
                address: "127.0.0.1:25565".parse().unwrap(),
                state: ConnectionState::Playing,
                username: Some("Ghost".to_string()),
                entity_id,
            },
        );
        assert_eq!(server.entities.player_count(), 1);

        // The ghost is removed, the name and the slot are free again.
        server.purge_dead_players();
        assert_eq!(server.entities.player_count(), 0);
        assert!(server.entities.get_by_conn(1).is_none());
    }

    #[test]
    fn a_live_session_is_kept() {
        let mut server = test_server();

        let (tx, _rx) = mpsc::unbounded_channel::<Outbound>();
        let entity_id = server
            .entities
            .insert_player(1, "Alive".to_string(), 0.0, 64.0, 0.0);
        server.connections.insert(
            1,
            Connection {
                sender: tx,
                address: "127.0.0.1:25565".parse().unwrap(),
                state: ConnectionState::Playing,
                username: Some("Alive".to_string()),
                entity_id,
            },
        );

        server.purge_dead_players();
        assert_eq!(server.entities.player_count(), 1);
        assert!(server.entities.get_by_conn(1).is_some());
    }

    /// A server with the standard input console attached and a sink that
    /// records everything the console prints.
    fn console_server() -> (
        Server,
        std::sync::mpsc::Sender<String>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        let mut server = test_server();
        let (tx, rx) = std::sync::mpsc::channel();
        server.attach_console(rx);
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_output = Arc::clone(&output);
        server.set_console_sink(Box::new(move |message: &str| {
            sink_output.lock().unwrap().push(message.to_string());
        }));
        (server, tx, output)
    }

    #[test]
    fn console_runs_commands_without_a_connected_player() {
        let (mut server, tx, output) = console_server();
        assert!(server.console_attached());

        tx.send("list".to_string()).unwrap();
        server.process_console_commands();

        let output = output.lock().unwrap();
        assert!(
            output
                .iter()
                .any(|line| line.contains("Connected players:")),
            "console output was {output:?}"
        );
    }

    #[test]
    fn console_reports_unknown_commands() {
        let (mut server, tx, output) = console_server();

        tx.send("definitely-not-a-command".to_string()).unwrap();
        server.process_console_commands();

        let output = output.lock().unwrap();
        assert!(
            output
                .iter()
                .any(|line| line.contains("Unknown command. Type \"help\" for help.")),
            "console output was {output:?}"
        );
    }

    #[test]
    fn console_accepts_commands_with_and_without_a_leading_slash() {
        let (mut server, tx, _output) = console_server();

        tx.send("time set 1000".to_string()).unwrap();
        server.process_console_commands();
        assert_eq!(server.world.world_time(), 1000);

        tx.send("/time set 2000".to_string()).unwrap();
        server.process_console_commands();
        assert_eq!(server.world.world_time(), 2000);
    }

    #[test]
    fn console_stop_shuts_the_server_down() {
        let (mut server, tx, _output) = console_server();

        tx.send("stop".to_string()).unwrap();
        server.process_console_commands();

        assert!(!server.running.load(Ordering::Relaxed));
    }

    #[test]
    fn console_broadcasts_reach_the_players() {
        let (mut server, tx, output) = console_server();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        server.connections.insert(
            1,
            Connection {
                sender: out_tx,
                address: "127.0.0.1:25565".parse().unwrap(),
                state: ConnectionState::Playing,
                username: Some("Tester".to_string()),
                entity_id: 0,
            },
        );
        server
            .entities
            .insert_player(1, "Tester".to_string(), 0.0, 64.0, 0.0);

        tx.send("say hello".to_string()).unwrap();
        server.process_console_commands();

        let mut player_saw_broadcast = false;
        while let Ok(outbound) = out_rx.try_recv() {
            if let Outbound::Packet(Clientbound::Chat(chat)) = outbound {
                if chat.message.contains("[Server] hello") {
                    player_saw_broadcast = true;
                }
            }
        }
        assert!(player_saw_broadcast, "the broadcast must reach the players");

        // The console prints the broadcast without the colour codes.
        let output = output.lock().unwrap();
        assert!(
            output.iter().any(|line| line.contains("[Server] hello")),
            "console output was {output:?}"
        );
        assert!(
            !output.iter().any(|line| line.contains('\u{00A7}')),
            "console output must not contain colour codes"
        );
    }

    #[test]
    fn console_commands_run_during_the_tick() {
        let (mut server, tx, _output) = console_server();

        tx.send("time set 4321".to_string()).unwrap();
        server.tick(0);

        assert_eq!(server.world.world_time(), 4321);
    }
}
