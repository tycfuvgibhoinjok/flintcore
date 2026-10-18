//! Live (server) chunk choreography, ported 1:1 from Project-Poseidon.
//!
//! The offline generator (`World::populate_chunk`) pre-generates neighbours
//! and populates unconditionally. The real server never works that way: it
//! loads chunks on demand and populates a chunk only when the trigger block
//! `(i,j), (i+1,j), (i,j+1), (i+1,j+1)` is complete. This module reproduces
//! the server-side choreography on top of the same generator:
//!
//! * `World.c()`            — spawn search (top block == sand, `isLoading` on).
//! * `MinecraftServer`      — spawn-region prep, a `(spawn-196)..(spawn+188)`
//!   square loaded x-major at startup.
//! * `ChunkProviderServer`  — chunk load plus the 2x2 populate triggers, the
//!   server-level populate wrapper, the unload queue
//!   and `keepSpawnInMemory` (radius 128).
//! * `PlayerManager`        — the `addPlayer` spiral, `movePlayer` frame moves
//!   and `removePlayer`, all of which drive loads.
//! * `World.getChunkAt`     — the `lastChunkAccessed` cache, which can serve a
//!   stale `EmptyChunk` for missing targets.
//!
//! Empty-chunk semantics: `EmptyChunk.getTypeId/getData` return 0, its writes
//! are silently accepted (`a(...)`/`b(...)` return true) and they do *not*
//! touch any data, while `World.update()` still runs its physics pass over the
//! (possibly loaded) neighbours.

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::gameplay::blocks;
use crate::util::rng::JavaRandom;
use crate::world::chunk::{ChunkPos, ChunkState};
use crate::world::world::{with_generator, World};

#[derive(Debug, Clone)]
pub struct DirtyChunk {
    pub blocks: [u16; 64],
    pub count: usize,
}

impl Default for DirtyChunk {
    fn default() -> Self {
        Self {
            blocks: [0; 64],
            count: 0,
        }
    }
}

/// Shared state for the live/server choreography.
pub struct LiveState {
    /// Gates the EmptyChunk semantics and the `lastChunkAccessed` cache in
    /// `World`. Offline generation keeps it off so the byte-exact offline
    /// paths stay untouched.
    pub active: bool,
    /// Java `World.isLoading` (set during the spawn search). While set,
    /// `getOrCreateChunk` generates chunks instead of returning `emptyChunk`.
    pub is_loading: bool,
    /// Java `ChunkProviderServer.forceChunkLoad`.
    pub force_chunk_load: bool,
    /// Cached `World.getChunkAt` target: `(cx, cz, resolves-to-empty)`.
    pub last_access: Cell<Option<(i32, i32, bool)>>,
    /// `PlayerManager.b`: chunk -> set of players watching it. A present key
    /// means a `PlayerInstance` exists (which keeps the chunk loaded).
    pub instances: HashMap<ChunkPos, BTreeSet<u64>>,
    /// `PlayerInstance.dirtyBlocks` / `dirtyCount` per watched chunk.
    pub dirty_chunks: HashMap<ChunkPos, DirtyChunk>,
    /// Per-player chunks added via `PlayerInstance.a(player)`.
    pub player_added: HashMap<u64, Vec<ChunkPos>>,
    /// Per-player chunks removed via `PlayerInstance.b(player)`.
    pub player_removed: HashMap<u64, Vec<ChunkPos>>,
    /// `EntityPlayer.d`/`e`: the position that opened the current view square.
    pub anchors: HashMap<u64, (f64, f64)>,
    /// `ChunkProviderServer.unloadQueue` (insertion-ordered; CraftBukkit's
    /// LongHashset pops by hash order, which is content-neutral for us).
    pub unload_queue: VecDeque<ChunkPos>,
    /// `PlayerManager.f` — view distance in chunks (server.properties).
    pub view_distance: i32,
    /// `World.keepSpawnInMemory` (bukkit.yml).
    pub keep_spawn_in_memory: bool,
}

