use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::config::{FixesConfig, MechanicsConfig, SpawningConfig, WorldConfig, WorldgenConfig};
use crate::gameplay::blocks;
use crate::net::packets::chunk::{MapChunk, PreChunk};
use crate::util::rng::JavaRandom;
use crate::util::{Error, Result};
use crate::world::biome::{Biome, WorldChunkManager};
use crate::world::chunk::{Chunk, ChunkPos, ChunkState, CHUNK_HEIGHT};
use crate::world::decorate::GenWorld;
use crate::world::generator::{Generator, HellGenerator};
use crate::world::lighting::{LightType, LightingWorld};
use crate::world::storage::{ChunkLoader, LevelData, RegionFileCache, SessionLock};

thread_local! {
    static GENERATOR: RefCell<Option<(i64, Generator)>> = const { RefCell::new(None) };
}

/// Debug tracing of the Java block-write pipeline. Set `FLINTCORE_TRACE` to
/// `x0,y0,z0,x1,y1,z1` to print every pipeline event inside that box. The
/// output matches the instrumented Poseidon reference dump byte for byte and
/// is used to locate divergences between the two generators.
pub(crate) type TraceBox = (i32, i32, i32, i32, i32, i32);

pub(crate) fn trace_region() -> Option<TraceBox> {
    use std::sync::OnceLock;
    static REGION: OnceLock<Option<TraceBox>> = OnceLock::new();
    *REGION.get_or_init(|| {
        std::env::var("FLINTCORE_TRACE").ok().and_then(|s| {
            let p: Vec<i32> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if p.len() == 6 {
                Some((p[0], p[1], p[2], p[3], p[4], p[5]))
            } else {
                None
            }
        })
    })
}

fn trace_in(x: i32, y: i32, z: i32) -> bool {
    trace_region()
        .map(|(x0, y0, z0, x1, y1, z1)| {
            x >= x0 && x <= x1 && y >= y0 && y <= y1 && z >= z0 && z <= z1
        })
        .unwrap_or(false)
}

fn trace_g_in(x: i32, z: i32) -> bool {
    trace_region()
        .map(|(x0, _, z0, x1, _, z1)| x >= x0 && x <= x1 && z >= z0 && z <= z1)
        .unwrap_or(false)
}

macro_rules! trace_a4 {
    ($x:expr, $y:expr, $z:expr, $old:expr, $new:expr) => {
        if trace_in($x, $y, $z) {
            println!("A4 {} {} {} {}->{}", $x, $y, $z, $old, $new);
        }
    };
}

macro_rules! trace_a5 {
    ($x:expr, $y:expr, $z:expr, $old:expr, $oldd:expr, $new:expr, $newd:expr) => {
        if trace_in($x, $y, $z) {
            println!(
                "A5 {} {} {} {}.{}->{}.{}",
                $x, $y, $z, $old, $oldd, $new, $newd
            );
        }
    };
}

macro_rules! trace_b {
    ($x:expr, $y:expr, $z:expr, $oldd:expr, $newd:expr) => {
        if trace_in($x, $y, $z) {
            println!("B {} {} {} {}->{}", $x, $y, $z, $oldd, $newd);
        }
    };
}

macro_rules! trace_g {
    ($x:expr, $z:expr, $oldh:expr, $newh:expr) => {
        if trace_g_in($x, $z) {
            println!("G {} {} {}->{}", $x, $z, $oldh, $newh);
        }
    };
}

macro_rules! trace_phys {
    ($x:expr, $y:expr, $z:expr, $l:expr, $id:expr) => {
        if trace_in($x, $y, $z) {
            println!("PHYS {} {} {} l={} id={}", $x, $y, $z, $l, $id);
        }
    };
}

macro_rules! trace_fa {
    ($x:expr, $y:expr, $z:expr, $id:expr, $data:expr) => {
        if trace_in($x, $y, $z) {
            println!("FA {} {} {} id={} d={}", $x, $y, $z, $id, $data);
        }
    };
}

macro_rules! trace_conv {
    ($x:expr, $y:expr, $z:expr, $oldd:expr, $new:expr) => {
        if trace_in($x, $y, $z) {
            println!("CONV {} {} {} d={} ->{}", $x, $y, $z, $oldd, $new);
        }
    };
}

macro_rules! trace_rng {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("RNG {} {} {}", $x, $y, $z);
        }
    };
}

macro_rules! trace_rng3 {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("RNG3 {} {} {}", $x, $y, $z);
        }
    };
}

macro_rules! trace_j {
    ($x:expr, $y:expr, $z:expr, $c:expr, $b:expr) => {
        if trace_in($x, $y, $z) {
            println!(
                "J {} {} {} c={},{},{},{} b={}{}{}{}",
                $x,
                $y,
                $z,
                $c[0],
                $c[1],
                $c[2],
                $c[3],
                $b[0] as i32,
                $b[1] as i32,
                $b[2] as i32,
                $b[3] as i32
            );
        }
    };
}

macro_rules! trace_flow {
    ($x:expr, $y:expr, $z:expr, $l:expr) => {
        if trace_in($x, $y, $z) {
            println!("FLOW {} {} {} l={}", $x, $y, $z, $l);
        }
    };
}

macro_rules! trace_down {
    ($x:expr, $y:expr, $z:expr) => {
        if trace_in($x, $y, $z) {
            println!("DOWN {} {} {}", $x, $y, $z);
        }
    };
}

pub(crate) fn with_generator<R>(seed: i64, f: impl FnOnce(&mut Generator) -> R) -> R {
    GENERATOR.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.as_ref().map(|(s, _)| *s) != Some(seed) {
            *opt = Some((seed, Generator::new(seed)));
        }
        let (_, generator) = opt.as_mut().unwrap();
        f(generator)
    })
}

pub struct World {
    config: WorldConfig,
    /// Java `WorldProvider.dimension`: 0 = overworld, -1 = Nether.
    pub dimension: i32,
    /// Cached `WorldProvider.e` (`isHell`): disables sky light entirely.
    pub is_hell: bool,
    pub allow_nether: bool,
    pub spawning: SpawningConfig,
    pub worldgen: WorldgenConfig,
    pub mechanics: MechanicsConfig,
    pub fixes: FixesConfig,
    world_dir: PathBuf,
    /// Java `ChunkProviderHell` instance. Its `Random` is stateful and is NOT
    /// re-seeded by `getChunkAt(populate)`, so it must live next to the world.
    hell_generator: Option<HellGenerator>,
    pub(crate) chunks: HashMap<ChunkPos, Chunk>,
    pub(crate) packet_cache: RefCell<HashMap<ChunkPos, (PreChunk, MapChunk)>>,
    chunk_mgr: RefCell<WorldChunkManager>,
    storage: Option<RefCell<RegionFileCache>>,
    pub(crate) level_data: RefCell<LevelData>,
    _session_lock: Option<SessionLock>,
    pub(crate) suppress_physics: bool,
    /// Java `World.a`: while true, `World.c`/`World.d` executes scheduled
    /// block ticks immediately (recursively). `WorldGenLiquids` enables it
    /// around the direct fluid tick, which cascades through flowed blocks.
    insta_tick: bool,
    /// Java `World.random` (a fresh `java.util.Random`) — only reachable during
    /// instant-tick cascades (lava flow randomness). Kept here so the port
    /// mirrors the reference implementation.
    pub(crate) world_random: JavaRandom,
    /// `BlockFlowing.b` — the per-direction flow flags returned by `j()`. In
    /// Java this is a mutable field on the shared `BlockFlowing` singleton:
    /// nested cascade ticks call `j()` again and overwrite it, and the outer
    /// flow loop then reads the clobbered flags. Indexed by [lava][direction].
    fluid_sides: [[bool; 4]; 2],
    /// Live/server choreography state (spawn search, populate triggers,
    /// player framing, unloads) — see `world::live`.
    pub(crate) live: crate::world::live::LiveState,
    /// True when no `level.dat` was found at load time: the server runs the
    /// Java spawn search for such worlds.
    fresh_world: bool,
    /// Java `WorldData.f()` — the world time in ticks. Scheduled block ticks
    /// are stamped with `time + delay` and the processor only runs entries
    /// whose time has arrived.
    pub(crate) live_time: i64,
    /// Java `World.E` — scheduled block ticks ordered by (time, insertion seq).
    pub(crate) scheduled: BTreeSet<ScheduledTick>,
    /// Java `World.F` — dedup set keyed by `(x, y, z, id)` (not by time).
    pub(crate) scheduled_set: HashSet<(i32, i32, i32, u8)>,
    /// Java `NextTickListEntry.f` — the global insertion counter.
    pub(crate) scheduled_seq: u64,
    /// Java `World.C` — pending `MetadataChunkBlock` light updates.
    pub(crate) light_updates: Vec<MetadataChunkBlock>,
    /// Java `World.A` — recursion depth counter for `World.a(EnumSkyBlock, ...)`.
    pub(crate) light_schedule_depth: i32,
    /// Java `World.M` — recursion depth counter for `World.doLighting()`.
    pub(crate) do_lighting_depth: i32,
    /// Chunks whose light layers were modified by `java_do_lighting` and should be compacted.
    pub(crate) light_dirty_chunks: HashSet<ChunkPos>,
    /// Java `EntityFallingSand` instances in `World.entityList`.
    pub(crate) falling_sands: Vec<FallingSand>,
    /// Java `BlockRedstoneWire.a` — false while evaluating wire indirect power.
    pub(crate) wire_can_provide_power: bool,
    /// Java `BlockPiston.b` — true while a piston is extending or retracting.
    pub(crate) piston_updating: bool,
    /// Pending 2-tick piston block confirmations (`(tick_time, x, y, z)`).
    pub(crate) pending_piston_syncs: Vec<(i64, i32, i32, i32)>,
    /// Java `BlockRedstoneTorch.b` — burnout toggle history.
    pub(crate) torch_burnout_list: Vec<crate::world::redstone::RedstoneUpdateInfo>,
    /// Events emitted by redstone/pistons/dispensers/random ticks for `Server`.
    pub(crate) world_events: Vec<crate::world::redstone::WorldEvent>,
    /// Java `World.g` LCG state for `World.j()` random block ticks.
    pub(crate) random_tick_lcg: i32,
    /// Separate RNG for live random block ticks so `world_random` stays untouched for parity tests.
    pub(crate) tick_random: JavaRandom,
    /// Detector rails currently occupied by at least one `EntityMinecart` (`BlockMinecartDetector.f`).
    pub active_detector_rails: HashSet<(i32, i32, i32)>,
    /// Pressure plates currently occupied by an entity (`BlockPressurePlate.g`).
    pub active_pressure_plates: HashSet<(i32, i32, i32)>,
    /// Optional dedicated Rayon pools from `world.gen_workers` /
    /// `world.compression_workers` (`0` means “use the global pool”).
    gen_pool: Option<rayon::ThreadPool>,
    compression_pool: Option<rayon::ThreadPool>,
}

/// Active `EntityFallingSand` instance ticked by `World.cleanUp()`.
#[derive(Clone, Debug)]
pub(crate) struct FallingSand {
    pub x: i32,
    pub z: i32,
    pub loc_y: f64,
    pub mot_y: f64,
    pub id: u8,
    pub ticks: i32,
    pub dead: bool,
}

/// Java `MetadataChunkBlock` — one queued light-propagation box in `World.C`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MetadataChunkBlock {
    pub kind: LightType,
    pub b: i32,
    pub c: i32,
    pub d: i32,
    pub e: i32,
    pub f: i32,
    pub g: i32,
}

impl MetadataChunkBlock {
    /// Java `MetadataChunkBlock.a(int i, int j, int k, int l, int i1, int j1)`.
    pub fn try_merge(
        &mut self,
        mut i: i32,
        mut j: i32,
        mut k: i32,
        mut l: i32,
        mut i1: i32,
        mut j1: i32,
    ) -> bool {
        if i >= self.b && j >= self.c && k >= self.d && l <= self.e && i1 <= self.f && j1 <= self.g
        {
            return true;
        }
        let b0 = 1;
        if i >= self.b - b0
            && j >= self.c - b0
            && k >= self.d - b0
            && l <= self.e + b0
            && i1 <= self.f + b0
            && j1 <= self.g + b0
        {
            let k1 = self.e - self.b;
            let l1 = self.f - self.c;
            let i2 = self.g - self.d;
            if i > self.b {
                i = self.b;
            }
            if j > self.c {
                j = self.c;
            }
            if k > self.d {
                k = self.d;
            }
            if l < self.e {
                l = self.e;
            }
            if i1 < self.f {
                i1 = self.f;
            }
            if j1 < self.g {
                j1 = self.g;
            }
            let j2 = l - i;
            let k2 = i1 - j;
            let l2 = j1 - k;
            let i3 = k1 * l1 * i2;
            let j3 = j2 * k2 * l2;
            if j3 - i3 <= 2 {
                self.b = i;
                self.c = j;
                self.d = k;
                self.e = l;
                self.f = i1;
                self.g = j1;
                return true;
            }
        }
        false
    }
}

/// One entry of `World.E` (`NextTickListEntry`). Ordering mirrors
/// `compareTo`: primary time, then the insertion counter.
#[derive(Clone, Debug)]
pub(crate) struct ScheduledTick {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: u8,
    pub time: i64,
    pub seq: u64,
}

impl PartialEq for ScheduledTick {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.seq == other.seq
    }
}
impl Eq for ScheduledTick {}
impl PartialOrd for ScheduledTick {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScheduledTick {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.time, self.seq).cmp(&(other.time, other.seq))
    }
}

impl World {
    pub fn new(config: WorldConfig) -> Self {
        Self::new_with_dimension(config, 0)
    }

    /// Create the Nether (`DIM-1`) world. Region files live in
    /// `<level_name>/DIM-1/region`, exactly like `ServerNBTManager`; the
    /// `level.dat` and session lock belong to the overworld.
    pub fn new_nether(config: WorldConfig) -> Self {
        Self::new_with_dimension(config, -1)
    }

    pub fn new_with_dimension(mut config: WorldConfig, dimension: i32) -> Self {
        let level_dir = PathBuf::from(&config.level_name);
        let world_dir = if dimension == -1 {
            level_dir.join("DIM-1")
        } else {
            level_dir
        };
        let is_hell = dimension == -1;
        if is_hell {
            // DIM-1 has no level.dat of its own; its spawn is the origin.
            config.spawn_x = 0;
            config.spawn_y = 64;
            config.spawn_z = 0;
        }

        let mut fresh_world = false;
        let mut lock = None;
        let mut storage = None;
        let level = if is_hell {
            // The Nether never owns `level.dat`; it is constructed with the
            // overworld seed and default level data. Its world spawn is the
            // origin (Java DIM-1 level data starts at (0, 64, 0)).
            fresh_world = true;
            LevelData {
                seed: config.seed,
                spawn_x: 0,
                spawn_y: 64,
                spawn_z: 0,
                level_name: config.level_name.clone(),
                ..LevelData::default()
            }
        } else if config.save_enabled {
            lock = SessionLock::acquire(&world_dir).ok();
            let loaded = LevelData::load_from_dir(&world_dir).ok().flatten();
            if let Some(mut lvl) = loaded {
                config.seed = lvl.seed;
                let custom_spawn = config.override_saved_spawn
                    || !config.auto_find_spawn
                    || config.spawn_x != 0
                    || config.spawn_y != 64
                    || config.spawn_z != 0;
                if custom_spawn {
                    lvl.spawn_x = config.spawn_x;
                    lvl.spawn_y = config.spawn_y;
                    lvl.spawn_z = config.spawn_z;
                } else {
                    config.spawn_x = lvl.spawn_x;
                    config.spawn_y = lvl.spawn_y;
                    config.spawn_z = lvl.spawn_z;
                }
                lvl
            } else {
                fresh_world = true;
                LevelData {
                    seed: config.seed,
                    spawn_x: config.spawn_x,
                    spawn_y: config.spawn_y,
                    spawn_z: config.spawn_z,
                    level_name: config.level_name.clone(),
                    ..LevelData::default()
                }
            }
        } else {
            fresh_world = true;
            LevelData {
                seed: config.seed,
                spawn_x: config.spawn_x,
                spawn_y: config.spawn_y,
                spawn_z: config.spawn_z,
                level_name: config.level_name.clone(),
                ..LevelData::default()
            }
        };

        if config.save_enabled {
            storage = Some(RefCell::new(RegionFileCache::new(&world_dir)));
        }

        let chunk_mgr = RefCell::new(WorldChunkManager::new(config.seed));
        let live = crate::world::live::LiveState::new(config.view_distance);
        let initial_time = level.time;
        let world_seed = config.seed;
        let hell_generator = if is_hell {
            Some(HellGenerator::new(world_seed))
        } else {
            None
        };
        let build_pool = |workers: usize, prefix: &str| {
            if workers == 0 {
                return None;
            }
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .thread_name({
                    let prefix = prefix.to_string();
                    move |i| format!("{prefix}-{i}")
                })
                .build()
                .ok()
        };
        let gen_pool = build_pool(config.gen_workers, "flintcore-gen");
        let compression_pool = build_pool(config.compression_workers, "flintcore-compress");

        Self {
            config,
            dimension,
            is_hell,
            allow_nether: true,
            spawning: SpawningConfig::default(),
            worldgen: WorldgenConfig::default(),
            mechanics: MechanicsConfig::default(),
            fixes: FixesConfig::default(),
            world_dir,
            hell_generator,
            chunks: HashMap::new(),
            packet_cache: RefCell::new(HashMap::new()),
            chunk_mgr,
            storage,
            level_data: RefCell::new(level),
            _session_lock: lock,
            suppress_physics: false,
            insta_tick: false,
            world_random: JavaRandom::new(0),
            fluid_sides: [[false; 4]; 2],
            live,
            fresh_world,
            live_time: initial_time,
            scheduled: BTreeSet::new(),
            scheduled_set: HashSet::new(),
            scheduled_seq: 0,
            light_updates: Vec::new(),
            light_schedule_depth: 0,
            do_lighting_depth: 0,
            light_dirty_chunks: HashSet::new(),
            falling_sands: Vec::new(),
            wire_can_provide_power: true,
            piston_updating: false,
            pending_piston_syncs: Vec::new(),
            torch_burnout_list: Vec::new(),
            world_events: Vec::new(),
            random_tick_lcg: (world_seed as i32)
                .wrapping_mul(3)
                .wrapping_add(1_013_904_223),
            tick_random: JavaRandom::new(world_seed ^ 0x5DEECE66D),
            active_detector_rails: HashSet::new(),
            active_pressure_plates: HashSet::new(),
            gen_pool,
            compression_pool,
        }
    }

    /// True when the world was created rather than loaded (`level.dat` was
    /// absent): the server performs the Java spawn search for such worlds.
    pub fn is_fresh_world(&self) -> bool {
        self.fresh_world
    }

    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    pub fn world_dir(&self) -> &Path {
        &self.world_dir
    }

    pub fn world_time(&self) -> i64 {
        self.level_data.borrow().time
    }

    pub fn increment_time(&self) -> i64 {
        let mut data = self.level_data.borrow_mut();
        data.time += 1;
        data.time
    }

    pub fn seed(&self) -> i64 {
        self.config.seed
    }

    /// Java portal search reads every column in a 257x257 square through
    /// `World.getTypeId`, which synchronously loads (or generates) each chunk
    /// it touches. The live server's `get_block` resolves missing chunks as
    /// empty air instead, so a travelling player must prefetch the search
    /// region before `find_or_create_portal` runs.
    pub fn ensure_portal_search_chunks(&mut self, x: f64, z: f64) {
        if !self.live.active {
            return;
        }
        let bx = x.floor() as i32;
        let bz = z.floor() as i32;
        let min_cx = (bx - 128) >> 4;
        let max_cx = (bx + 128) >> 4;
        let min_cz = (bz - 128) >> 4;
        let max_cz = (bz + 128) >> 4;
        for cx in min_cx..=max_cx {
            for cz in min_cz..=max_cz {
                self.server_get_chunk_at(ChunkPos::new(cx, cz));
            }
        }
    }

    pub fn spawn_point(&self) -> (i32, i32, i32) {
        (
            self.config.spawn_x,
            self.config.spawn_y,
            self.config.spawn_z,
        )
    }

    pub fn safe_spawn_point(&mut self) -> (i32, i32, i32) {
        let (x, default_y, z) = self.spawn_point();
        let chunk_pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active {
            self.server_get_chunk_at(chunk_pos);
        } else {
            self.populate_chunk(chunk_pos);
        }
        if !self.config.teleport_to_highest_safe_block {
            return (x, default_y.clamp(1, 127), z);
        }
        if default_y != 64 {
            let mut y = default_y.clamp(1, 126);
            while y < 126
                && (blocks::is_solid(self.get_block(x, y, z))
                    || blocks::is_solid(self.get_block(x, y + 1, z)))
            {
                y += 1;
            }
            return (x, y, z);
        }
        let ground = self.highest_solid_y(x, z);
        let y = if ground > 0 {
            ground
        } else {
            default_y.max(64)
        };
        (x, y, z)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    pub fn get_chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn get_chunk_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.chunks.get_mut(&pos)
    }

    pub fn ensure_chunk_base(&mut self, pos: ChunkPos) -> &mut Chunk {
        if !self.chunks.contains_key(&pos) {
            let loaded_chunk = if let Some(storage) = &self.storage {
                match storage.borrow_mut().read_chunk(pos.x, pos.z) {
                    Ok(Some(nbt)) => match ChunkLoader::chunk_from_nbt(&nbt) {
                        Ok(c) => Some(c),
                        Err(err) => {
                            if self.config.regenerate_corrupt_chunks {
                                crate::util::log::warn(&format!(
                                    "regenerating corrupt chunk ({}, {}): {err}",
                                    pos.x, pos.z
                                ));
                            }
                            None
                        }
                    },
                    _ => None,
                }
            } else {
                None
            };

            let chunk = match loaded_chunk {
                Some(c) => c,
                None => self.generate_base_chunk_inner(pos),
            };
            self.chunks.insert(pos, chunk);
        }
        self.chunks.get_mut(&pos).unwrap()
    }

    /// Base generation that does not insert into the chunk map. The Nether
    /// keeps a per-world `ChunkProviderHell`-equivalent whose RNG state is
    /// carried across calls.
    fn generate_base_chunk_inner(&mut self, pos: ChunkPos) -> Chunk {
        if self.is_hell {
            let mut generator = self
                .hell_generator
                .take()
                .expect("nether world initialized without a hell generator");
            let chunk = generator.generate_base(pos.x, pos.z);
            self.hell_generator = Some(generator);
            chunk
        } else {
            let seed = self.config.seed;
            with_generator(seed, |gen| gen.generate_base(pos.x, pos.z))
        }
    }

    /// Dispatch the populator for the active dimension. The overworld uses the
    /// thread-local generator (reseeded by `Generator::populate`); the Nether
    /// continues the stateful `ChunkProviderHell` RNG.
    pub(crate) fn populate_center(&mut self, pos: ChunkPos) {
        if self.is_hell {
            let mut generator = self.hell_generator.take().unwrap();
            generator.populate(self, pos.x, pos.z);
            self.hell_generator = Some(generator);
        } else {
            let seed = self.config.seed;
            with_generator(seed, |gen| {
                gen.populate(self, pos.x, pos.z);
            });
        }
    }

    pub fn populate_chunk(&mut self, pos: ChunkPos) {
        if let Some(chunk) = self.chunks.get(&pos) {
            if chunk.state == ChunkState::Populated {
                return;
            }
        }

        // Ensure surrounding chunks have base terrain generated
        for dz in -2..=2 {
            for dx in -2..=2 {
                self.ensure_chunk_base(ChunkPos::new(pos.x + dx, pos.z + dz));
            }
        }

        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.state = ChunkState::Populating;
        }