impl LiveState {
    pub fn new(view_distance: i32) -> Self {
        Self {
            active: false,
            is_loading: false,
            force_chunk_load: false,
            last_access: Cell::new(None),
            instances: HashMap::new(),
            dirty_chunks: HashMap::new(),
            player_added: HashMap::new(),
            player_removed: HashMap::new(),
            anchors: HashMap::new(),
            unload_queue: VecDeque::new(),
            view_distance,
            keep_spawn_in_memory: true,
        }
    }
}

/// `PlayerManager.g`: left-turning direction table (+x, +z, -x, -z).
const SPIRAL: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// `PlayerManager.a(i, j, k, l)`: is `(i, j)` inside the `f`-square of `(k, l)`?
fn in_view_square(x: i32, z: i32, cx: i32, cz: i32, f: i32) -> bool {
    let dx = x - cx;
    let dz = z - cz;
    dx >= -f && dx <= f && dz >= -f && dz <= f
}

impl World {
    /// Turns on the live/server choreography (EmptyChunk semantics etc.).
    pub fn live_enable(&mut self) {
        self.live.active = true;
    }

    pub fn live_is_active(&self) -> bool {
        self.live.active
    }

    /// Current movement anchor (`EntityPlayer.d`/`e`) for a player.
    pub fn live_anchor(&self, id: u64) -> Option<(f64, f64)> {
        self.live.anchors.get(&id).copied()
    }

    // ------------------------------------------------------------ spawn search

    /// Java `World.c()`: place the spawn on a column whose top block is sand.
    ///
    /// Java seeds `World.random` from system entropy (`new Random()`), so the
    /// found spawn is not reproducible between world creations; callers pass
    /// an explicit seed for reproducible runs.
    pub fn live_find_spawn(&mut self, rng_seed: i64) -> (i32, i32) {
        self.world_random = JavaRandom::new(rng_seed);
        // `this.Q = this.random.nextInt(12000);` from the world constructor.
        let _ = self.world_random.next_i32_bound(12000);

        self.live.is_loading = true;
        let mut i = 0;
        let mut j = 0;
        let mut attempts = 0;
        loop {
            if self.live_can_spawn(i, j) {
                break;
            }
            i += self.world_random.next_i32_bound(64) - self.world_random.next_i32_bound(64);
            attempts += 1;
            if attempts > 1024 {
                i = 0;
                j = 0;
                break;
            }
            j += self.world_random.next_i32_bound(64) - self.world_random.next_i32_bound(64);
        }
        self.live.is_loading = false;
        self.set_spawn_xyz(i, 64, j);
        (i, j)
    }

    /// `World.canSpawn(x, z)` -> `WorldProvider.canSpawn`: the top block of the
    /// column is sand.
    fn live_can_spawn(&mut self, x: i32, z: i32) -> bool {
        self.live_world_top_block(x, z) == blocks::SAND
    }

    /// Java `World.a(i, j)`: scan up from `y = 63` to the first empty cell and
    /// return the block below it.
    fn live_world_top_block(&mut self, x: i32, z: i32) -> u8 {
        let mut k = 63;
        while self.live_world_get_type(x, k + 1, z) != 0 {
            k += 1;
        }
        self.live_world_get_type(x, k, z)
    }