        self.populate_center(pos);

        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.state = ChunkState::Populated;
            chunk.dirty = true;
            chunk.compact();
        }
        self.packet_cache.borrow_mut().remove(&pos);
    }

    /// Base-generate one chunk through the shared generator context. Java
    /// base generation writes through `Chunk.a(...)`, which sets `o = true`
    /// (dirty), so the chunk is flushed by the world save even when it is
    /// never populated (spawn-search candidates, prep edges).
    pub(crate) fn generate_base_chunk(&mut self, pos: ChunkPos) -> Chunk {
        let mut chunk = self.generate_base_chunk_inner(pos);
        if self.live.active && !self.is_hell {
            // `Chunk.initLighting` runs inside `ChunkProviderGenerate.getOrCreateChunk`
            // before the chunk is inserted into `ChunkProviderServer.chunks`:
            // its `this.c(j, k)` loop queues sky-light updates against already
            // loaded neighbor columns.
            for j in 0..16usize {
                for k in 0..16usize {
                    let h = chunk.height_map[(k << 4) | j] as i32;
                    let wx = pos.x * 16 + j as i32;
                    let wz = pos.z * 16 + k as i32;
                    self.chunk_relight_neighbor(wx - 1, wz, h);
                    self.chunk_relight_neighbor(wx + 1, wz, h);
                    self.chunk_relight_neighbor(wx, wz - 1, h);
                    self.chunk_relight_neighbor(wx, wz + 1, h);
                }
            }
        }
        chunk.compact();
        chunk.dirty = true;
        chunk
    }

    /// Try to load a chunk from the region storage; `None` when absent or
    /// when saving is disabled.
    pub(crate) fn try_load_chunk_from_storage(&mut self, pos: ChunkPos) -> Option<Chunk> {
        let storage = self.storage.as_ref()?;
        match storage.borrow_mut().read_chunk(pos.x, pos.z) {
            Ok(Some(nbt)) => {
                let mut c = ChunkLoader::chunk_from_nbt(&nbt).ok()?;
                c.compact();
                Some(c)
            }
            _ => None,
        }
    }

    /// Save one chunk to storage right away (used by chunk unloading).
    pub(crate) fn write_chunk_to_storage(&mut self, pos: ChunkPos) -> Result<()> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        let Some(chunk) = self.chunks.get(&pos) else {
            return Ok(());
        };
        if !chunk.dirty {
            return Ok(());
        }
        let time = self.level_data.borrow().time;
        let nbt = ChunkLoader::chunk_to_nbt(chunk, time);
        storage.borrow_mut().write_chunk(pos.x, pos.z, &nbt)?;
        self.chunks.get_mut(&pos).unwrap().dirty = false;
        Ok(())
    }

    /// Java `WorldData.setSpawn` plus the in-memory `config` copy.
    pub(crate) fn set_spawn_xyz(&mut self, x: i32, y: i32, z: i32) {
        self.config.spawn_x = x;
        self.config.spawn_y = y;
        self.config.spawn_z = z;
        let mut level = self.level_data.borrow_mut();
        level.spawn_x = x;
        level.spawn_y = y;
        level.spawn_z = z;
    }

    pub fn save_all(&mut self) -> Result<()> {
        if let Some(storage) = &self.storage {
            let time = self.level_data.borrow().time;
            for (&pos, chunk) in &mut self.chunks {
                if chunk.dirty {
                    let nbt = ChunkLoader::chunk_to_nbt(chunk, time);
                    storage.borrow_mut().write_chunk(pos.x, pos.z, &nbt)?;
                    chunk.dirty = false;
                }
            }
            storage.borrow_mut().flush_all()?;
            if self.dimension == 0 {
                self.level_data.borrow_mut().save_to_dir(&self.world_dir)?;
            }
        }
        Ok(())
    }

    /// Java `ChunkProviderServer.saveChunks(false, null)` — save up to `max_chunks` dirty chunks per call.
    pub fn save_chunks_incremental(&mut self, max_chunks: usize) -> Result<usize> {
        let mut saved = 0usize;
        if let Some(storage) = &self.storage {
            let time = self.level_data.borrow().time;
            for (&pos, chunk) in &mut self.chunks {
                if chunk.dirty {
                    let nbt = ChunkLoader::chunk_to_nbt(chunk, time);
                    storage.borrow_mut().write_chunk(pos.x, pos.z, &nbt)?;
                    chunk.dirty = false;
                    saved += 1;
                    if saved >= max_chunks {
                        break;
                    }
                }
            }
            if saved > 0 && self.dimension == 0 {
                self.level_data.borrow_mut().save_to_dir(&self.world_dir)?;
            }
        }
        Ok(saved)
    }

    pub fn generate_base_chunks_parallel(&mut self, positions: &[ChunkPos]) {
        let needed: Vec<ChunkPos> = positions
            .iter()
            .copied()
            .filter(|pos| !self.chunks.contains_key(pos))
            .collect();

        if needed.is_empty() {
            return;
        }

        if self.is_hell {
            for pos in needed {
                let chunk = self.generate_base_chunk_inner(pos);
                self.chunks.insert(pos, chunk);
            }
            return;
        }

        let seed = self.config.seed;
        let generate = || {
            needed
                .par_iter()
                .map(|&pos| {
                    let chunk = with_generator(seed, |gen| gen.generate_base(pos.x, pos.z));
                    (pos, chunk)
                })
                .collect::<Vec<(ChunkPos, Chunk)>>()
        };
        let generated = match &self.gen_pool {
            Some(pool) => pool.install(generate),
            None => generate(),
        };

        for (pos, chunk) in generated {
            self.chunks.insert(pos, chunk);
        }
    }

    pub fn prepare_chunk_packets(&self, pos: ChunkPos) -> Result<(PreChunk, MapChunk)> {
        if let Some(cached) = self.packet_cache.borrow().get(&pos) {
            return Ok(cached.clone());
        }

        let chunk = self
            .chunks
            .get(&pos)
            .ok_or(Error::InvalidState("chunk not loaded for packet prep"))?;
        let pre_chunk = PreChunk {
            x: pos.x,
            z: pos.z,
            mode: true,
        };
        let map_chunk = MapChunk::from_chunk(chunk)?;
        let result = (pre_chunk, map_chunk);
        let mut cache = self.packet_cache.borrow_mut();
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(pos, result.clone());
        Ok(result)
    }

    pub fn compress_chunks_parallel(
        &self,
        positions: &[ChunkPos],
    ) -> Result<Vec<(PreChunk, MapChunk)>> {
        let mut chunks = Vec::with_capacity(positions.len());
        for &pos in positions {
            let chunk = self.chunks.get(&pos).ok_or(Error::InvalidState(
                "chunk not loaded for parallel compression",
            ))?;
            chunks.push(chunk);
        }

        let compress = || {
            chunks
                .par_iter()
                .map(|&chunk| {
                    let pre = PreChunk {
                        x: chunk.pos.x,
                        z: chunk.pos.z,
                        mode: true,
                    };
                    let map = MapChunk::from_chunk(chunk)?;
                    Ok((pre, map))
                })
                .collect::<Result<Vec<_>>>()
        };
        match &self.compression_pool {
            Some(pool) => pool.install(compress),
            None => compress(),
        }
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return blocks::AIR;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active && self.live_resolve_read(pos) {
            // EmptyChunk.getTypeId -> 0.
            return blocks::AIR;
        }
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_block((x & 15) as usize, y as usize, (z & 15) as usize)
        } else {
            blocks::AIR
        }
    }

    /// Java `World.e(int, int)`: the height of the terrain at the column.
    pub fn height_map_at(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        match self.chunks.get(&pos) {
            Some(chunk) => chunk.height_map[(((z & 15) << 4) | (x & 15)) as usize] as i32,
            None => 0,
        }
    }

    /// Java `World.s(int, int, int)`: can a mob live at the position?
    ///
    /// The height of the terrain has to be below the position and the biome
    /// has to be one where the mobs can spawn.
    pub fn can_spawn_at(&self, x: i32, y: i32, z: i32) -> bool {
        if !self.is_chunk_loaded(ChunkPos::new(x >> 4, z >> 4)) {
            return false;
        }
        if self.height_map_at(x, z) > y {
            return false;
        }
        // The biomes of the reference where the mobs cannot spawn (the ocean
        // and the ones with the flag of the jungle).
        !matches!(
            self.biome_at(x, z),
            crate::world::biome::Biome::Swampland | crate::world::biome::Biome::Sky
        )
    }

    /// Java `World.d(AxisAlignedBB)`: is there a fire or a lava block in the
    /// box?
    pub fn has_fire_or_lava_in(&self, aabb: &crate::entity::physics::Aabb) -> bool {
        let min_x = aabb.min_x.floor() as i32;
        let max_x = (aabb.max_x + 1.0).floor() as i32;
        let min_y = aabb.min_y.floor() as i32;
        let max_y = (aabb.max_y + 1.0).floor() as i32;
        let min_z = aabb.min_z.floor() as i32;
        let max_z = (aabb.max_z + 1.0).floor() as i32;
        for x in min_x..max_x {
            for y in min_y..max_y {
                for z in min_z..max_z {
                    let id = self.get_block(x, y, z);
                    if id == blocks::FIRE || id == blocks::LAVA || id == blocks::STATIONARY_LAVA {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Java `World.e(int, int, int)`: the block is solid.
    ///
    /// The reference checks the material of the block (`material.h()`) and
    /// the block itself (`block.b()`).
    pub fn is_solid_at(&self, x: i32, y: i32, z: i32) -> bool {
        let id = self.get_block(x, y, z);
        blocks::material(id).is_solid_for_collision() && blocks::is_opaque_cube(id)
    }

    pub fn get_data(&self, x: i32, y: i32, z: i32) -> u8 {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return 0;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active && self.live_resolve_read(pos) {
            // EmptyChunk.getData -> 0.
            return 0;
        }
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_metadata((x & 15) as usize, y as usize, (z & 15) as usize)
        } else {
            0
        }
    }

    // ---------------------------------------------------------------------
    // Java block-write pipeline (Chunk.a / World.setTypeId semantics).
    //
    // The generator must reproduce Poseidon's exact write path: `Block.remove`
    // callbacks, height-map updates (`Chunk.g`, including its direct sky-light
    // writes), `Block.c` on-place callbacks and physics propagation
    // (`World.update` -> `applyPhysics` -> `Block.doPhysics`).
    // ---------------------------------------------------------------------

    /// Java `Chunk.a(i, j, k, l)` (4-arg form): raw type write with callbacks.
    pub fn java_set_raw_type_id(&mut self, x: i32, y: i32, z: i32, id: u8) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let lx = (x & 15) as usize;
        let lz = (z & 15) as usize;
        let live_active = self.live.active;
        if live_active && self.live_resolve_write(pos) {
            // EmptyChunk.a(): the write is accepted (returns true), the caller
            // still runs its `World.update` physics, and no data changes.
            return true;
        }
        let (old_id, old_height) = match self.chunks.get(&pos) {
            Some(chunk) => (
                chunk.get_block(lx, y as usize, lz),
                chunk.height_map[(lz << 4) | lx] as i32,
            ),
            None => return live_active,
        };
        if old_id == id {
            return false;
        }
        trace_a4!(x, y, z, old_id, id);

        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_block(lx, y as usize, lz, id);
            chunk.dirty = true;
        }
        self.packet_cache.borrow_mut().remove(&pos);

        // `Block.remove(old)` runs before the data reset, so a removal callback
        // (leaves/log marking) still sees the previous metadata of this block.
        if old_id != 0 {
            let old_data = self
                .chunks
                .get(&pos)
                .map(|c| c.get_metadata(lx, y as usize, lz))
                .unwrap_or(0);
            self.block_remove(old_id, old_data, x, y, z);
        }
        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_metadata(lx, y as usize, lz, 0);
        }

        if blocks::light_opacity(id) != 0 {
            if y >= old_height {
                self.chunk_height_update(pos.x, pos.z, lx, y + 1, lz);
            }
        } else if y == old_height - 1 {
            self.chunk_height_update(pos.x, pos.z, lx, y, lz);
        }

        if live_active {
            self.java_schedule_light(LightType::Sky, x, y, z, x, y, z);
            self.java_schedule_light(LightType::Block, x, y, z, x, y, z);
            self.chunk_relight_neighbors_at(pos.x, pos.z, lx, lz);
        }

        if id != 0 {
            self.block_on_place(id, x, y, z);
        }
        true
    }

    /// Java `Chunk.a(i, j, k, l, i1)` (5-arg form): raw type+data write.
    pub fn java_set_raw_type_id_and_data(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        id: u8,
        data: u8,
    ) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let lx = (x & 15) as usize;
        let lz = (z & 15) as usize;
        let live_active = self.live.active;
        if live_active && self.live_resolve_write(pos) {
            // EmptyChunk.a(): accepted silently; the physics pass still runs.
            return true;
        }
        let (old_id, old_data, old_height) = match self.chunks.get(&pos) {
            Some(chunk) => (
                chunk.get_block(lx, y as usize, lz),
                chunk.get_metadata(lx, y as usize, lz),
                chunk.height_map[(lz << 4) | lx] as i32,
            ),
            None => return live_active,
        };
        if old_id == id && old_data == data {
            return false;
        }
        trace_a5!(x, y, z, old_id, old_data, id, data);

        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_block(lx, y as usize, lz, id);
            if !self.fixes.piston_transmutation_fix {
                chunk.dirty = true;
            }
        }
        self.packet_cache.borrow_mut().remove(&pos);

        // Poseidon `world.settings.pistons.transmutation-fix.enabled`:
        // `Chunk.a(i,j,k,l,i1)` normally runs `Block.remove` *after* the new
        // metadata is stored, so callbacks observe the replacement block's
        // data. The pre-fix order let a piston mutate the old block after the
        // replacement type had already been written.
        if old_id != 0 && !self.fixes.piston_transmutation_fix {
            self.block_remove(old_id, old_data, x, y, z);
        }

        {
            let chunk = self.chunks.get_mut(&pos).unwrap();
            chunk.set_metadata(lx, y as usize, lz, data);
            chunk.dirty = true;
        }

        if old_id != 0 && self.fixes.piston_transmutation_fix {
            self.block_remove(old_id, old_data, x, y, z);
        }

        if !self.is_hell {
            if blocks::light_opacity(id) != 0 {
                if y >= old_height {
                    self.chunk_height_update(pos.x, pos.z, lx, y + 1, lz);
                }
            } else if y == old_height - 1 {
                self.chunk_height_update(pos.x, pos.z, lx, y, lz);
            }
        }

        if live_active {
            self.java_schedule_light(LightType::Sky, x, y, z, x, y, z);
            self.java_schedule_light(LightType::Block, x, y, z, x, y, z);
            self.chunk_relight_neighbors_at(pos.x, pos.z, lx, lz);
        }

        if id != 0 {
            self.block_on_place(id, x, y, z);
        }
        true
    }

    /// Java `World.setRawData(i, j, k, l)`: metadata write only. Java returns
    /// true for every in-range target — including an `EmptyChunk` — so the
    /// caller's physics pass always runs.
    pub fn java_set_raw_data(&mut self, x: i32, y: i32, z: i32, data: u8) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let live_active = self.live.active;
        if live_active && self.live_resolve_write(pos) {
            // EmptyChunk.b(): silently accepted, `setRawData` still returns true.
            return true;
        }
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            let old = chunk.get_metadata((x & 15) as usize, y as usize, (z & 15) as usize);
            trace_b!(x, y, z, old, data);
            chunk.set_metadata((x & 15) as usize, y as usize, (z & 15) as usize, data);
            chunk.dirty = true;
            self.packet_cache.borrow_mut().remove(&pos);
            true
        } else {
            live_active
        }
    }

    /// Java `World.setData(i, j, k, l)`: metadata write followed by physics.
    pub fn java_set_data(&mut self, x: i32, y: i32, z: i32, data: u8) -> bool {
        if self.java_set_raw_data(x, y, z, data) {
            let id = self.get_block(x, y, z);
            // The block notifies the clients when its data changes only when
            // the flag `Block.t[]` is set (`World.setData`).
            if blocks::notifies_on_data_change(id) {
                self.java_update(x, y, z, id as i32);
            } else {
                self.java_apply_physics(x, y, z, id as i32);
            }
            true
        } else {
            false
        }
    }

    /// Java `World.setTypeId(i, j, k, l)`.
    pub fn java_set_type_id(&mut self, x: i32, y: i32, z: i32, id: u8) -> bool {
        let old = self.get_block(x, y, z);
        if self.java_set_raw_type_id(x, y, z, id) {
            self.java_update(x, y, z, if id == 0 { old as i32 } else { id as i32 });
            true
        } else {
            false
        }
    }

    /// Java `World.setTypeIdAndData(i, j, k, l, i1)`.
    pub fn java_set_type_id_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) -> bool {
        let old = self.get_block(x, y, z);
        if self.java_set_raw_type_id_and_data(x, y, z, id, data) {
            self.java_update(x, y, z, if id == 0 { old as i32 } else { id as i32 });
            true
        } else {
            false
        }
    }

    fn java_update(&mut self, x: i32, y: i32, z: i32, l: i32) {
        self.flag_dirty(x, y, z);
        self.java_apply_physics(x, y, z, l);
    }

    pub(crate) fn java_apply_physics(&mut self, x: i32, y: i32, z: i32, l: i32) {
        self.java_neighbor_physics(x - 1, y, z, l);
        self.java_neighbor_physics(x + 1, y, z, l);
        self.java_neighbor_physics(x, y - 1, z, l);
        self.java_neighbor_physics(x, y + 1, z, l);
        self.java_neighbor_physics(x, y, z - 1, l);
        self.java_neighbor_physics(x, y, z + 1, l);
    }

    /// Java `World.k(i, j, k, l)` — dispatch `Block.doPhysics` to one neighbor.
    fn java_neighbor_physics(&mut self, x: i32, y: i32, z: i32, l: i32) {
        if self.suppress_physics {
            return;
        }
        let id = self.get_block(x, y, z);
        trace_phys!(x, y, z, l, id as i32);
        match id {
            blocks::WATER | blocks::STATIONARY_WATER | blocks::LAVA | blocks::STATIONARY_LAVA => {
                // BlockFluids.doPhysics -> fizz (lava only).
                self.fluid_fizz(x, y, z, id);
                if id == blocks::STATIONARY_WATER || id == blocks::STATIONARY_LAVA {
                    // BlockStationary.doPhysics tail: convert to flowing (raw).
                    if self.get_block(x, y, z) == id {
                        self.stationary_to_flowing(x, y, z, id);
                    }
                }
            }
            blocks::SAPLING
            | blocks::TALL_GRASS
            | blocks::DEAD_BUSH
            | blocks::YELLOW_FLOWER
            | blocks::RED_ROSE
            | blocks::BROWN_MUSHROOM
            | blocks::RED_MUSHROOM => {
                // BlockFlower.doPhysics -> destroy and drop when the plant can't survive.
                self.destroy_plant(id, x, y, z);
            }
            blocks::CROPS => {
                if self.get_block(x, y - 1, z) != blocks::FARMLAND {
                    let meta = self.get_data(x, y, z);
                    if !self.insta_tick && meta == 7 {
                        self.world_events
                            .push(crate::world::redstone::WorldEvent::DropItem {
                                x: x as f64,
                                y: y as f64,
                                z: z as f64,
                                item: crate::gameplay::items::ItemStack::new(
                                    crate::gameplay::items::WHEAT,
                                    1,
                                    0,
                                ),
                            });
                    }
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            blocks::FARMLAND => {
                if y < 127 && blocks::material(self.get_block(x, y + 1, z)).is_buildable() {
                    self.java_set_type_id(x, y, z, blocks::DIRT);
                }
            }
            blocks::PORTAL => {
                self.do_physics_portal(x, y, z);
            }
            blocks::CACTUS => {
                if !self.cactus_can_survive(x, y, z) {
                    if !self.insta_tick {
                        self.world_events
                            .push(crate::world::redstone::WorldEvent::DropItem {
                                x: x as f64,
                                y: y as f64,
                                z: z as f64,
                                item: crate::gameplay::items::ItemStack::new(
                                    blocks::CACTUS as i16,
                                    1,
                                    0,
                                ),
                            });
                    }
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            blocks::REED => {
                if !self.reed_can_survive(x, y, z) {
                    if !self.insta_tick {
                        self.world_events
                            .push(crate::world::redstone::WorldEvent::DropItem {
                                x: x as f64,
                                y: y as f64,
                                z: z as f64,
                                item: crate::gameplay::items::ItemStack::new(
                                    crate::gameplay::items::SUGAR_CANE,
                                    1,
                                    0,
                                ),
                            });
                    }
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            blocks::SNOW => {
                let below = self.get_block(x, y - 1, z);
                if below == 0 || !blocks::is_opaque_cube(below) || !blocks::is_solid(below) {
                    self.java_set_type_id(x, y, z, 0);
                }
            }
            // BlockSand/BlockGravel schedule a tick (delay `BlockSand.c()` = 3).
            blocks::SAND | blocks::GRAVEL => self.java_schedule_tick(x, y, z, id, 3),
            blocks::REDSTONE_WIRE => {
                if !self.is_solid_normal_at(x, y - 1, z) {
                    self.java_set_type_id(x, y, z, 0);
                    self.world_events
                        .push(crate::world::redstone::WorldEvent::DropItem {
                            x: x as f64,
                            y: y as f64,
                            z: z as f64,
                            item: crate::gameplay::items::ItemStack::new(
                                crate::gameplay::items::REDSTONE,
                                1,
                                0,
                            ),
                        });
                } else {
                    self.update_redstone_wire(x, y, z);
                }
            }
            blocks::REDSTONE_TORCH_OFF | blocks::REDSTONE_TORCH_ON => {
                self.java_schedule_tick(x, y, z, id, 2);
            }
            blocks::REPEATER_OFF | blocks::REPEATER_ON => {
                self.do_physics_repeater(x, y, z, id);
            }
            blocks::WOODEN_DOOR | blocks::IRON_DOOR => {
                let meta = self.get_data(x, y, z);
                if (meta & 8) != 0 {
                    if self.get_block(x, y - 1, z) != id {
                        self.java_set_type_id(x, y, z, 0);
                    } else if l > 0 && self.is_power_source_block(l as u8) {
                        self.java_neighbor_physics(x, y - 1, z, l);
                    }
                } else if self.get_block(x, y + 1, z) != id || !self.is_solid_normal_at(x, y - 1, z)
                {
                    self.java_set_type_id(x, y, z, 0);
                } else if l > 0 && self.is_power_source_block(l as u8) {
                    let powered = self.is_block_indirectly_powered(x, y, z)
                        || self.is_block_indirectly_powered(x, y + 1, z);
                    self.power_door(x, y, z, id, powered);
                }
            }
            blocks::TRAPDOOR => {
                if l > 0 && self.is_power_source_block(l as u8) {
                    let powered = self.is_block_indirectly_powered(x, y, z);
                    self.power_trapdoor(x, y, z, powered);
                }
            }
            blocks::NOTE_BLOCK => {
                if l > 0 && self.is_power_source_block(l as u8) {
                    let powered = self.is_block_indirectly_powered(x, y, z);
                    let was_powered = self.get_data(x, y, z) != 0;
                    if was_powered != powered {
                        self.java_set_raw_data(x, y, z, u8::from(powered));
                        if powered {
                            self.trigger_note_block(x, y, z);
                        }
                    }
                }
            }
            blocks::DISPENSER => {
                if l > 0 && self.is_power_source_block(l as u8) {
                    let powered = self.is_block_indirectly_powered(x, y, z)
                        || self.is_block_indirectly_powered(x, y + 1, z);
                    if powered {
                        self.java_schedule_tick(x, y, z, blocks::DISPENSER, 4);
                    }
                }
            }
            blocks::PISTON | blocks::STICKY_PISTON => {
                if !self.piston_updating {
                    self.update_piston_state(x, y, z, id);
                }
            }
            blocks::PISTON_HEAD => {
                if !self.piston_updating {
                    let meta = self.get_data(x, y, z);
                    let dir = (meta & 7) as usize;
                    if dir <= 5 {
                        let bx = x - [0, 0, 0, 0, -1, 1][dir];
                        let by = y - [-1, 1, 0, 0, 0, 0][dir];
                        let bz = z - [0, 0, -1, 1, 0, 0][dir];
                        let base_id = self.get_block(bx, by, bz);
                        if base_id != blocks::PISTON && base_id != blocks::STICKY_PISTON {
                            self.java_set_type_id(x, y, z, 0);
                        } else {
                            self.java_neighbor_physics(bx, by, bz, l);
                        }
                    }
                }
            }
            blocks::TNT => {
                if l > 0
                    && self.is_power_source_block(l as u8)
                    && self.is_block_indirectly_powered(x, y, z)
                {
                    self.ignite_tnt(x, y, z, false);
                }
            }
            blocks::RAIL | blocks::POWERED_RAIL | blocks::DETECTOR_RAIL => {
                self.do_physics_rail(x, y, z, id, l);
            }
            blocks::FIRE
                if !self.is_solid_normal_at(x, y - 1, z)
                    && !self.has_flammable_neighbor(x, y, z) =>
            {
                self.java_set_type_id(x, y, z, 0);
            }
            _ => {}
        }
    }

    /// Java `World.c(i, j, k, l, i1)`: schedule a block tick. While `insta_tick`
    /// (Java `World.a`) is set the tick runs immediately and recursively; in
    /// the server tick loop (`World.a(boolean)`) queued entries run when their
    /// time arrives. The queue branch drops entries outside loaded chunks.
    pub(crate) fn java_schedule_tick(&mut self, x: i32, y: i32, z: i32, id: u8, delay: i64) {
        // `NextTickListEntry` is constructed (and the global sequence counter
        // consumed) before any branch checks.
        let seq = self.scheduled_seq;
        self.scheduled_seq += 1;

        if self.insta_tick {
            // `World.a(i - 8, j - 8, k - 8, i + 8, j + 8, k + 8)` guard.
            if !self.chunks_area_loaded(x - 8, y - 8, z - 8, x + 8, y + 8, z + 8) {
                return;
            }
            let cur = self.get_block(x, y, z);
            if cur == id && cur > 0 {
                self.java_dispatch_tick(x, y, z, cur);
            }
            return;
        }

        if !self.chunks_area_loaded(x - 8, y - 8, z - 8, x + 8, y + 8, z + 8) {
            return;
        }
        let time = if id > 0 { delay + self.live_time } else { 0 };
        let key = (x, y, z, id);
        if !self.scheduled_set.contains(&key) {
            self.scheduled_set.insert(key);
            self.scheduled.insert(ScheduledTick {
                x,
                y,
                z,
                id,
                time,
                seq,
            });
        }
    }

    /// Java `World.a(boolean)` — process scheduled ticks. Up to 1000 entries
    /// per call; with `flag == false` processing stops at the first entry
    /// whose time is still in the future. Entries outside loaded chunks are
    /// dropped (`World.a` pops them before the chunk check).
    pub fn java_process_scheduled(&mut self, flag: bool) {
        let cap = self.scheduled.len().min(1000);
        for _ in 0..cap {
            let Some(first) = self.scheduled.iter().next().cloned() else {
                break;
            };
            if !flag && first.time > self.live_time {
                break;
            }
            self.scheduled.remove(&first);
            self.scheduled_set
                .remove(&(first.x, first.y, first.z, first.id));
            if !self.chunks_area_loaded(
                first.x - 8,
                first.y - 8,
                first.z - 8,
                first.x + 8,
                first.y + 8,
                first.z + 8,
            ) {
                continue;
            }
            let cur = self.get_block(first.x, first.y, first.z);
            if cur == first.id && cur > 0 {
                self.java_dispatch_tick(first.x, first.y, first.z, cur);
            }
        }
    }

    /// Dispatch a scheduled tick to the current block, mirroring
    /// `Block.byId[id].a(world, x, y, z, world.random)`.
    fn java_dispatch_tick(&mut self, x: i32, y: i32, z: i32, id: u8) {
        match id {
            blocks::WATER | blocks::LAVA => {
                let lava = id == blocks::LAVA;
                let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
                self.java_fluid_tick(x, y, z, &mut rng, lava);
                self.world_random = rng;
            }
            blocks::SAND | blocks::GRAVEL => self.java_sand_fall(x, y, z, id),
            blocks::REDSTONE_TORCH_OFF => self.tick_redstone_torch(x, y, z, false),
            blocks::REDSTONE_TORCH_ON => self.tick_redstone_torch(x, y, z, true),
            blocks::REPEATER_OFF => self.tick_repeater(x, y, z, false),
            blocks::REPEATER_ON => self.tick_repeater(x, y, z, true),
            blocks::STONE_BUTTON => self.tick_button(x, y, z),
            blocks::STONE_PLATE | blocks::WOOD_PLATE => {
                if self.get_data(x, y, z) == 1 {
                    if self.active_pressure_plates.contains(&(x, y, z)) {
                        self.java_schedule_tick(x, y, z, id, 20);
                    } else {
                        self.set_pressure_plate_state(x, y, z, id, false);
                    }
                }
            }
            blocks::DETECTOR_RAIL => {
                let meta = self.get_data(x, y, z);
                if (meta & 8) != 0 {
                    if self.active_detector_rails.contains(&(x, y, z)) {
                        self.java_schedule_tick(x, y, z, id, 20);
                    } else {
                        self.java_set_data(x, y, z, meta & 7);
                        self.java_apply_physics(x, y, z, id as i32);
                        self.java_apply_physics(x, y - 1, z, id as i32);
                    }
                }
            }
            blocks::DISPENSER => {
                if self.is_block_indirectly_powered(x, y, z)
                    || self.is_block_indirectly_powered(x, y + 1, z)
                {
                    self.tick_dispenser(x, y, z);
                }
            }
            blocks::FIRE => self.tick_fire(x, y, z),
            _ => {}
        }
    }

    /// `BlockSand.c_`: can this block fall through the block below?
    fn sand_can_fall_through(&self, x: i32, y: i32, z: i32) -> bool {
        let id = self.get_block(x, y, z);
        if id == blocks::AIR || id == blocks::FIRE {
            return true;
        }
        blocks::is_water(id) || blocks::is_lava(id)
    }

    /// `BlockSand.g`: while `instaFall` (`self.insta_tick`) is true or the ±32
    /// area is not fully loaded, sand/gravel teleports down immediately; during
    /// server ticks (`!self.insta_tick`) with the ±32 area loaded, Poseidon
    /// clears the source block (`sand-gravel-duping-fix`) and spawns an
    /// `EntityFallingSand` ticked in `cleanUp()`.
    fn java_sand_fall(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if !self.sand_can_fall_through(x, y - 1, z) || y < 0 {
            return;
        }
        let b0 = 32;
        if self.live.active
            && !self.insta_tick
            && self.chunks_area_loaded(x - b0, y - b0, z - b0, x + b0, y + b0, z + b0)
        {
            if self.fixes.piston_sand_gravel_duping_fix {
                self.java_set_type_id(x, y, z, 0);
            }
            if self.chunks.contains_key(&ChunkPos::new(x >> 4, z >> 4)) {
                let _ = self.live_resolve_read(ChunkPos::new(x >> 4, z >> 4));
                self.falling_sands.push(FallingSand {
                    x,
                    z,
                    loc_y: y as f64 + 0.5,
                    mot_y: 0.0,
                    id,
                    ticks: 0,
                    dead: false,
                });
            }
            return;
        }
        self.java_set_type_id(x, y, z, 0);
        let mut j = y;
        while self.sand_can_fall_through(x, j - 1, z) && j > 0 {
            j -= 1;
        }
        if j > 0 {
            self.java_set_type_id(x, j, z, id);
        }
    }

    /// Java `World.cleanUp()` entity pass for `EntityFallingSand`.
    pub fn java_tick_falling_sands(&mut self) {
        let half_h = (0.98_f32 / 2.0_f32) as f64;
        let height_box = 0.98_f32 as f64;
        let mut i = 0;
        while i < self.falling_sands.len() {
            let mut ent = self.falling_sands[i].clone();
            if !ent.dead
                && self.chunks_area_loaded(ent.x - 32, 0, ent.z - 32, ent.x + 32, 128, ent.z + 32)
            {
                if ent.id == 0 {
                    ent.dead = true;
                } else {
                    ent.ticks += 1;
                    ent.mot_y -= 0.03999999910593033_f64;
                    let d6 = ent.mot_y;
                    let mut d1 = d6;
                    let mut min_y = ent.loc_y - half_h;
                    let max_y = min_y + height_box;
                    let y_lo = (min_y + d1).floor() as i32;
                    let y_hi = (max_y + 1.0).floor() as i32;
                    for by in y_lo..y_hi {
                        if (0..128).contains(&by) {
                            let bid = self.get_block(ent.x, by, ent.z);
                            if bid != 0 && !self.sand_can_fall_through(ent.x, by, ent.z) {
                                let block_top = (by + 1) as f64;
                                if min_y >= block_top {
                                    let diff = block_top - min_y;
                                    if diff > d1 {
                                        d1 = diff;
                                    }
                                }
                            }
                        }
                    }
                    min_y += d1;
                    ent.loc_y = min_y + half_h;
                    let on_ground = d6 != d1 && d6 < 0.0;
                    if d6 != d1 {
                        ent.mot_y = 0.0;
                    }
                    ent.mot_y *= 0.9800000190734863_f64;
                    let j = ent.loc_y.floor() as i32;
                    if self.get_block(ent.x, j, ent.z) == ent.id {
                        self.java_set_type_id(ent.x, j, ent.z, 0);
                    }
                    if on_ground {
                        ent.mot_y *= -0.5;
                        ent.dead = true;
                        let cur = self.get_block(ent.x, j, ent.z);
                        let can_build = cur == 0
                            || blocks::is_water(cur)
                            || blocks::is_lava(cur)
                            || cur == blocks::FIRE
                            || cur == blocks::SNOW;
                        if can_build && !self.sand_can_fall_through(ent.x, j - 1, ent.z) {
                            self.java_set_type_id(ent.x, j, ent.z, ent.id);
                        } else {
                            self.world_events
                                .push(crate::world::redstone::WorldEvent::DropItem {
                                    x: ent.x as f64,
                                    y: j as f64,
                                    z: ent.z as f64,
                                    item: crate::gameplay::items::ItemStack::new(
                                        ent.id as i16,
                                        1,
                                        0,
                                    ),
                                });
                        }
                    } else if ent.ticks > 100 {
                        ent.dead = true;
                        self.world_events
                            .push(crate::world::redstone::WorldEvent::DropItem {
                                x: ent.x as f64,
                                y: ent.loc_y,
                                z: ent.z as f64,
                                item: crate::gameplay::items::ItemStack::new(ent.id as i16, 1, 0),
                            });
                    }
                }
            }
            if ent.dead {
                self.falling_sands.remove(i);
            } else {
                self.falling_sands[i] = ent;
                i += 1;
            }
        }
    }

    /// BlockFluids fizz: lava touching water turns into obsidian/cobblestone.
    fn fluid_fizz(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if self.get_block(x, y, z) != id {
            return;
        }
        if !blocks::is_lava(id) {
            return;
        }
        let touches_water = self.is_water_at(x, y, z - 1)
            || self.is_water_at(x, y, z + 1)
            || self.is_water_at(x - 1, y, z)
            || self.is_water_at(x + 1, y, z)
            || self.is_water_at(x, y + 1, z);
        if !touches_water {
            return;
        }
        let data = self.get_data(x, y, z);
        if data == 0 {
            self.java_set_type_id(x, y, z, blocks::OBSIDIAN);
        } else if data <= 4 {
            self.java_set_type_id(x, y, z, blocks::COBBLESTONE);
        }
        // BlockFluids.h plays the fizz sound, consuming `world.random` twice.
        let _ = self.world_random.next_f32();
        let _ = self.world_random.next_f32();
    }

    /// BlockStationary: raw conversion to the flowing counterpart.
    fn stationary_to_flowing(&mut self, x: i32, y: i32, z: i32, id: u8) {
        let data = self.get_data(x, y, z);
        trace_conv!(x, y, z, data, id - 1);
        self.suppress_physics = true;
        // The raw write runs `Block.byId[id - 1].c` (fizz + schedule inside
        // BlockFlowing.c), then `BlockStationary.i` schedules again explicitly.
        self.java_set_raw_type_id_and_data(x, y, z, id - 1, data);
        let delay = if id == blocks::STATIONARY_WATER {
            5
        } else {
            30
        };
        self.java_schedule_tick(x, y, z, id - 1, delay);
        self.suppress_physics = false;
    }

    fn is_water_at(&self, x: i32, y: i32, z: i32) -> bool {
        blocks::is_water(self.get_block(x, y, z))
    }

    fn is_same_fluid_at(&self, x: i32, y: i32, z: i32, lava: bool) -> bool {
        let id = self.get_block(x, y, z);
        if lava {
            blocks::is_lava(id)
        } else {
            blocks::is_water(id)
        }
    }

    /// Java `BlockFlower.g`: the plant that can't survive disappears and
    /// drops its item.
    ///
    /// The physics of the neighbours (`BlockFlower.doPhysics`) and the random
    /// tick of the block (`BlockFlower.a`) call the same method of the
    /// reference.
    pub(crate) fn destroy_plant(&mut self, id: u8, x: i32, y: i32, z: i32) {
        if self.plant_can_survive(id, x, y, z) {
            return;
        }
        let meta = self.get_data(x, y, z);
        if !self.insta_tick && id != blocks::TALL_GRASS && id != blocks::DEAD_BUSH {
            let dmg = if id == blocks::SAPLING {
                (meta & 3) as i16
            } else {
                0
            };
            self.world_events
                .push(crate::world::redstone::WorldEvent::DropItem {
                    x: x as f64,
                    y: y as f64,
                    z: z as f64,
                    item: crate::gameplay::items::ItemStack::new(id as i16, 1, dmg),
                });
        }
        self.java_set_type_id(x, y, z, 0);
    }

    /// Java `BlockFlower.f` / BlockMushroom.f / BlockDeadBush.f.
    pub(crate) fn plant_can_survive(&self, id: u8, x: i32, y: i32, z: i32) -> bool {
        let below = self.get_block(x, y - 1, z);
        match id {
            blocks::BROWN_MUSHROOM | blocks::RED_MUSHROOM => {
                (0..CHUNK_HEIGHT as i32).contains(&y)
                    && self.light_at(x, y, z) < 13
                    && blocks::is_opaque_cube(below)
            }
            blocks::DEAD_BUSH => {
                // The block of the plant is at or above the terrain
                // (`World.isChunkLoaded`), not under the sky.
                (self.light_at(x, y, z) >= 8 || self.java_is_chunk_loaded_at(x, y, z))
                    && below == blocks::SAND
            }
            _ => {
                (self.light_at(x, y, z) >= 8 || self.java_is_chunk_loaded_at(x, y, z))
                    && matches!(below, blocks::GRASS | blocks::DIRT | blocks::FARMLAND)
            }
        }
    }

    /// Java `BlockCactus.f`.
    fn cactus_can_survive(&self, x: i32, y: i32, z: i32) -> bool {
        if blocks::material_is_buildable(self.get_block(x - 1, y, z))
            || blocks::material_is_buildable(self.get_block(x + 1, y, z))
            || blocks::material_is_buildable(self.get_block(x, y, z - 1))
            || blocks::material_is_buildable(self.get_block(x, y, z + 1))
        {
            return false;
        }
        matches!(self.get_block(x, y - 1, z), blocks::CACTUS | blocks::SAND)
    }

    /// Java `BlockReed.f` -> `canPlace`.
    fn reed_can_survive(&self, x: i32, y: i32, z: i32) -> bool {
        let below = self.get_block(x, y - 1, z);
        if below == blocks::REED {
            return true;
        }
        if below != blocks::GRASS && below != blocks::DIRT {
            return false;
        }
        self.is_water_at(x - 1, y - 1, z)
            || self.is_water_at(x + 1, y - 1, z)
            || self.is_water_at(x, y - 1, z - 1)
            || self.is_water_at(x, y - 1, z + 1)
    }

    /// `world.k(i, j, k)` — max(sky, block) light with y clamped to [0, 127].
    pub(crate) fn light_at(&self, x: i32, y: i32, z: i32) -> i32 {
        if y < 0 {
            return 0;
        }
        let y = y.min(CHUNK_HEIGHT as i32 - 1);
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active && self.live_resolve_read(pos) {
            return 0;
        }
        match self.chunks.get(&pos) {
            Some(chunk) => {
                chunk.get_block_light_value((x & 15) as usize, y as usize, (z & 15) as usize, 0)
            }
            // EmptyChunk light reads are 0; offline the code never reaches an
            // absent chunk, so the historical 15 stays for the offline path.
            None => {
                if self.live.active {
                    0
                } else {
                    15
                }
            }
        }
    }

    /// `world.isChunkLoaded(i, j, k)` as used by `BlockFlower.f` — it is really
    /// `Chunk.c(i, j, k)`: the block is at or above the height map.
    fn can_see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active && self.live_resolve_read(pos) {
            return false;
        }
        match self.chunks.get(&pos) {
            Some(chunk) => y >= chunk.get_height((x & 15) as usize, (z & 15) as usize) as i32,
            None => !self.live.active,
        }
    }

    /// Java `Chunk.g(i, j, k)`: recompute one height-map column and write the
    /// new sky-light values directly into the light array.
    fn chunk_height_update(&mut self, cx: i32, cz: i32, lx: usize, j: i32, lz: usize) {
        let pos = ChunkPos::new(cx, cz);
        let Some(chunk) = self.chunks.get_mut(&pos) else {
            return;
        };
        let l = chunk.height_map[(lz << 4) | lx] as i32;
        let mut i1 = l;
        if j > l {
            i1 = j;
        }
        while i1 > 0 && blocks::light_opacity(chunk.get_block(lx, (i1 - 1) as usize, lz)) == 0 {
            i1 -= 1;
        }
        if i1 != l {
            trace_g!(cx * 16 + lx as i32, cz * 16 + lz as i32, l, i1);
            let k1 = cx * 16 + lx as i32;
            let l1 = cz * 16 + lz as i32;
            chunk.height_map[(lz << 4) | lx] = i1 as u8;
            if i1 < l {
                for i2 in i1..l {
                    chunk.set_sky_light(lx, i2 as usize, lz, 15);
                }
            } else {
                if self.live.active {
                    self.java_schedule_light(LightType::Sky, k1, l, l1, k1, i1, l1);
                }
                let chunk = self.chunks.get_mut(&pos).unwrap();
                for i2 in l..i1 {
                    chunk.set_sky_light(lx, i2 as usize, lz, 0);
                }
            }
            let chunk = self.chunks.get_mut(&pos).unwrap();
            let mut i2 = 15i32;
            let j2 = i1;
            let mut y1 = i1;
            while y1 > 0 && i2 > 0 {
                y1 -= 1;
                let mut k2 = blocks::light_opacity(chunk.get_block(lx, y1 as usize, lz)) as i32;
                if k2 == 0 {
                    k2 = 1;
                }
                i2 -= k2;
                if i2 < 0 {
                    i2 = 0;
                }
                chunk.set_sky_light(lx, y1 as usize, lz, i2 as u8);
            }
            while y1 > 0 && blocks::light_opacity(chunk.get_block(lx, (y1 - 1) as usize, lz)) == 0 {
                y1 -= 1;
            }
            chunk.dirty = true;
            self.packet_cache.borrow_mut().remove(&pos);
            if self.live.active && y1 != j2 {
                self.java_schedule_light(LightType::Sky, k1 - 1, y1, l1 - 1, k1 + 1, j2, l1 + 1);
            }
        }
    }

    /// Java `Chunk.c(int i, int j)`.
    fn chunk_relight_neighbors_at(&mut self, cx: i32, cz: i32, lx: usize, lz: usize) {
        let Some(chunk) = self.chunks.get(&ChunkPos::new(cx, cz)) else {
            return;
        };
        let k = chunk.height_map[(lz << 4) | lx] as i32;
        let l = cx * 16 + lx as i32;
        let i1 = cz * 16 + lz as i32;
        self.chunk_relight_neighbor(l - 1, i1, k);
        self.chunk_relight_neighbor(l + 1, i1, k);
        self.chunk_relight_neighbor(l, i1 - 1, k);
        self.chunk_relight_neighbor(l, i1 + 1, k);
    }

    /// Java `Chunk.f(int i, int j, int k)`.
    fn chunk_relight_neighbor(&mut self, x: i32, z: i32, k: i32) {
        let l = self.java_highest_block_y_at(x, z);
        if l > k {
            self.java_schedule_light(LightType::Sky, x, k, z, x, l, z);
        } else if l < k {
            self.java_schedule_light(LightType::Sky, x, l, z, x, k, z);
        }
    }

    /// Java `World.getHighestBlockYAt(int i, int j)` (lines 562-574).
    fn java_highest_block_y_at(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return 0;
        }
        if self.live.active && self.live_resolve_read(pos) {
            return 0;
        }
        self.chunks
            .get(&pos)
            .map(|c| c.get_height((x & 15) as usize, (z & 15) as usize) as i32)
            .unwrap_or(0)
    }

    /// Java `World.m(int i, int j, int k)` (lines 542-560).
    pub(crate) fn java_can_see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        if y < 0 {
            return false;
        }
        if y >= CHUNK_HEIGHT as i32 {
            return true;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return false;
        }
        if self.live.active && self.live_resolve_read(pos) {
            return false;
        }
        self.chunks
            .get(&pos)
            .map(|c| c.can_block_see_sky((x & 15) as usize, y as usize, (z & 15) as usize))
            .unwrap_or(false)
    }

    /// Java `World.isChunkLoaded(int, int, int)` (line 472).
    ///
    /// The reference asks the chunk if the block is at or above the terrain
    /// (`Chunk.c(x, y, z)` compares the height with the height map of the
    /// chunk), the mob under a roof is not in the ground.
    pub(crate) fn java_is_chunk_loaded_at(&self, x: i32, y: i32, z: i32) -> bool {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        match self.chunks.get(&pos) {
            Some(chunk) => y >= (chunk.height_map[(((z & 15) << 4) | (x & 15)) as usize] as i32),
            None => false,
        }
    }

    /// Java `World.a(EnumSkyBlock, int i, int j, int k)` (lines 598-621).
    pub(crate) fn java_get_saved_light(&self, kind: LightType, x: i32, mut y: i32, z: i32) -> i32 {
        if y < 0 {
            y = 0;
        }
        if y >= CHUNK_HEIGHT as i32 {
            y = CHUNK_HEIGHT as i32 - 1;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return 0;
        }
        if self.live.active && self.live_resolve_read(pos) {
            return 0;
        }
        let Some(chunk) = self.chunks.get(&pos) else {
            return 0;
        };
        let lx = (x & 15) as usize;
        let lz = (z & 15) as usize;
        match kind {
            LightType::Sky => chunk.get_sky_light(lx, y as usize, lz) as i32,
            LightType::Block => chunk.get_block_light(lx, y as usize, lz) as i32,
        }
    }

    /// Java `World.b(EnumSkyBlock, int i, int j, int k, int l)` (lines 623-639).
    fn java_set_saved_light(&mut self, kind: LightType, x: i32, y: i32, z: i32, l: i32) {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return;
        }
        if self.live.active && self.live_resolve_read(pos) {
            return;
        }
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            let lx = (x & 15) as usize;
            let lz = (z & 15) as usize;
            chunk.dirty = true;
            match kind {
                LightType::Sky => chunk.set_sky_light(lx, y as usize, lz, l as u8),
                LightType::Block => chunk.set_block_light(lx, y as usize, lz, l as u8),
            }
            self.light_dirty_chunks.insert(pos);
            self.packet_cache.borrow_mut().remove(&pos);
        }
    }

    /// Java `World.a(EnumSkyBlock, int i, int j, int k, int l, int i1, int j1, boolean flag)`
    /// (lines 1652-1700).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn java_schedule_light(
        &mut self,
        kind: LightType,
        i: i32,
        j: i32,
        k: i32,
        l: i32,
        i1: i32,
        j1: i32,
    ) {
        // Java `World.a(...).a(...)`: the Nether has no sky light at all.
        if self.is_hell && kind == LightType::Sky {
            return;
        }
        self.light_schedule_depth += 1;
        if self.light_schedule_depth < 50 {
            let k1 = (l + i) / 2;
            let l1 = (j1 + k) / 2;
            let mid_pos = ChunkPos::new(k1 >> 4, l1 >> 4);
            if self.chunks.contains_key(&mid_pos) && !self.live_resolve_read(mid_pos) {
                let i2 = self.light_updates.len();
                let j2 = 5.min(i2);
                let mut merged = false;
                for k2 in 0..j2 {
                    let idx = i2 - k2 - 1;
                    if self.light_updates[idx].kind == kind
                        && self.light_updates[idx].try_merge(i, j, k, l, i1, j1)
                    {
                        merged = true;
                        break;
                    }
                }
                if !merged {
                    self.light_updates.push(MetadataChunkBlock {
                        kind,
                        b: i,
                        c: j,
                        d: k,
                        e: l,
                        f: i1,
                        g: j1,
                    });
                    if self.light_updates.len() > 1_000_000 {
                        self.light_updates.clear();
                    }
                }
            }
        }
        self.light_schedule_depth -= 1;
    }

    /// Java `World.a(EnumSkyBlock, int i, int j, int k, int l)` (lines 576-596).
    fn java_check_neighbor_light(&mut self, kind: LightType, i: i32, j: i32, k: i32, mut l: i32) {
        if self.is_hell && kind == LightType::Sky {
            return;
        }
        if !(0..CHUNK_HEIGHT as i32).contains(&j) {
            return;
        }
        if !self.chunks.contains_key(&ChunkPos::new(i >> 4, k >> 4)) {
            return;
        }
        match kind {
            LightType::Sky => {
                if self.java_can_see_sky(i, j, k) {
                    l = 15;
                }
            }
            LightType::Block => {
                let i1 = self.get_block(i, j, k);
                let em = blocks::light_emission(i1) as i32;
                if em > l {
                    l = em;
                }
            }
        }
        if self.java_get_saved_light(kind, i, j, k) != l {
            self.java_schedule_light(kind, i, j, k, i, j, k);
        }
    }

    /// Java `MetadataChunkBlock.a(World world)` (lines 23-160).
    fn java_run_light_block(&mut self, mut blk: MetadataChunkBlock) {
        let i = blk.e - blk.b + 1;
        let j = blk.f - blk.c + 1;
        let k = blk.g - blk.d + 1;
        let l = i * j * k;
        if l > 32768 {
            return;
        }
        for k1 in blk.b..=blk.e {
            for l1 in blk.d..=blk.g {
                let mut flag2 = self.chunks_area_loaded(k1 - 1, -1, l1 - 1, k1 + 1, 1, l1 + 1);
                if flag2 && self.live_resolve_read(ChunkPos::new(k1 >> 4, l1 >> 4)) {
                    flag2 = false;
                }
                if flag2 {
                    if blk.c < 0 {
                        blk.c = 0;
                    }
                    if blk.f >= 128 {
                        blk.f = 127;
                    }
                    for k2 in blk.c..=blk.f {
                        let l2 = self.java_get_saved_light(blk.kind, k1, k2, l1);
                        let i3 = self.get_block(k1, k2, l1);
                        let mut j3 = blocks::light_opacity(i3) as i32;
                        if j3 == 0 {
                            j3 = 1;
                        }
                        let k3 = match blk.kind {
                            LightType::Sky => {
                                if self.java_can_see_sky(k1, k2, l1) {
                                    15
                                } else {
                                    0
                                }
                            }
                            LightType::Block => blocks::light_emission(i3) as i32,
                        };
                        let i4 = if j3 >= 15 && k3 == 0 {
                            0
                        } else {
                            let l3 = self.java_get_saved_light(blk.kind, k1 - 1, k2, l1);
                            let j4 = self.java_get_saved_light(blk.kind, k1 + 1, k2, l1);
                            let k4 = self.java_get_saved_light(blk.kind, k1, k2 - 1, l1);
                            let l4 = self.java_get_saved_light(blk.kind, k1, k2 + 1, l1);
                            let i5 = self.java_get_saved_light(blk.kind, k1, k2, l1 - 1);
                            let j5 = self.java_get_saved_light(blk.kind, k1, k2, l1 + 1);
                            let m = l3.max(j4).max(k4).max(l4).max(i5).max(j5);
                            (m - j3).max(0).max(k3)
                        };
                        if l2 != i4 {
                            self.java_set_saved_light(blk.kind, k1, k2, l1, i4);
                            let l3 = (i4 - 1).max(0);
                            self.java_check_neighbor_light(blk.kind, k1 - 1, k2, l1, l3);
                            self.java_check_neighbor_light(blk.kind, k1, k2 - 1, l1, l3);
                            self.java_check_neighbor_light(blk.kind, k1, k2, l1 - 1, l3);
                            if k1 + 1 >= blk.e {
                                self.java_check_neighbor_light(blk.kind, k1 + 1, k2, l1, l3);
                            }
                            if k2 + 1 >= blk.f {
                                self.java_check_neighbor_light(blk.kind, k1, k2 + 1, l1, l3);
                            }
                            if l1 + 1 >= blk.g {
                                self.java_check_neighbor_light(blk.kind, k1, k2, l1 + 1, l3);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Java `World.doLighting()` (lines 1618-1645).
    pub fn java_do_lighting(&mut self) -> bool {
        if self.do_lighting_depth >= 50 {
            return false;
        }
        self.do_lighting_depth += 1;
        let mut i = 500i32;
        let mut flag = false;
        while !self.light_updates.is_empty() {
            i -= 1;
            if i <= 0 {
                flag = true;
                break;
            }
            let blk = self.light_updates.pop().unwrap();
            self.java_run_light_block(blk);
        }
        self.do_lighting_depth -= 1;
        if self.do_lighting_depth == 0
            && self.light_updates.is_empty()
            && self.light_updates.capacity() > 2048
        {
            self.light_updates.shrink_to(512);
        }
        flag
    }

    pub fn compact_dirty_light_chunks(&mut self) {
        if self.light_updates.is_empty() && !self.light_dirty_chunks.is_empty() {
            for pos in self.light_dirty_chunks.drain() {
                if let Some(chunk) = self.chunks.get_mut(&pos) {
                    chunk.compact();
                }
            }
            if self.light_dirty_chunks.capacity() > 256 {
                self.light_dirty_chunks.shrink_to(64);
            }
        }
    }

    pub fn compact_all_chunks(&mut self) {
        for chunk in self.chunks.values_mut() {
            chunk.compact();
        }
        self.chunks.shrink_to_fit();
        self.light_updates.shrink_to_fit();
        self.light_dirty_chunks.shrink_to_fit();
        self.scheduled_set.shrink_to_fit();
        self.packet_cache.borrow_mut().clear();
        self.packet_cache.borrow_mut().shrink_to_fit();
        crate::util::trim_heap_memory();
    }

    /// `Block.remove` dispatch for the replaced block.
    fn block_remove(&mut self, id: u8, old_data: u8, x: i32, y: i32, z: i32) {
        match id {
            blocks::LEAVES => self.mark_leaves_around(x, y, z, 1, false),
            blocks::LOG => self.mark_leaves_around(x, y, z, 4, true),
            blocks::REDSTONE_WIRE => {
                if !self.suppress_physics {
                    self.on_wire_place_or_remove(x, y, z);
                }
            }
            blocks::REDSTONE_TORCH_ON => {
                self.notify_torch_neighbors(x, y, z, id);
            }
            blocks::LEVER | blocks::STONE_BUTTON => {
                if (old_data & 8) > 0 {
                    self.notify_attachment_neighbors(x, y, z, id, old_data & 7);
                }
            }
            blocks::STONE_PLATE | blocks::WOOD_PLATE => {
                if old_data > 0 {
                    self.java_apply_physics(x, y, z, id as i32);
                    self.java_apply_physics(x, y - 1, z, id as i32);
                }
            }
            blocks::PISTON | blocks::STICKY_PISTON if !self.piston_updating => {
                if (old_data & 8) != 0 {
                    let dir = (old_data & 7) as usize;
                    if dir <= 5 {
                        let hx = x + [0, 0, 0, 0, -1, 1][dir];
                        let hy = y + [-1, 1, 0, 0, 0, 0][dir];
                        let hz = z + [0, 0, -1, 1, 0, 0][dir];
                        if self.get_block(hx, hy, hz) == blocks::PISTON_HEAD
                            && (self.get_data(hx, hy, hz) & 7) as usize == dir
                        {
                            self.java_set_type_id(hx, hy, hz, 0);
                        }
                    }
                }
            }
            blocks::PISTON_HEAD if !self.piston_updating => {
                let dir = (old_data & 7) as usize;
                if dir <= 5 {
                    let bx = x - [0, 0, 0, 0, -1, 1][dir];
                    let by = y - [-1, 1, 0, 0, 0, 0][dir];
                    let bz = z - [0, 0, -1, 1, 0, 0][dir];
                    let base = self.get_block(bx, by, bz);
                    if (base == blocks::PISTON || base == blocks::STICKY_PISTON)
                        && (self.get_data(bx, by, bz) & 8) != 0
                    {
                        self.world_events
                            .push(crate::world::redstone::WorldEvent::DropItem {
                                x: bx as f64,
                                y: by as f64,
                                z: bz as f64,
                                item: crate::gameplay::items::ItemStack::new(base as i16, 1, 0),
                            });
                        self.java_set_type_id(bx, by, bz, 0);
                    }
                }
            }
            blocks::SPONGE => {
                let r = self.mechanics.sponge_radius.clamp(1, 8);
                let opt = self.mechanics.optimize_sponges;
                for dx in -r..=r {
                    for dy in -r..=r {
                        let ny = y + dy;
                        if !(0..128).contains(&ny) {
                            continue;
                        }
                        for dz in -r..=r {
                            let t = self.get_block(x + dx, ny, z + dz);
                            if !opt || t == blocks::WATER || t == blocks::STATIONARY_WATER {
                                self.java_apply_physics(x + dx, ny, z + dz, t as i32);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// BlockLeaves.remove / BlockLog.remove: mark nearby leaves with data bit 8.
    fn mark_leaves_around(&mut self, x: i32, y: i32, z: i32, radius: i32, skip_marked: bool) {
        let check = radius + 1;
        if !self.chunks_area_loaded(
            x - check,
            y - check,
            z - check,
            x + check,
            y + check,
            z + check,
        ) {
            return;
        }
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let (ix, iy, iz) = (x + dx, y + dy, z + dz);
                    if self.get_block(ix, iy, iz) != blocks::LEAVES {
                        continue;
                    }
                    let data = self.get_data(ix, iy, iz);
                    if skip_marked && (data & 8) != 0 {
                        continue;
                    }
                    self.java_set_raw_data(ix, iy, iz, data | 8);
                }
            }
        }
    }

    /// Java `World.a(i, j, k, l, i1, j1)`: are all chunks in the box loaded?
    pub fn chunks_area_loaded(&self, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> bool {
        if y1 < 0 || y0 >= CHUNK_HEIGHT as i32 {
            return false;
        }
        for cx in (x0 >> 4)..=(x1 >> 4) {
            for cz in (z0 >> 4)..=(z1 >> 4) {
                if !self.chunks.contains_key(&ChunkPos::new(cx, cz)) {
                    return false;
                }
            }
        }
        true
    }

    /// `Block.c` on-place dispatch for the freshly written block.
    fn block_on_place(&mut self, id: u8, x: i32, y: i32, z: i32) {
        match id {
            blocks::WATER | blocks::LAVA => {
                // BlockFlowing.c: super.c (fizz) then schedule a tick when the
                // block is still flowing.
                self.fluid_fizz(x, y, z, id);
                if self.get_block(x, y, z) == id {
                    let delay = if id == blocks::WATER { 5 } else { 30 };
                    self.java_schedule_tick(x, y, z, id, delay);
                }
            }
            blocks::STATIONARY_WATER | blocks::STATIONARY_LAVA => {
                // BlockFluids.c -> fizz.
                self.fluid_fizz(x, y, z, id);
            }
            blocks::SAND | blocks::GRAVEL => {
                // BlockSand.c: schedule a fall check (delay 3).
                self.java_schedule_tick(x, y, z, id, 3);
            }
            blocks::REDSTONE_WIRE => {
                if !self.suppress_physics {
                    self.on_wire_place_or_remove(x, y, z);
                }
            }
            blocks::REDSTONE_TORCH_ON => {
                self.notify_torch_neighbors(x, y, z, id);
                self.java_schedule_tick(x, y, z, id, 2);
            }
            blocks::REDSTONE_TORCH_OFF => {
                self.java_schedule_tick(x, y, z, id, 2);
            }
            blocks::REPEATER_OFF | blocks::REPEATER_ON => {
                self.notify_torch_neighbors(x, y, z, id);
            }
            blocks::TNT if self.is_block_indirectly_powered(x, y, z) => {
                self.ignite_tnt(x, y, z, false);
            }
            blocks::RAIL | blocks::POWERED_RAIL | blocks::DETECTOR_RAIL => {
                if !self.suppress_physics {
                    self.update_rail_track(x, y, z, true);
                    if id == blocks::POWERED_RAIL {
                        self.do_physics_rail(x, y, z, id, id as i32);
                    }
                }
            }
            blocks::FIRE => {
                if y > 0
                    && self.get_block(x, y - 1, z) == blocks::OBSIDIAN
                    && self.try_create_portal(x, y, z)
                {
                    return;
                }
                if !self.is_solid_normal_at(x, y - 1, z) && !self.has_flammable_neighbor(x, y, z) {
                    self.java_set_type_id(x, y, z, 0);
                } else {
                    let delay = if self.mechanics.pre_1_6_fire { 10 } else { 40 };
                    self.java_schedule_tick(x, y, z, blocks::FIRE, delay);
                }
            }
            blocks::SPONGE if self.mechanics.sponge_absorption_enabled => {
                let r = self.mechanics.sponge_radius.clamp(1, 8);
                for dx in -r..=r {
                    for dy in -r..=r {
                        let ny = y + dy;
                        if !(0..128).contains(&ny) {
                            continue;
                        }
                        for dz in -r..=r {
                            let t = self.get_block(x + dx, ny, z + dz);
                            if t == blocks::WATER || t == blocks::STATIONARY_WATER {
                                self.java_set_type_id(x + dx, ny, z + dz, 0);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ----- BlockFlowing helpers (fluids spread only when ticked directly) -----

    /// BlockFluids.g: the fluid data at a position, or -1 when another material.
    fn fluid_data_at(&self, x: i32, y: i32, z: i32, lava: bool) -> i32 {
        if self.is_same_fluid_at(x, y, z, lava) {
            self.get_data(x, y, z) as i32
        } else {
            -1
        }
    }

    /// BlockFlowing.k: does a solid-ish block stop the flow here?
    fn flowing_blocked(&self, x: i32, y: i32, z: i32) -> bool {
        let l = self.get_block(x, y, z);
        if matches!(
            l,
            blocks::WOODEN_DOOR | blocks::IRON_DOOR | blocks::SIGN | blocks::LADDER | blocks::REED
        ) {
            return true;
        }
        if l == 0 {
            return false;
        }
        blocks::material_is_solid(l)
    }

    /// BlockFlowing.l: can the fluid spread into this position?
    fn flowing_can_flow_into(&self, x: i32, y: i32, z: i32, lava: bool) -> bool {
        if self.is_same_fluid_at(x, y, z, lava) {
            return false;
        }
        if blocks::is_lava(self.get_block(x, y, z)) {
            return false;
        }
        !self.flowing_blocked(x, y, z)
    }

    /// BlockFlowing.b: distance to a fully spread level in one direction.
    fn flowing_distance(&self, x: i32, y: i32, z: i32, l: i32, avoid: i32, lava: bool) -> i32 {
        let mut best = 1000;
        for k1 in 0..4 {
            if (k1 == 0 && avoid == 1)
                || (k1 == 1 && avoid == 0)
                || (k1 == 2 && avoid == 3)
                || (k1 == 3 && avoid == 2)
            {
                continue;
            }
            let mut l1 = x;
            let mut i2 = z;
            if k1 == 0 {
                l1 = x - 1;
            }
            if k1 == 1 {
                l1 += 1;
            }
            if k1 == 2 {
                i2 = z - 1;
            }
            if k1 == 3 {
                i2 += 1;
            }
            if self.flowing_blocked(l1, y, i2)
                || (self.is_same_fluid_at(l1, y, i2, lava) && self.get_data(l1, y, i2) == 0)
            {
                continue;
            }
            if !self.flowing_blocked(l1, y - 1, i2) {
                return l;
            }
            if l < 4 {
                let j2 = self.flowing_distance(l1, y, i2, l + 1, k1, lava);
                if j2 < best {
                    best = j2;
                }
            }
        }
        best
    }

    /// BlockFlowing.j: choose the directions the fluid spreads to. Stores the
    /// result in the shared `fluid_sides` field (Java `this.b`) exactly like
    /// the reference singleton — nested ticks overwrite it mid-loop.
    #[allow(clippy::needless_range_loop)]
    fn flowing_sides(&mut self, x: i32, y: i32, z: i32, lava: bool) {
        let li = if lava { 1 } else { 0 };
        let mut c = [1000i32; 4];
        for l in 0..4usize {
            let mut i1 = x;
            let mut j1 = z;
            if l == 0 {
                i1 = x - 1;
            }
            if l == 1 {
                i1 += 1;
            }
            if l == 2 {
                j1 = z - 1;
            }
            if l == 3 {
                j1 += 1;
            }
            if !self.flowing_blocked(i1, y, j1)
                && (!self.is_same_fluid_at(i1, y, j1, lava) || self.get_data(i1, y, j1) != 0)
            {
                if !self.flowing_blocked(i1, y - 1, j1) {
                    c[l] = 0;
                } else {
                    c[l] = self.flowing_distance(i1, y, j1, 1, l as i32, lava);
                }
            }
        }
        let mut min = c[0];
        for &value in c.iter().skip(1) {
            if value < min {
                min = value;
            }
        }
        for (i, &value) in c.iter().enumerate() {
            self.fluid_sides[li][i] = value == min;
        }
        trace_j!(x, y, z, c, self.fluid_sides[li]);
    }

    /// BlockFlowing.f: running minimum of neighboring fluid levels.
    fn flowing_min_side(
        &self,
        x: i32,
        y: i32,
        z: i32,
        l: i32,
        lava: bool,
        counter: &mut i32,
    ) -> i32 {
        let i1 = self.fluid_data_at(x, y, z, lava);
        if i1 < 0 {
            return l;
        }
        if i1 == 0 {
            *counter += 1;
        }
        let i1 = if i1 >= 8 { 0 } else { i1 };
        if l >= 0 && i1 >= l {
            l
        } else {
            i1
        }
    }

    /// BlockFlowing.flow: actually spread the fluid into a position.
    fn fluid_flow(&mut self, x: i32, y: i32, z: i32, level: i32, lava: bool) {
        trace_flow!(x, y, z, level);
        if !self.flowing_can_flow_into(x, y, z, lava) {
            return;
        }
        let id = if lava { blocks::LAVA } else { blocks::WATER };
        let target = self.get_block(x, y, z);
        if target > 0 {
            if lava {
                // BlockFluids.h fizz sound (world.random x2).
                let _ = self.world_random.next_f32();
                let _ = self.world_random.next_f32();
            } else {
                // Block.g -> dropNaturally (world.random; drops are entities).
                trace_rng3!(x, y, z);
            }
        }
        self.java_set_type_id_and_data(x, y, z, id, level as u8);
    }

    /// `BlockFlowing.a(World, i, j, k, Random)` — invoked directly by
    /// WorldGenLiquids after the fluid is placed (scheduled ticks never run
    /// during generation).
    #[allow(clippy::if_same_then_else)]
    pub fn java_fluid_tick(&mut self, x: i32, y: i32, z: i32, random: &mut JavaRandom, lava: bool) {
        let this_id = if lava { blocks::LAVA } else { blocks::WATER };
        // `BlockFlowing` constructor: lava in the Nether spreads with step 1
        // (`worldProvider.d` = isHell) instead of the overworld's 2.
        let b0: i32 = if lava && !self.is_hell { 2 } else { 1 };
        let mut l = self.fluid_data_at(x, y, z, lava);
        trace_fa!(x, y, z, this_id as i32, l);
        let mut flag = true;
        let mut i1;

        if l > 0 {
            let mut counter = 0i32;
            let mut j1 = self.flowing_min_side(x - 1, y, z, -100, lava, &mut counter);
            j1 = self.flowing_min_side(x + 1, y, z, j1, lava, &mut counter);
            j1 = self.flowing_min_side(x, y, z - 1, j1, lava, &mut counter);
            j1 = self.flowing_min_side(x, y, z + 1, j1, lava, &mut counter);
            i1 = j1 + b0;
            if i1 >= 8 || j1 < 0 {
                i1 = -1;
            }
            let above = self.fluid_data_at(x, y + 1, z, lava);
            if above >= 0 {
                i1 = if above >= 8 { above } else { above + 8 };
            }
            if counter >= 2 && !lava {
                if blocks::material_is_buildable(self.get_block(x, y - 1, z)) {
                    i1 = 0;
                } else if self.is_same_fluid_at(x, y - 1, z, lava) && self.get_data(x, y, z) == 0 {
                    i1 = 0;
                }
            }
            if lava && l < 8 && i1 < 8 && i1 > l {
                trace_rng!(x, y, z);
            }
            if lava && l < 8 && i1 < 8 && i1 > l && random.next_i32_bound(4) != 0 {
                // Poseidon flowing-lava fix (`world.settings.flowing-lava-fix.enabled`)
                if !self.mechanics.flowing_lava_fix {
                    i1 = l;
                }
                flag = false;
            }
            if i1 != l {
                l = i1;
                if i1 < 0 {
                    self.java_set_type_id(x, y, z, 0);
                } else {
                    self.java_set_data(x, y, z, i1 as u8);
                    let delay = if lava { 30 } else { 5 };
                    self.java_schedule_tick(x, y, z, this_id, delay);
                    self.java_apply_physics(x, y, z, this_id as i32);
                }
            } else if flag {
                let data = self.get_data(x, y, z);
                trace_conv!(x, y, z, data, this_id + 1);
                self.java_set_raw_type_id_and_data(x, y, z, this_id + 1, data);
            }
        } else {
            let data = self.get_data(x, y, z);
            trace_conv!(x, y, z, data, this_id + 1);
            self.java_set_raw_type_id_and_data(x, y, z, this_id + 1, data);
        }

        if self.flowing_can_flow_into(x, y - 1, z, lava) {
            trace_down!(x, y - 1, z);
            if l >= 8 {
                self.java_set_type_id_and_data(x, y - 1, z, this_id, l as u8);
            } else {
                self.java_set_type_id_and_data(x, y - 1, z, this_id, (l + 8) as u8);
            }
        } else if l >= 0 && (l == 0 || self.flowing_blocked(x, y - 1, z)) {
            self.flowing_sides(x, y, z, lava);
            i1 = l + b0;
            if l >= 8 {
                i1 = 1;
            }
            if i1 >= 8 {
                return;
            }
            // CraftBukkit maps the movement-flag indices to BlockFace in the
            // order NORTH(x-1), SOUTH(x+1), EAST(z-1), WEST(z+1).
            let li = if lava { 1 } else { 0 };
            let offsets = [(-1, 0), (1, 0), (0, -1), (0, 1)];
            for (idx, &(dx, dz)) in offsets.iter().enumerate() {
                // Read the flag fresh: nested ticks may have overwritten the
                // shared BlockFlowing.b array (Java aliasing quirk).
                if self.fluid_sides[li][idx] {
                    self.fluid_flow(x + dx, y, z + dz, i1, lava);
                }
            }
        }
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.java_set_raw_type_id(x, y, z, id);
    }

    pub fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.java_set_raw_type_id_and_data(x, y, z, id, data);
    }

    /// Java `World.e(x, y, z)`: `block.isOpaqueCube() && block.b()`.
    pub fn is_solid_normal_at(&self, x: i32, y: i32, z: i32) -> bool {
        blocks::is_solid_normal_block(self.get_block(x, y, z))
    }

    pub fn get_tile_entity(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<&crate::gameplay::tile::TileEntity> {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        self.chunks.get(&pos)?.get_tile_entity(x, y, z)
    }

    pub fn get_tile_entity_mut(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<&mut crate::gameplay::tile::TileEntity> {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let chunk = self.chunks.get_mut(&pos)?;
        chunk.dirty = true;
        chunk
            .tile_entities
            .iter_mut()
            .find(|te| te.x == x && te.y == y && te.z == z)
    }

    pub fn set_world_tile_entity(&mut self, te: crate::gameplay::tile::TileEntity) {
        let pos = ChunkPos::new(te.x >> 4, te.z >> 4);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.set_tile_entity(te);
            self.packet_cache.borrow_mut().remove(&pos);
        }
    }

    pub fn remove_world_tile_entity(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<crate::gameplay::tile::TileEntity> {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        let chunk = self.chunks.get_mut(&pos)?;
        let idx = chunk
            .tile_entities
            .iter()
            .position(|te| te.x == x && te.y == y && te.z == z)?;
        let removed = chunk.tile_entities.swap_remove(idx);
        chunk.dirty = true;
        self.packet_cache.borrow_mut().remove(&pos);
        Some(removed)
    }

    /// Java `Block.canPlace(world, x, y, z, face)`.
    pub fn can_place_block_at(&self, block_id: u8, x: i32, y: i32, z: i32, face: u8) -> bool {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        let existing = self.get_block(x, y, z);
        if existing != blocks::AIR && !blocks::material(existing).is_replaceable() {
            return false;
        }

        match block_id {
            blocks::TORCH | blocks::REDSTONE_TORCH_OFF | blocks::REDSTONE_TORCH_ON => {
                self.is_solid_normal_at(x - 1, y, z)
                    || self.is_solid_normal_at(x + 1, y, z)
                    || self.is_solid_normal_at(x, y, z - 1)
                    || self.is_solid_normal_at(x, y, z + 1)
                    || self.is_solid_normal_at(x, y - 1, z)
                    || (self.mechanics.allow_1_7_fence_placement
                        && self.get_block(x, y - 1, z) == blocks::FENCE)
            }
            blocks::FENCE => {
                self.mechanics.allow_1_7_fence_placement
                    || blocks::material(self.get_block(x, y - 1, z)).is_buildable()
            }
            blocks::LADDER => {
                self.is_solid_normal_at(x - 1, y, z)
                    || self.is_solid_normal_at(x + 1, y, z)
                    || self.is_solid_normal_at(x, y, z - 1)
                    || self.is_solid_normal_at(x, y, z + 1)
            }
            blocks::STONE_BUTTON => match face {
                2 => self.is_solid_normal_at(x, y, z + 1),
                3 => self.is_solid_normal_at(x, y, z - 1),
                4 => self.is_solid_normal_at(x + 1, y, z),
                5 => self.is_solid_normal_at(x - 1, y, z),
                _ => false,
            },
            blocks::LEVER => match face {
                1 => self.is_solid_normal_at(x, y - 1, z),
                2 => self.is_solid_normal_at(x, y, z + 1),
                3 => self.is_solid_normal_at(x, y, z - 1),
                4 => self.is_solid_normal_at(x + 1, y, z),
                5 => self.is_solid_normal_at(x - 1, y, z),
                _ => false,
            },
            blocks::TRAPDOOR => match face {
                2 => self.is_solid_normal_at(x, y, z + 1),
                3 => self.is_solid_normal_at(x, y, z - 1),
                4 => self.is_solid_normal_at(x + 1, y, z),
                5 => self.is_solid_normal_at(x - 1, y, z),
                _ => false,
            },
            blocks::STONE_PLATE
            | blocks::WOOD_PLATE
            | blocks::REDSTONE_WIRE
            | blocks::RAIL
            | blocks::POWERED_RAIL
            | blocks::DETECTOR_RAIL
            | blocks::REPEATER_OFF
            | blocks::REPEATER_ON
            | blocks::SNOW => self.is_solid_normal_at(x, y - 1, z),
            blocks::PUMPKIN | blocks::JACK_O_LANTERN => {
                self.mechanics.pre_b1_5_pumpkins || self.is_solid_normal_at(x, y - 1, z)
            }
            blocks::CAKE => blocks::material(self.get_block(x, y - 1, z)).is_buildable(),
            blocks::SAPLING | blocks::YELLOW_FLOWER | blocks::RED_ROSE | blocks::TALL_GRASS => {
                matches!(
                    self.get_block(x, y - 1, z),
                    blocks::GRASS | blocks::DIRT | blocks::FARMLAND
                )
            }
            blocks::DEAD_BUSH => self.get_block(x, y - 1, z) == blocks::SAND,
            blocks::BROWN_MUSHROOM | blocks::RED_MUSHROOM => {
                blocks::is_opaque_cube(self.get_block(x, y - 1, z))
            }
            blocks::CROPS => self.get_block(x, y - 1, z) == blocks::FARMLAND,
            blocks::CACTUS => self.cactus_can_survive(x, y, z),
            blocks::REED => self.reed_can_survive(x, y, z),
            blocks::CHEST => {
                let mut adj = 0;
                for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    if self.get_block(x + dx, y, z + dz) == blocks::CHEST {
                        adj += 1;
                        // Cannot place next to an already-double chest
                        for (dx2, dz2) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                            if (dx + dx2 != 0 || dz + dz2 != 0)
                                && self.get_block(x + dx + dx2, y, z + dz + dz2) == blocks::CHEST
                            {
                                return false;
                            }
                        }
                    }
                }
                adj <= 1
            }
            _ => true,
        }
    }

    /// Reproduce `Block.postPlace` + `Block.c` orientation and `TileEntity` setup on player placement.
    #[allow(clippy::too_many_arguments, clippy::if_same_then_else)]
    pub fn post_place_block(
        &mut self,
        block_id: u8,
        x: i32,
        y: i32,
        z: i32,
        face: u8,
        yaw: f32,
        _pitch: f32,
    ) {
        use crate::gameplay::tile::TileEntity;
        let facing_4 = (((yaw * 4.0 / 360.0) + 0.5).floor() as i32) & 3;

        match block_id {
            blocks::TORCH | blocks::REDSTONE_TORCH_OFF | blocks::REDSTONE_TORCH_ON => {
                let mut meta = 0u8;
                let can_floor = self.is_solid_normal_at(x, y - 1, z)
                    || (self.mechanics.allow_1_7_fence_placement
                        && self.get_block(x, y - 1, z) == blocks::FENCE);
                if face == 1 && can_floor {
                    meta = 5;
                } else if face == 2 && self.is_solid_normal_at(x, y, z + 1) {
                    meta = 4;
                } else if face == 3 && self.is_solid_normal_at(x, y, z - 1) {
                    meta = 3;
                } else if face == 4 && self.is_solid_normal_at(x + 1, y, z) {
                    meta = 2;
                } else if face == 5 && self.is_solid_normal_at(x - 1, y, z) {
                    meta = 1;
                } else if self.is_solid_normal_at(x - 1, y, z) {
                    meta = 1;
                } else if self.is_solid_normal_at(x + 1, y, z) {
                    meta = 2;
                } else if self.is_solid_normal_at(x, y, z - 1) {
                    meta = 3;
                } else if self.is_solid_normal_at(x, y, z + 1) {
                    meta = 4;
                } else if can_floor {
                    meta = 5;
                }
                self.java_set_data(x, y, z, meta);
            }
            blocks::LADDER => {
                let mut meta = 0u8;
                if (meta == 0 || face == 2) && self.is_solid_normal_at(x, y, z + 1) {
                    meta = 2;
                }
                if (meta == 0 || face == 3) && self.is_solid_normal_at(x, y, z - 1) {
                    meta = 3;
                }
                if (meta == 0 || face == 4) && self.is_solid_normal_at(x + 1, y, z) {
                    meta = 4;
                }
                if (meta == 0 || face == 5) && self.is_solid_normal_at(x - 1, y, z) {
                    meta = 5;
                }
                self.java_set_data(x, y, z, meta);
            }
            blocks::WOOD_STAIRS | blocks::COBBLESTONE_STAIRS => {
                let meta = match facing_4 {
                    0 => 2,
                    1 => 1,
                    2 => 3,
                    _ => 0,
                };
                self.java_set_data(x, y, z, meta);
            }
            blocks::FURNACE | blocks::LIT_FURNACE | blocks::DISPENSER => {
                let meta = match facing_4 {
                    0 => 2,
                    1 => 5,
                    2 => 3,
                    _ => 4,
                };
                self.java_set_data(x, y, z, meta);
                if block_id == blocks::DISPENSER {
                    self.set_world_tile_entity(TileEntity::new_dispenser(x, y, z));
                } else {
                    self.set_world_tile_entity(TileEntity::new_furnace(x, y, z));
                }
            }
            blocks::CHEST => {
                self.set_world_tile_entity(TileEntity::new_chest(x, y, z));
            }
            blocks::NOTE_BLOCK => {
                self.set_world_tile_entity(TileEntity::new_note(x, y, z));
            }
            blocks::JUKEBOX => {
                self.set_world_tile_entity(TileEntity::new_record_player(x, y, z));
            }
            blocks::MOB_SPAWNER => {
                self.set_world_tile_entity(TileEntity::new_spawner(x, y, z, "Pig".to_string()));
            }
            blocks::PUMPKIN | blocks::JACK_O_LANTERN => {
                let meta = (((yaw * 4.0 / 360.0) + 2.5).floor() as i32 & 3) as u8;
                self.java_set_data(x, y, z, meta);
            }
            blocks::TRAPDOOR => {
                let meta = match face {
                    2 => 0,
                    3 => 1,
                    4 => 2,
                    5 => 3,
                    _ => 0,
                };
                self.java_set_data(x, y, z, meta);
            }
            blocks::STONE_BUTTON => {
                let meta = match face {
                    2 if self.is_solid_normal_at(x, y, z + 1) => 4,
                    3 if self.is_solid_normal_at(x, y, z - 1) => 3,
                    4 if self.is_solid_normal_at(x + 1, y, z) => 2,
                    5 if self.is_solid_normal_at(x - 1, y, z) => 1,
                    _ => 1,
                };
                self.java_set_data(x, y, z, meta);
            }
            blocks::LEVER => {
                let meta = match face {
                    1 if self.is_solid_normal_at(x, y - 1, z) => {
                        5 + (self.world_random.next_i32_bound(2) as u8)
                    }
                    2 if self.is_solid_normal_at(x, y, z + 1) => 4,
                    3 if self.is_solid_normal_at(x, y, z - 1) => 3,
                    4 if self.is_solid_normal_at(x + 1, y, z) => 2,
                    5 if self.is_solid_normal_at(x - 1, y, z) => 1,
                    _ => 5,
                };
                self.java_set_data(x, y, z, meta);
            }
            blocks::REPEATER_OFF | blocks::REPEATER_ON => {
                let meta = (((yaw * 4.0 / 360.0) + 0.5).floor() as i32 & 3) as u8;
                let meta = (meta + 2) & 3;
                self.java_set_data(x, y, z, meta);
                if self.is_repeater_receiving_power(x, y, z, meta) {
                    self.java_schedule_tick(x, y, z, block_id, 1);
                }
            }
            blocks::PISTON | blocks::STICKY_PISTON => {
                let meta = match facing_4 {
                    0 => 2,
                    1 => 5,
                    2 => 3,
                    _ => 4,
                };
                self.java_set_data(x, y, z, meta);
                self.update_piston_state(x, y, z, block_id);
            }
            blocks::STEP if y > 0 && self.get_block(x, y - 1, z) == blocks::STEP => {
                // BlockStep.c: merge onto matching bottom slab
                let top_data = self.get_data(x, y, z);
                let bot_data = self.get_data(x, y - 1, z);
                if top_data == bot_data {
                    self.java_set_type_id(x, y, z, blocks::AIR);
                    self.java_set_type_id_and_data(x, y - 1, z, blocks::DOUBLE_STEP, bot_data);
                }
            }
            _ => {}
        }
    }

    /// Check attached blocks around `(cx, cy, cz)` after a player breaks/places a block,
    /// popping off unsupported torches, ladders, levers, buttons, doors, beds, signs, etc.
    pub fn check_attached_blocks_around(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
    ) -> Vec<(i32, i32, i32, crate::gameplay::items::ItemStack)> {
        let mut drops = Vec::new();
        let neighbors = [
            (cx - 1, cy, cz),
            (cx + 1, cy, cz),
            (cx, cy - 1, cz),
            (cx, cy + 1, cz),
            (cx, cy, cz - 1),
            (cx, cy, cz + 1),
        ];

        for (x, y, z) in neighbors {
            if !(0..CHUNK_HEIGHT as i32).contains(&y) {
                continue;
            }
            let id = self.get_block(x, y, z);
            if id == blocks::AIR {
                continue;
            }
            let data = self.get_data(x, y, z);
            let mut supported = true;

            match id {
                blocks::TORCH | blocks::REDSTONE_TORCH_OFF | blocks::REDSTONE_TORCH_ON => {
                    let can_floor = self.is_solid_normal_at(x, y - 1, z)
                        || self.get_block(x, y - 1, z) == blocks::FENCE;
                    supported = match data {
                        1 => self.is_solid_normal_at(x - 1, y, z),
                        2 => self.is_solid_normal_at(x + 1, y, z),
                        3 => self.is_solid_normal_at(x, y, z - 1),
                        4 => self.is_solid_normal_at(x, y, z + 1),
                        5 => can_floor,
                        _ => false,
                    };
                }
                blocks::LADDER => {
                    supported = match data {
                        2 => self.is_solid_normal_at(x, y, z + 1),
                        3 => self.is_solid_normal_at(x, y, z - 1),
                        4 => self.is_solid_normal_at(x + 1, y, z),
                        5 => self.is_solid_normal_at(x - 1, y, z),
                        _ => false,
                    };
                }
                blocks::STONE_BUTTON => {
                    supported = match data & 7 {
                        1 => self.is_solid_normal_at(x - 1, y, z),
                        2 => self.is_solid_normal_at(x + 1, y, z),
                        3 => self.is_solid_normal_at(x, y, z - 1),
                        4 => self.is_solid_normal_at(x, y, z + 1),
                        _ => false,
                    };
                }
                blocks::LEVER => {
                    supported = match data & 7 {
                        1 => self.is_solid_normal_at(x - 1, y, z),
                        2 => self.is_solid_normal_at(x + 1, y, z),
                        3 => self.is_solid_normal_at(x, y, z - 1),
                        4 => self.is_solid_normal_at(x, y, z + 1),
                        5 | 6 => self.is_solid_normal_at(x, y - 1, z),
                        _ => false,
                    };
                }
                blocks::TRAPDOOR => {
                    supported = match data & 3 {
                        0 => self.is_solid_normal_at(x, y, z + 1),
                        1 => self.is_solid_normal_at(x, y, z - 1),
                        2 => self.is_solid_normal_at(x + 1, y, z),
                        3 => self.is_solid_normal_at(x - 1, y, z),
                        _ => false,
                    };
                }
                blocks::SIGN => {
                    supported = blocks::material(self.get_block(x, y - 1, z)).is_buildable();
                }
                blocks::WALL_SIGN => {
                    supported = match data {
                        2 => blocks::material(self.get_block(x, y, z + 1)).is_buildable(),
                        3 => blocks::material(self.get_block(x, y, z - 1)).is_buildable(),
                        4 => blocks::material(self.get_block(x + 1, y, z)).is_buildable(),
                        5 => blocks::material(self.get_block(x - 1, y, z)).is_buildable(),
                        _ => false,
                    };
                }
                blocks::WOODEN_DOOR | blocks::IRON_DOOR => {
                    if (data & 8) != 0 {
                        if self.get_block(x, y - 1, z) != id {
                            self.java_set_type_id(x, y, z, blocks::AIR);
                        }
                    } else if self.get_block(x, y + 1, z) != id
                        || !self.is_solid_normal_at(x, y - 1, z)
                    {
                        self.java_set_type_id(x, y, z, blocks::AIR);
                        if self.get_block(x, y + 1, z) == id {
                            self.java_set_type_id(x, y + 1, z, blocks::AIR);
                        }
                        supported = false;
                    }
                }
                blocks::BED => {
                    let dir = (data & 3) as usize;
                    let offsets = [(0, 1), (-1, 0), (0, -1), (1, 0)];
                    let (dx, dz) = offsets[dir];
                    if (data & 8) != 0 {
                        // Head of bed: check foot at (x - dx, y, z - dz)
                        if self.get_block(x - dx, y, z - dz) != blocks::BED {
                            self.java_set_type_id(x, y, z, blocks::AIR);
                        }
                    } else if self.get_block(x + dx, y, z + dz) != blocks::BED {
                        self.java_set_type_id(x, y, z, blocks::AIR);
                        supported = false;
                    }
                }
                blocks::STONE_PLATE
                | blocks::WOOD_PLATE
                | blocks::REDSTONE_WIRE
                | blocks::RAIL
                | blocks::POWERED_RAIL
                | blocks::DETECTOR_RAIL
                | blocks::REPEATER_OFF
                | blocks::REPEATER_ON => {
                    supported = self.is_solid_normal_at(x, y - 1, z);
                }
                blocks::CAKE => {
                    supported = blocks::material(self.get_block(x, y - 1, z)).is_buildable();
                }
                _ => {}
            }

            if !supported {
                let _ = self.remove_world_tile_entity(x, y, z);
                self.java_set_type_id(x, y, z, blocks::AIR);
                let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
                let item_drops = blocks::get_block_drops(id, data, &mut rng, None);
                self.world_random = rng;
                for stack in item_drops {
                    drops.push((x, y, z, stack));
                }
            }
        }

        drops
    }

    /// Tick all loaded furnace tile entities (`TileEntityFurnace.g_()`).
    pub fn tick_furnaces(&mut self) {
        let positions: Vec<ChunkPos> = self.chunks.keys().copied().collect();
        let mut block_switches: Vec<(i32, i32, i32, bool)> = Vec::new();

        for pos in positions {
            let Some(chunk) = self.chunks.get_mut(&pos) else {
                continue;
            };
            let mut chunk_dirty = false;
            for te in &mut chunk.tile_entities {
                let (dirty, switch) = te.tick_furnace();
                if dirty {
                    chunk_dirty = true;
                }
                if let Some(burning) = switch {
                    block_switches.push((te.x, te.y, te.z, burning));
                }
            }
            if chunk_dirty {
                chunk.dirty = true;
            }
        }

        for (x, y, z, burning) in block_switches {
            let meta = self.get_data(x, y, z);
            let new_id = if burning {
                blocks::LIT_FURNACE
            } else {
                blocks::FURNACE
            };
            self.java_set_type_id_and_data(x, y, z, new_id, meta);
        }
    }

    /// Java `BlockSapling.b(world, x, y, z, random)` — grow a tree from a sapling.
    pub fn grow_sapling_tree(&mut self, x: i32, y: i32, z: i32) -> bool {
        use crate::world::biome::TreeKind;
        use crate::world::decorate;

        let data = self.get_data(x, y, z) & 3;
        self.java_set_raw_type_id(x, y, z, blocks::AIR);

        let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
        let kind = match data {
            1 if self.worldgen.generate_spruces => TreeKind::Taiga2,
            2 if self.worldgen.generate_birches => TreeKind::Forest,
            _ => {
                if !self.worldgen.pre_b1_2_tree_generation && rng.next_i32_bound(10) == 0 {
                    TreeKind::BigTree
                } else {
                    TreeKind::Trees
                }
            }
        };
        decorate::generate_tree(self, &mut rng, kind, x, y, z);
        self.world_random = rng;

        let grown = self.get_block(x, y, z) == blocks::LOG;
        if !grown {
            self.java_set_raw_type_id_and_data(x, y, z, blocks::SAPLING, data);
            false
        } else {
            let cpos = ChunkPos::new(x >> 4, z >> 4);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let p = ChunkPos::new(cpos.x + dx, cpos.z + dz);
                    self.packet_cache.borrow_mut().remove(&p);
                    if self.live.instances.contains_key(&p) {
                        let entry = self.live.dirty_chunks.entry(p).or_default();
                        entry.full_resync = true;
                    }
                }
            }
            true
        }
    }

    /// Java `ItemDye.a`: apply bone meal to a grass block, growing tall grass and flowers around `(x, y, z)`.
    pub fn apply_bonemeal_to_grass(&mut self, x: i32, y: i32, z: i32) {
        if !self.mechanics.allow_bone_meal_on_grass {
            return;
        }
        let allow_tallgrass = self.mechanics.allow_grow_tallgrass;
        let mut rng = std::mem::replace(&mut self.world_random, JavaRandom::new(0));
        'outer: for j1 in 0..128 {
            let mut kx = x;
            let mut ly = y + 1;
            let mut iz = z;
            for _ in 0..(j1 / 16) {
                kx += rng.next_i32_bound(3) - 1;
                ly += (rng.next_i32_bound(3) - 1) * rng.next_i32_bound(3) / 2;
                iz += rng.next_i32_bound(3) - 1;
                if self.get_block(kx, ly - 1, iz) != blocks::GRASS
                    || self.is_solid_normal_at(kx, ly, iz)
                {
                    continue 'outer;
                }
            }
            if self.get_block(kx, ly, iz) == blocks::AIR {
                if allow_tallgrass && rng.next_i32_bound(10) != 0 {
                    self.java_set_type_id_and_data(kx, ly, iz, blocks::TALL_GRASS, 1);
                } else if rng.next_i32_bound(3) != 0 {
                    self.java_set_type_id(kx, ly, iz, blocks::YELLOW_FLOWER);
                } else {
                    self.java_set_type_id(kx, ly, iz, blocks::RED_ROSE);
                }
            }
        }
        self.world_random = rng;
    }

    fn is_ray_hittable(id: u8, meta: u8, hit_liquids: bool) -> bool {
        if id == blocks::AIR || id == blocks::FIRE || id == blocks::PORTAL {
            return false;
        }
        if blocks::is_liquid(id) {
            return hit_liquids && meta == 0;
        }
        true
    }

    /// Java `World.rayTrace(Vec3D, Vec3D, boolean)` — 3D voxel DDA raytrace.
    pub fn ray_trace(
        &self,
        start: (f64, f64, f64),
        end: (f64, f64, f64),
        hit_liquids: bool,
    ) -> Option<(i32, i32, i32, u8)> {
        let (mut x0, mut y0, mut z0) = start;
        let (x1, y1, z1) = end;
        if x0.is_nan() || y0.is_nan() || z0.is_nan() || x1.is_nan() || y1.is_nan() || z1.is_nan() {
            return None;
        }

        let target_x = x1.floor() as i32;
        let target_y = y1.floor() as i32;
        let target_z = z1.floor() as i32;
        let mut cur_x = x0.floor() as i32;
        let mut cur_y = y0.floor() as i32;
        let mut cur_z = z0.floor() as i32;

        let init_id = self.get_block(cur_x, cur_y, cur_z);
        let init_meta = self.get_data(cur_x, cur_y, cur_z);
        if Self::is_ray_hittable(init_id, init_meta, hit_liquids) {
            return Some((cur_x, cur_y, cur_z, 1));
        }

        for _ in 0..200 {
            if x0.is_nan() || y0.is_nan() || z0.is_nan() {
                return None;
            }
            if cur_x == target_x && cur_y == target_y && cur_z == target_z {
                return None;
            }

            let mut has_x = true;
            let mut has_y = true;
            let mut has_z = true;
            let mut next_bx = 999.0;
            let mut next_by = 999.0;
            let mut next_bz = 999.0;

            if target_x > cur_x {
                next_bx = cur_x as f64 + 1.0;
            } else if target_x < cur_x {
                next_bx = cur_x as f64;
            } else {
                has_x = false;
            }

            if target_y > cur_y {
                next_by = cur_y as f64 + 1.0;
            } else if target_y < cur_y {
                next_by = cur_y as f64;
            } else {
                has_y = false;
            }

            if target_z > cur_z {
                next_bz = cur_z as f64 + 1.0;
            } else if target_z < cur_z {
                next_bz = cur_z as f64;
            } else {
                has_z = false;
            }

            let dx = x1 - x0;
            let dy = y1 - y0;
            let dz = z1 - z0;
            let tx = if has_x { (next_bx - x0) / dx } else { 999.0 };
            let ty = if has_y { (next_by - y0) / dy } else { 999.0 };
            let tz = if has_z { (next_bz - z0) / dz } else { 999.0 };

            let face: u8;
            if tx < ty && tx < tz {
                face = if target_x > cur_x { 4 } else { 5 };
                x0 = next_bx;
                y0 += dy * tx;
                z0 += dz * tx;
            } else if ty < tz {
                face = if target_y > cur_y { 0 } else { 1 };
                x0 += dx * ty;
                y0 = next_by;
                z0 += dz * ty;
            } else {
                face = if target_z > cur_z { 2 } else { 3 };
                x0 += dx * tz;
                y0 += dy * tz;
                z0 = next_bz;
            }

            cur_x = x0.floor() as i32;
            if face == 5 {
                cur_x -= 1;
            }
            cur_y = y0.floor() as i32;
            if face == 1 {
                cur_y -= 1;
            }
            cur_z = z0.floor() as i32;
            if face == 3 {
                cur_z -= 1;
            }

            let id = self.get_block(cur_x, cur_y, cur_z);
            let meta = self.get_data(cur_x, cur_y, cur_z);
            if Self::is_ray_hittable(id, meta, hit_liquids) {
                return Some((cur_x, cur_y, cur_z, face));
            }
        }

        None
    }

    /// Raytrace from a player's eye position (`y + 1.62`) along `(yaw, pitch)` for `distance` blocks,
    /// matching `ItemBoat.a` and `ItemBucket.a`.
    #[allow(clippy::too_many_arguments)]
    pub fn ray_trace_from_player(
        &self,
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        distance: f64,
        hit_liquids: bool,
    ) -> Option<(i32, i32, i32, u8)> {
        let eye_y = y + 1.62;
        let deg_to_rad = 0.017_453_292_f32;
        let f3 = (-yaw * deg_to_rad - std::f32::consts::PI).cos();
        let f4 = (-yaw * deg_to_rad - std::f32::consts::PI).sin();
        let f5 = -(-pitch * deg_to_rad).cos();
        let f6 = (-pitch * deg_to_rad).sin();
        let dir_x = (f4 * f5) as f64;
        let dir_y = f6 as f64;
        let dir_z = (f3 * f5) as f64;
        self.ray_trace(
            (x, eye_y, z),
            (
                x + dir_x * distance,
                eye_y + dir_y * distance,
                z + dir_z * distance,
            ),
            hit_liquids,
        )
    }
}

impl GenWorld for World {
    fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        self.get_block(x, y, z)
    }

    fn get_data(&self, x: i32, y: i32, z: i32) -> u8 {
        self.get_data(x, y, z)
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.set_block(x, y, z, id);
    }

    fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.set_block_and_data(x, y, z, id, data);
    }

    fn set_block_physics(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.java_set_type_id(x, y, z, id);
    }

    fn set_block_and_data_physics(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.java_set_type_id_and_data(x, y, z, id, data);
    }

    fn fluid_tick(&mut self, x: i32, y: i32, z: i32, lava: bool, random: &mut JavaRandom) {
        // WorldGenLiquids: `world.a = true` around the direct tick, which makes
        // every tick scheduled by the cascade below run immediately.
        self.insta_tick = true;
        self.java_fluid_tick(x, y, z, random, lava);
        self.insta_tick = false;
    }

    fn set_tile_entity(&mut self, te: crate::gameplay::tile::TileEntity) {
        let pos = ChunkPos::new(te.x >> 4, te.z >> 4);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.set_tile_entity(te);
        }
    }

    fn highest_block_y(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return 0;
        }
        if self.live.active && self.live_resolve_read(pos) {
            return 0;
        }
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_height((x & 15) as usize, (z & 15) as usize) as i32
        } else {
            0
        }
    }

    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        if y < 0 {
            return 0;
        }
        if y >= CHUNK_HEIGHT as i32 {
            return 15;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if !self.chunks.contains_key(&pos) {
            return if self.live.active { 0 } else { 15 };
        }
        if self.live.active && self.live_resolve_read(pos) {
            return 0;
        }
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_sky_light((x & 15) as usize, y as usize, (z & 15) as usize) as i32
        } else if self.live.active {
            0
        } else {
            15
        }
    }

    fn light_value(&self, x: i32, y: i32, z: i32) -> i32 {
        if y < 0 {
            return 0;
        }
        if y >= CHUNK_HEIGHT as i32 {
            return 15;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active && self.live_resolve_read(pos) {
            return 0;
        }
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.get_block_light_value((x & 15) as usize, y as usize, (z & 15) as usize, 0)
        } else if self.live.active {
            0
        } else {
            15
        }
    }

    fn see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        if y < 0 {
            return false;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live.active && self.live_resolve_read(pos) {
            return false;
        }
        if let Some(chunk) = self.chunks.get(&pos) {
            chunk.can_block_see_sky((x & 15) as usize, y as usize, (z & 15) as usize)
        } else {
            !self.live.active
        }
    }

    fn highest_solid_y(&self, x: i32, z: i32) -> i32 {
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            for y in (1..CHUNK_HEIGHT as i32).rev() {
                let id = chunk.get_block((x & 15) as usize, y as usize, (z & 15) as usize);
                if blocks::is_solid(id) || blocks::is_liquid(id) {
                    return y + 1;
                }
            }
        }
        -1
    }

    fn is_normal_solid(&self, x: i32, y: i32, z: i32) -> bool {
        blocks::is_normal_solid(self.get_block(x, y, z))
    }

    fn biome_at(&self, x: i32, z: i32) -> Biome {
        self.chunk_mgr.borrow_mut().biome_at(x, z)
    }

    fn worldgen_config(&self) -> crate::config::WorldgenConfig {
        self.worldgen.clone()
    }
}

impl LightingWorld for World {
    fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        self.get_block(x, y, z)
    }

    fn get_light(&self, light_type: LightType, x: i32, y: i32, z: i32) -> u8 {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return if light_type == LightType::Sky { 15 } else { 0 };
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get(&pos) {
            match light_type {
                LightType::Sky => {
                    chunk.get_sky_light((x & 15) as usize, y as usize, (z & 15) as usize)
                }
                LightType::Block => {
                    chunk.get_block_light((x & 15) as usize, y as usize, (z & 15) as usize)
                }
            }
        } else if light_type == LightType::Sky {
            15
        } else {
            0
        }
    }

    fn set_light(&mut self, light_type: LightType, x: i32, y: i32, z: i32, val: u8) {
        if !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            match light_type {
                LightType::Sky => {
                    chunk.set_sky_light((x & 15) as usize, y as usize, (z & 15) as usize, val);
                }
                LightType::Block => {
                    chunk.set_block_light((x & 15) as usize, y as usize, (z & 15) as usize, val);
                }
            }
            chunk.dirty = true;
        }
        self.packet_cache.borrow_mut().remove(&pos);
    }

    fn can_see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        self.see_sky(x, y, z)
    }

    fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool {
        self.is_chunk_loaded(ChunkPos::new(cx, cz))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_creates_and_populates_chunk() {
        let config = WorldConfig {
            seed: 12345,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        let pos = ChunkPos::new(0, 0);
        world.populate_chunk(pos);

        let chunk = world.get_chunk(pos).unwrap();
        assert_eq!(chunk.state, ChunkState::Populated);
        assert!(world.chunk_count() >= 9);

        let (pre, map) = world.prepare_chunk_packets(pos).unwrap();
        assert_eq!(pre.x, 0);
        assert_eq!(pre.z, 0);
        assert!(pre.mode);
        assert_eq!(map.x, 0);
        assert_eq!(map.z, 0);
        assert!(!map.compressed.is_empty());
    }

    #[test]
    fn parallel_generation_and_compression_match() {
        let config = WorldConfig {
            seed: 12345,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        let positions = [
            ChunkPos::new(0, 0),
            ChunkPos::new(1, 0),
            ChunkPos::new(0, 1),
            ChunkPos::new(-1, -1),
        ];

        world.generate_base_chunks_parallel(&positions);
        for &pos in &positions {
            assert!(world.is_chunk_loaded(pos));
        }

        let packets = world.compress_chunks_parallel(&positions).unwrap();
        assert_eq!(packets.len(), positions.len());
        for (i, (pre, map)) in packets.iter().enumerate() {
            assert_eq!(pre.x, positions[i].x);
            assert_eq!(pre.z, positions[i].z);
            assert_eq!(map.x, positions[i].x * 16);
            assert_eq!(map.z, positions[i].z * 16);
            assert!(!map.compressed.is_empty());
        }
    }
}