    /// `World.getTypeId` through the cached `getChunkAt`, with the loading
    /// branch active while the spawn search has `isLoading` set.
    fn live_world_get_type(&mut self, x: i32, y: i32, z: i32) -> u8 {
        if !(0..128).contains(&y) {
            return 0;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        if self.live_access(pos) {
            return 0;
        }
        self.chunks
            .get(&pos)
            .unwrap()
            .get_block((x & 15) as usize, y as usize, (z & 15) as usize)
    }

    /// `World.getChunkAt` + `getOrCreateChunk` resolution from `&mut self`:
    /// generates the chunk when it is missing and `isLoading`/`forceChunkLoad`
    /// is set; otherwise reports a missing chunk as empty.
    fn live_access(&mut self, pos: ChunkPos) -> bool {
        if let Some((cx, cz, empty)) = self.live.last_access.get() {
            if cx == pos.x && cz == pos.z {
                return empty;
            }
        }
        let mut empty = !self.chunks.contains_key(&pos);
        if empty && (self.live.is_loading || self.live.force_chunk_load) {
            self.server_get_chunk_at(pos);
            empty = !self.chunks.contains_key(&pos);
        }
        self.live.last_access.set(Some((pos.x, pos.z, empty)));
        empty
    }

    /// Read-side resolution for `&self` code paths (`get_block`/`get_data`).
    /// Identical to [`World::live_access`] except that it cannot generate
    /// chunks: the loading branch only runs during the spawn search, whose
    /// reads all go through `live_access`.
    pub(crate) fn live_resolve_read(&self, pos: ChunkPos) -> bool {
        if let Some((cx, cz, empty)) = self.live.last_access.get() {
            if cx == pos.x && cz == pos.z {
                return empty;
            }
        }
        let empty = !self.chunks.contains_key(&pos);
        self.live.last_access.set(Some((pos.x, pos.z, empty)));
        empty
    }

    /// Write-side resolution: returns true when the target currently resolves
    /// to `EmptyChunk`, i.e. the write must be accepted silently.
    pub(crate) fn live_resolve_write(&mut self, pos: ChunkPos) -> bool {
        if let Some((cx, cz, empty)) = self.live.last_access.get() {
            if cx == pos.x && cz == pos.z {
                return empty;
            }
        }
        let missing = !self.chunks.contains_key(&pos);
        if missing && (self.live.is_loading || self.live.force_chunk_load) {
            self.server_get_chunk_at(pos);
        }
        let empty = !self.chunks.contains_key(&pos);
        self.live.last_access.set(Some((pos.x, pos.z, empty)));
        empty
    }

    // ------------------------------------------------------- provider server

    /// Java `ChunkProviderServer.getChunkAt(i, j)`: load (or generate) a
    /// chunk and run the 2x2 populate trigger block.
    pub fn server_get_chunk_at(&mut self, pos: ChunkPos) {
        // `this.unloadQueue.remove(i, j);`
        self.live.unload_queue.retain(|p| *p != pos);
        if self.chunks.contains_key(&pos) {
            return;
        }

        let chunk = match self.try_load_chunk_from_storage(pos) {
            Some(chunk) => chunk,
            None => self.generate_base_chunk(pos),
        };
        self.chunks.insert(pos, chunk);

        // `chunk.loadNOP()` / `chunk.addEntities()` only touch entity payload.

        self.server_populate_triggers(pos);
    }

    /// The trigger block of `ChunkProviderServer.getChunkAt`. Conditions and
    /// order follow the Java source character for character.
    fn server_populate_triggers(&mut self, pos: ChunkPos) {
        let (i, j) = (pos.x, pos.z);

        if !self.chunk_populated(i, j)
            && self.is_chunk_loaded(ChunkPos::new(i + 1, j + 1))
            && self.is_chunk_loaded(ChunkPos::new(i, j + 1))
            && self.is_chunk_loaded(ChunkPos::new(i + 1, j))
        {
            self.server_populate(ChunkPos::new(i, j));
        }

        if self.is_chunk_loaded(ChunkPos::new(i - 1, j))
            && !self.chunk_populated(i - 1, j)
            && self.is_chunk_loaded(ChunkPos::new(i - 1, j + 1))
            && self.is_chunk_loaded(ChunkPos::new(i, j + 1))
        {
            self.server_populate(ChunkPos::new(i - 1, j));
        }

        if self.is_chunk_loaded(ChunkPos::new(i, j - 1))
            && !self.chunk_populated(i, j - 1)
            && self.is_chunk_loaded(ChunkPos::new(i + 1, j - 1))
            && self.is_chunk_loaded(ChunkPos::new(i + 1, j))
        {
            self.server_populate(ChunkPos::new(i, j - 1));
        }

        if self.is_chunk_loaded(ChunkPos::new(i - 1, j - 1))
            && !self.chunk_populated(i - 1, j - 1)
            && self.is_chunk_loaded(ChunkPos::new(i, j - 1))
            && self.is_chunk_loaded(ChunkPos::new(i - 1, j))
        {
            self.server_populate(ChunkPos::new(i - 1, j - 1));
        }
    }

    fn chunk_populated(&self, x: i32, z: i32) -> bool {
        self.chunks
            .get(&ChunkPos::new(x, z))
            .map(|chunk| chunk.state == ChunkState::Populated)
            .unwrap_or(true)
    }

    /// Java `ChunkProviderServer.getChunkAt(IChunkProvider, i, j)`: the
    /// server-level populate. The `done` flag is set before the generator
    /// runs, and the chunk is marked dirty afterwards (`chunk.f()`).
    fn server_populate(&mut self, pos: ChunkPos) {
        if self.chunk_populated(pos.x, pos.z) {
            return;
        }
        self.chunks.get_mut(&pos).unwrap().state = ChunkState::Populating;

        let seed = self.seed();
        with_generator(seed, |gen| {
            gen.populate(self, pos.x, pos.z);
        });

        let chunk = self.chunks.get_mut(&pos).unwrap();
        chunk.state = ChunkState::Populated;
        chunk.dirty = true;
        for dz in 0..=1 {
            for dx in 0..=1 {
                let cpos = ChunkPos::new(pos.x + dx, pos.z + dz);
                if let Some(c) = self.chunks.get_mut(&cpos) {
                    c.compact();
                }
                self.packet_cache.borrow_mut().remove(&cpos);
                if self.live.instances.contains_key(&cpos) {
                    let entry = self.live.dirty_chunks.entry(cpos).or_default();
                    entry.count = 10;
                }
            }
        }
    }

    // ---------------------------------------------------------- spawn prep

    /// `MinecraftServer` startup: load the spawn region. The Java loop runs
    /// `i1`/`j1` over `-196..=196` with step 16, so the last stop is `188`:
    /// chunk square `((spawn-196)>>4)..=((spawn+188)>>4)`, x-major.
    pub fn live_prepare_spawn_region(&mut self) {
        let (sx, sz) = (self.spawn_point().0, self.spawn_point().2);
        let mut i1 = -196;
        while i1 <= 196 {
            let mut j1 = -196;
            while j1 <= 196 {
                self.server_get_chunk_at(ChunkPos::new((sx + i1) >> 4, (sz + j1) >> 4));
                while self.java_do_lighting() {}
                j1 += 16;
            }
            i1 += 16;
        }
    }

    // -------------------------------------------------------- player manager

    /// Java `PlayerManager.addPlayer`: open the view square around the join
    /// position with the unrolled spiral (`f` = view distance).
    pub fn live_player_join(&mut self, id: u64, loc_x: f64, loc_z: f64) {
        let i = (loc_x as i32) >> 4;
        let j = (loc_z as i32) >> 4;

        // `entityplayer.d = entityplayer.locX; entityplayer.e = ...`
        self.live.anchors.insert(id, (loc_x, loc_z));

        // `this.a(i, j, true).a(entityplayer)` — the centre instance loads the
        // chunk through the provider.
        self.instance_add(ChunkPos::new(i, j), id);

        let f = self.live.view_distance;
        let mut k: usize = 0;
        let mut i1 = 0;
        let mut j1 = 0;
        for k1 in 1..=(f * 2) {
            for _ in 0..2 {
                let d = SPIRAL[k % 4];
                k += 1;
                for _ in 0..k1 {
                    i1 += d.0;
                    j1 += d.1;
                    self.instance_add(ChunkPos::new(i + i1, j + j1), id);
                }
            }
        }
        k %= 4;
        let d = SPIRAL[k];
        for _ in 0..(f * 2) {
            i1 += d.0;
            j1 += d.1;
            self.instance_add(ChunkPos::new(i + i1, j + j1), id);
        }

        // `ServerConfigurationManager.a`: the player chunk through the
        // provider again (normally a no-op right after the spiral).
        self.server_get_chunk_at(ChunkPos::new(i, j));
    }

    /// Java `PlayerManager.movePlayer`: when the player left the 8-block
    /// anchor square into a different chunk, open the new view square and
    /// release the diverging chunks.
    pub fn live_player_move(&mut self, id: u64, loc_x: f64, loc_z: f64) {
        let Some(&(ax, az)) = self.live.anchors.get(&id) else {
            return;
        };
        let i = (loc_x as i32) >> 4;
        let j = (loc_z as i32) >> 4;
        let d0 = ax - loc_x;
        let d1 = az - loc_z;
        if d0 * d0 + d1 * d1 < 64.0 {
            return;
        }
        let k = (ax as i32) >> 4;
        let l = (az as i32) >> 4;
        let di = i - k;
        let dj = j - l;
        if di == 0 && dj == 0 {
            return;
        }

        let f = self.live.view_distance;
        for k1 in (i - f)..=(i + f) {
            for l1 in (j - f)..=(j + f) {
                if !in_view_square(k1, l1, k, l, f) {
                    self.instance_add(ChunkPos::new(k1, l1), id);
                }
                if !in_view_square(k1 - di, l1 - dj, i, j, f) {
                    self.instance_remove(ChunkPos::new(k1 - di, l1 - dj), id);
                }
            }
        }

        self.live.anchors.insert(id, (loc_x, loc_z));
    }

    /// Java `PlayerManager.removePlayer`: release the view square around the
    /// last frame anchor.
    pub fn live_player_quit(&mut self, id: u64) {
        if let Some(&(ax, az)) = self.live.anchors.get(&id) {
            let i = (ax as i32) >> 4;
            let j = (az as i32) >> 4;
            let f = self.live.view_distance;
            for k1 in (i - f)..=(i + f) {
                for l1 in (j - f)..=(j + f) {
                    self.instance_remove(ChunkPos::new(k1, l1), id);
                }
            }
        }
        self.live.anchors.remove(&id);
        self.live.player_added.remove(&id);
        self.live.player_removed.remove(&id);
    }

    pub fn take_player_chunk_events(&mut self, id: u64) -> (Vec<ChunkPos>, Vec<ChunkPos>) {
        let added = self.live.player_added.remove(&id).unwrap_or_default();
        let removed = self.live.player_removed.remove(&id).unwrap_or_default();
        (added, removed)
    }

    /// `PlayerManager.flagDirty(x, y, z)`: record a block modification in a watched chunk.
    pub fn flag_dirty(&mut self, x: i32, y: i32, z: i32) {
        if !(0..128).contains(&y) {
            return;
        }
        let pos = ChunkPos::new(x >> 4, z >> 4);
        self.packet_cache.borrow_mut().remove(&pos);
        if self.live.instances.contains_key(&pos) {
            let entry = self.live.dirty_chunks.entry(pos).or_default();
            if entry.count < 64 {
                let short1 = (((x & 15) as u16) << 12) | (((z & 15) as u16) << 8) | (y as u16);
                if !entry.blocks[..entry.count].contains(&short1) {
                    entry.blocks[entry.count] = short1;
                    entry.count += 1;
                }
            }
        }
    }

    /// `PlayerManager.flush()`: drain all dirty chunk records accumulated this tick.
    pub fn take_dirty_chunks(&mut self) -> Vec<(ChunkPos, DirtyChunk, Vec<u64>)> {
        let dirty = std::mem::take(&mut self.live.dirty_chunks);
        let mut out = Vec::with_capacity(dirty.len());
        for (pos, entry) in dirty {
            if entry.count > 0 {
                if let Some(watchers) = self.live.instances.get(&pos) {
                    out.push((pos, entry, watchers.iter().copied().collect()));
                }
            }
        }
        out
    }

    /// Queue any loaded chunks that have no `PlayerInstance` and lie outside
    /// the `keepSpawnInMemory` radius (used to clean up spawn-search / prep edge chunks).
    pub fn queue_unwatched_chunks(&mut self) {
        let positions: Vec<ChunkPos> = self.chunks.keys().copied().collect();
        for pos in positions {
            if !self.live.instances.contains_key(&pos) {
                self.queue_unload(pos);
            }
        }
    }

    /// `PlayerManager.a(i, j, true)` + `PlayerInstance.a(player)`: create the
    /// instance (loading the chunk) if needed and register the player.
    fn instance_add(&mut self, pos: ChunkPos, id: u64) {
        if !self.live.instances.contains_key(&pos) {
            // `new PlayerInstance(...)` loads through the provider.
            self.server_get_chunk_at(pos);
            self.live.instances.insert(pos, BTreeSet::new());
        }
        if self.live.instances.get_mut(&pos).unwrap().insert(id) {
            self.live.player_added.entry(id).or_default().push(pos);
        }
    }

    /// `PlayerInstance.b(player)`: unregister; an empty instance queues the
    /// chunk for unload.
    fn instance_remove(&mut self, pos: ChunkPos, id: u64) {
        let (removed_player, emptied) = match self.live.instances.get_mut(&pos) {
            Some(players) => {
                let removed = players.remove(&id);
                (removed, players.is_empty())
            }
            None => (false, false),
        };
        if removed_player {
            self.live.player_removed.entry(id).or_default().push(pos);
        }
        if emptied {
            self.live.instances.remove(&pos);
            self.live.dirty_chunks.remove(&pos);
            self.queue_unload(pos);
        }
    }

    /// Java `ChunkProviderServer.queueUnload`: chunks farther than 128 blocks
    /// from spawn (when `keepSpawnInMemory` is set) are queued for unload.
    pub fn queue_unload(&mut self, pos: ChunkPos) {
        let (sx, sz) = (self.spawn_point().0, self.spawn_point().2);
        let k = pos.x * 16 + 8 - sx;
        let l = pos.z * 16 + 8 - sz;
        if (!(-128..=128).contains(&k)
            || !(-128..=128).contains(&l)
            || !self.live.keep_spawn_in_memory)
            && !self.live.unload_queue.contains(&pos)
        {
            self.live.unload_queue.push_back(pos);
        }
    }

    /// `World.doTick` -> `ChunkProviderServer.unloadChunks`: up to 50 queued
    /// chunks are saved and dropped from memory each tick.
    pub fn live_tick(&mut self) {
        self.live_tick_ex(true);
    }

    /// Mirror `ChunkProviderServer.unloadChunks()` directly (unload up to 50
    /// chunks from `unloadQueue` without advancing world time or running
    /// scheduled ticks).
    pub fn live_unload_chunks(&mut self) {
        let mut unloaded = 0usize;
        for _ in 0..50 {
            let Some(pos) = self.live.unload_queue.pop_front() else {
                break;
            };
            if self.live.instances.contains_key(&pos) {
                continue;
            }
            if !self.chunks.contains_key(&pos) {
                continue;
            }
            self.falling_sands
                .retain(|e| (e.x >> 4) != pos.x || (e.z >> 4) != pos.z);
            let _ = self.write_chunk_to_storage(pos);
            self.chunks.remove(&pos);
            self.packet_cache.borrow_mut().remove(&pos);
            self.live.dirty_chunks.remove(&pos);
            self.light_dirty_chunks.remove(&pos);
            if let Some((cx, cz, _)) = self.live.last_access.get() {
                if cx == pos.x && cz == pos.z {
                    self.live.last_access.set(None);
                }
            }
            unloaded += 1;
        }
        if unloaded > 0 && self.live.unload_queue.is_empty() {
            if self.chunks.capacity() > self.chunks.len() * 2 + 64 {
                self.chunks.shrink_to_fit();
            }
            self.live.unload_queue.shrink_to_fit();
            self.packet_cache.borrow_mut().shrink_to_fit();
            crate::util::trim_heap_memory();
        }
    }

    /// One `World.doTick()` for the server path: unload up to 50 released
    /// chunks, advance the world time, then process due scheduled ticks
    /// (`doTick` order: `unloadChunks` -> `worldData.a(f + 1)` -> `a(false)`).
    /// Random block ticks (`World.j()`) are not modelled: their positions come
    /// from the unseeded `World.g` LCG.
    pub fn live_tick_ex(&mut self, unloads: bool) {
        if unloads {
            self.live_unload_chunks();
        }
        self.live_time += 1;
        self.level_data.borrow_mut().time = self.live_time;
        self.java_process_scheduled(false);
        while self.java_do_lighting() {}
        self.java_tick_falling_sands();
    }

    /// Current world time in ticks.
    pub fn live_time(&self) -> i64 {
        self.live_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;

    #[test]
    fn player_join_spiral_covers_exact_view_square() {
        let config = WorldConfig {
            seed: 100,
            save_enabled: false,
            view_distance: 2,
            ..Default::default()
        };
        let mut world = World::new(config);
        world.live_enable();
        world.live_player_join(1, 8.5, 8.5);
        // View distance 2 -> (2*2 + 1)^2 = 25 chunks around (0, 0).
        assert_eq!(world.chunk_count(), 25);
        for cx in -2..=2 {
            for cz in -2..=2 {
                assert!(
                    world.is_chunk_loaded(ChunkPos::new(cx, cz)),
                    "missing spiral chunk ({cx}, {cz})"
                );
            }
        }
    }

    #[test]
    fn player_move_requires_distance_and_chunk_change() {
        let config = WorldConfig {
            seed: 100,
            save_enabled: false,
            view_distance: 1,
            ..Default::default()
        };
        let mut world = World::new(config);
        world.live_enable();
        world.live_player_join(1, 14.0, 8.0);
        assert_eq!(world.live_anchor(1), Some((14.0, 8.0)));

        // Cross into chunk (1, 0) at x=17.0, but d^2 = 9.0 < 64.0 -> no frame.
        world.live_player_move(1, 17.0, 8.0);
        assert_eq!(world.live_anchor(1), Some((14.0, 8.0)));
        assert!(!world.is_chunk_loaded(ChunkPos::new(2, 0)));

        // Move >= 8 blocks within the same chunk (0, 0): x=2.0 -> d^2 = 144 >= 64,
        // but chunk is still (0, 0) -> no frame.
        world.live_player_move(1, 2.0, 8.0);
        assert_eq!(world.live_anchor(1), Some((14.0, 8.0)));

        // Move >= 8 blocks into chunk (1, 0): x=24.0 -> d^2 = 100 >= 64 and chunk (1, 0) != (0, 0).
        world.live_player_move(1, 24.0, 8.0);
        assert_eq!(world.live_anchor(1), Some((24.0, 8.0)));
        assert!(world.is_chunk_loaded(ChunkPos::new(2, 0)));
    }

    #[test]
    fn spawn_search_finds_sand_top_block() {
        let config = WorldConfig {
            seed: 42,
            save_enabled: false,
            view_distance: 2,
            ..Default::default()
        };
        let mut world = World::new(config);
        world.live_enable();
        let (sx, sz) = world.live_find_spawn(12345);
        assert_eq!((sx, sz), (-45, -3));
        assert_eq!(world.live_world_top_block(sx, sz), blocks::SAND);
    }

    #[test]
    fn walking_forward_unloads_chunks_and_evicts_packet_cache() {
        let config = WorldConfig {
            seed: 100,
            save_enabled: false,
            view_distance: 2,
            ..Default::default()
        };
        let mut world = World::new(config);
        world.live_enable();
        world.live.keep_spawn_in_memory = false;

        world.live_player_join(1, 8.0, 8.0);
        for step in 1..=25 {
            let wx = (step * 16 + 8) as f64;
            world.live_player_move(1, wx, 8.0);
            let (added, _removed) = world.take_player_chunk_events(1);
            for pos in added {
                let _ = world.prepare_chunk_packets(pos);
            }
            world.live_tick();
        }

        // View distance 2 -> (2*2 + 1)^2 = 25 chunks. Even after walking 25 chunks
        // (which would load 25 + 25*5 = 150 chunks without unloading), both
        // `world.chunks` and `world.packet_cache` must stay strictly bounded to 25!
        assert_eq!(world.chunk_count(), 25);
        assert!(world.packet_cache.borrow().len() <= 25);
    }
}
