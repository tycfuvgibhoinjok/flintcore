# Architecture

This document describes **how `flintcore` is built**: the concurrency model, the
tick loop, module responsibilities, core data structures, and the invariants
that must never be violated.

It is intentionally focused on the parts that are hard to rediscover and slow to
change. Wire-format constants, block tables and packet layouts live in code
(where the compiler and tests keep them honest) rather than in prose.

Read [README.md](README.md) first for goals and scope, and
[DEVELOPMENT.md](DEVELOPMENT.md) for coding and workflow rules.

---

## 1. Design philosophy

Three ideas drive every decision:

1. **One writer for game state.** A single simulation thread owns the world,
   entities, inventories and plugins. This removes an entire class of bugs,
   keeps behavior deterministic, and matches how the original server (and the
   client) reasons about time.
2. **Parallelize the edges, not the simulation.** Networking, world generation,
   region I/O and compression are parallelized aggressively. The tick itself is
   not.
3. **Data-oriented, not object-oriented.** Hot data (chunks, entities, block
   properties) is laid out in arrays and tables for cache efficiency, not as a
   graph of boxed objects.

The reference implementation this design learns from is Betrock++ (C++): a
single-threaded tick with off-thread chunk generation, region I/O, compression
and socket writes. Poseidon provides the behavioral truth; UberBukkit provides
the protocol-capability pattern.

---

## 2. Concurrency model

### 2.1 Thread topology

| Owner | Count | Responsibility |
|---|---|---|
| Tokio worker pool | `ncpu` (runtime-managed) | TCP accept, per-connection read/decode, write/encode, timeouts, async auth |
| Simulation thread | 1 | Game state, tick loop, event dispatch, plugin host |
| Rayon pool | `ncpu - 1` | Chunk terrain generation (CPU-bound) |
| Region I/O pool | configurable (default 2–4) | `.mcr` region read/write, NBT (blocking file I/O) |
| Compression pool | configurable (default 2–4) | libdeflate chunk/region compression |

The simulation thread is created from `main` and is **not** a Tokio worker. It
must never `await`, never block on I/O, and never hold a lock that a network task
could hold. Tokio tasks must never touch `World`/`EntityStore` directly.

### 2.2 Data flow

```
 client ──TCP──▶ Tokio read task ──decode──▶ [inbound queue] ──▶ SIM THREAD
                                                                     │
                                                     tick mutates state
                                                                     │
 client ◀─TCP── Tokio write task ◀─encode── [outbound queue] ◀───────┘
```

- **Inbound:** each connection decodes packets into owned, `Send` packet values
  and pushes them onto a shared inbound channel. The sim thread drains it at the
  start of each tick.
- **Outbound:** the sim thread pushes owned outbound messages (packet values or
  pre-serialized frames) onto a per-connection bounded channel. The connection's
  write task drains and writes them.
- **Heavy results:** generation, load and compression jobs are submitted to
  pools; results come back to the sim thread via result channels and are
  **integrated only on the sim thread**.

### 2.3 Channels and backpressure

- All cross-thread channels are **bounded**. Unbounded queues are banned: they
  turn a slow consumer into an OOM.
- Inbound: if the inbound queue is saturated, apply network-level backpressure
  (stop reading from the socket) rather than dropping game actions.
- Outbound: a full outbound queue means the client is slow. Apply flow control
  per connection — throttle game-driven sends (chunk streaming, entity tracking)
  for that player; never block the sim thread.
- A connection that cannot keep up within a timeout is disconnected with the
  vanilla "Took too long to log in"/timeout semantics.

### 2.4 Determinism rules

- The sim thread is the only source of ordering for game events.
- All gameplay randomness uses a **vanilla-faithful PRNG** whose sequence is
  derived from world seed exactly as Beta 1.7.3 does. Do not substitute
  `rand::thread_rng` for gameplay.
- Parallel world generation must be **per-chunk deterministic**: a chunk's output
  depends only on `(seed, chunk coordinate)`, never on scheduling order.
  Generator state that is order-dependent must be seeded per chunk.
- Iteration over `HashMap`/`HashSet` must never influence game outcomes; use
  deterministic containers (sorted/`IndexMap`/arrays) where order matters.

---

## 3. Tick loop

The server runs at a fixed **20 TPS (50 ms per tick)** using an accumulator,
mirroring Betrock++'s Minestom-style loop.

```
loop {
    now = instant()
    accumulator += now - last
    last = now
    // catch-up protection: never simulate more than N ticks of backlog
    if accumulator > MAX_CATCHUP * 50ms { accumulator = MAX_CATCHUP * 50ms; warn() }
    while accumulator >= 50ms {
        tick(ticks++);
        accumulator -= 50ms;
    }
    sleep(until next tick boundary)   // sliced so shutdown is observed promptly
}
```

Per-tick phases, in order:

1. **Drain inbound** — process decoded packets from the inbound channel
   (bounded work per tick; excess is deferred to the next tick).
2. **Apply scheduled tasks** — plugin/engine tasks due this tick.
3. **Per world:** scheduled block ticks, random block ticks, redstone/neighbor
   update batches, weather, time, lighting propagation.
4. **Per world:** entity ticks (movement, physics, AI, block interactions).
5. **Per world:** mob spawning / despawning.
6. **Entity tracking** — recompute visibility, emit spawn/update/despawn packets.
7. **Inventory sync** — diff each open container against its snapshot.
8. **Integrate ready generation/load results** — chunks finished by worker pools.
9. **Emit outbound** — push queued packets to connection write tasks.
10. **Bookkeeping** — autosave cadence, TPS sampling, watchdog reset, stats.

`MAX_CATCHUP` is small (order of a few ticks) so a stalled tick cannot cause a
death spiral; the loop reports and moves on rather than simulating minutes of
missed time at once.

---

## 4. Module map

```
src/
  main.rs         Entry point: parse args, load config, install signal handling,
                  construct Server, run, handle graceful shutdown.
  server.rs       Server struct: owns worlds, players, scheduler, tick loop.
  config.rs       Config structs, defaults, schema version + migration.
  net/
    listener.rs   Tokio accept loop; per-IP connection throttle; spawns connections.
    connection.rs Per-connection state machine and read/write tasks (see §5).
    codec.rs      Framing (varint-free, fixed protocol-14 sizes), BE primitives,
                  UTF-16BE strings, bounded read buffers.
    packets/      One module per packet family: typed structs + encode/decode.
    protocol.rs   PacketId enum, protocol version constant, capability predicates.
  world/
    chunk.rs      Sub-chunk fill-layer storage; height map, biomes, payload packing.
    biome.rs      Climate octaves, 4096-entry classification table, temperature/rain.
    caves.rs      Deterministic 3D cave carving (MapGenCaves).
    decorate.rs   GenWorld trait; lakes, dungeons, ores, trees, flowers, snow.
    generator.rs  Terrain density field, surface replacement, populate orchestration.
    storage.rs    Region file (.mcr) reader/writer; NBT encode/decode.
    lighting.rs   Sky/block light propagation (queue-based).
    ticks.rs      Scheduled block ticks + random ticks.
    world.rs      World aggregate: chunk store, generation pool, packet streaming.
  entity/
    store.rs      Arena of entities with generational IDs (see §6).
    player.rs     Player entity: connection handle, inventory, view state.
    mob.rs        Mob entities, AI, spawning rules.
    tracker.rs    View-distance visibility; spawn/update/despawn packet planning.
  gameplay/
    blocks.rs     Block property table + behavior dispatch.
    items.rs      Item property/behavior tables, tool/durability data.
    inventory.rs  Inventories, containers, slot operations.
    crafting.rs   Shaped/shapeless recipe matching.
    tile.rs       Tile entities (chest, furnace, sign, spawner, ...).
    physics.rs    Movement, collisions, projectiles, falling blocks.
  plugin/
    abi.rs        C ABI types (stable, versioned) for native plugins.
    native.rs     Dynamic library loading (libloading), lifecycle.
    wasm.rs       Sandboxed WASM host (wasmtime), capability limits.
    host.rs       Host API implementation + event dispatch from the sim thread.
  util/
    math.rs       Vanilla-faithful math (trig tables, floor/mod semantics).
    rng.rs        Vanilla PRNG.
    bytes.rs      Zero-copy read/write helpers.
```

Each module has exactly one responsibility. Cross-module access goes through
public types, never by reaching into another module's internals.

---

## 5. Networking

### 5.1 Connection state machine

```
Handshaking ─▶ LoggingIn ─▶ VerifyingUsername ─▶ WaitingForSpawnChunks ─▶ Playing
```

- `Handshaking`: expect `Packet2Handshake`.
- `LoggingIn`: expect `Packet1Login`; validate protocol **14**, username rules,
  bans/whitelist, duplicate names; set up online-mode auth if enabled.
- `VerifyingUsername`: async session check (off sim thread); result delivered
  back to the sim thread.
- `WaitingForSpawnChunks`: send spawn-area chunks before admitting the player to
  gameplay; buffer/queue inbound gameplay packets.
- `Playing`: normal packet handling.

The phases match Poseidon's login flow (handshake → login → proxy handling →
async verification → player creation) but run without blocking the sim thread.

### 5.2 Read path

- Each connection has a read task that owns the read buffer for that socket.
- Framing is protocol-14 fixed-size per packet id; there is no length-prefix
  version negotiation. The codec reads exactly the bytes a packet needs and
  leaves the rest buffered.
- **Partial reads are rolled back**, not consumed: if a packet is incomplete, the
  buffer cursor is restored and the connection waits for more data. (Pattern
  from Betrock++ `NetworkStream::Mark/Rollback`.)
- Per-tick packet budget and buffer caps (e.g. ~1 MiB read cap, packet-rate
  limit) protect against spam/DoS, matching Poseidon/vanilla hardening.
- Decoded packets are pushed into the inbound channel with the connection id.

### 5.3 Write path

- Outbound packets are owned messages in a per-connection bounded queue.
- The write task serializes and writes them, using buffered, non-blocking writes.
- **Compression is decoupled:** chunk packets whose payload is expensive are
  pre-compressed on the compression pool before being queued for writing, so
  neither the sim thread nor the write task stalls on deflate.
- `TCP_NODELAY` is on by default for latency (configurable, as in Poseidon).

### 5.4 Packet serialization

- Packets are typed structs with explicit `encode`/`decode`.
- Big-endian primitives; strings are `i16 length + UTF-16BE`, bounded to the
  vanilla maximum.
- Encoders are **allocation-light**: they write into a reused per-connection
  buffer, not fresh `Vec`s per packet.
- A `PacketId` enum is the single source of truth for ids; a unit test asserts the
  enum covers exactly the protocol-14 set (no gaps, no extras).
- For broadcast packets (same content, many recipients), serialize once into a
  shared immutable buffer and hand the same buffer to every recipient, instead of
  re-encoding per player.

### 5.5 Capabilities and future versioning

Following UberBukkit's pattern, version-dependent decisions are expressed as
**capability predicates** rather than scattered `if version ==` checks. With a
single target version this is mostly a single constant, but the seam exists so
that adding another protocol later does not require surgery across the codebase.

---

## 6. Entity storage

Entities use a **data-oriented arena** with generational IDs, not `Rc<RefCell>`
graphs (a common Rust mistake that adds refcount overhead and runtime panics).

- `EntityId = (index: u32, generation: u32)`. Removing an entity bumps its
  generation, invalidating stale handles immediately.
- Storage is an `ElementStore` with per-component arrays (SoA) for hot fields
  (position, velocity, rotation, flags) and a side table for cold/variable data.
- Lookups are O(1); iteration is cache-friendly.
- **No entity reference crosses a tick boundary.** Cross-entity links
  (passenger/vehicle, targets) are stored as `EntityId`, resolved on demand.
- Spatial queries use a chunk/section bucket index (per-chunk vertical buckets),
  matching Betrock++'s `EntityManager`.

This replaces Java's object graph while preserving Beta semantics.

---

## 7. Worlds, chunks and generation

### 7.1 Chunk representation

Chunks are the biggest memory consumer, so they use **palette sub-chunks with
lazy fill layers** (Betrock++, `subchunk.h`):

- A chunk is 16×16×128, split into 8 sections of 16×16×16.
- Each section has four layers: block id, metadata, block light, sky light.
- A layer that is uniform costs one fill value instead of a full array; setters
  materialize the array only when a value differs from the fill.
- Nibble layers (metadata, light) are packed 4 bytes per 8 blocks.
- Per-section block counts enable O(1) "does this chunk contain block X" checks
  used by spawning and lighting.

Block id layer uses a **palette** so that chunks dominated by a few block types
store a small palette plus packed indices.

### 7.2 World tick

Each world owns its chunk store, entity list and block-update queues, and runs:

1. scheduled tile/block ticks (priority queue, deduplicated, capped per tick);
2. random block ticks per chunk near players;
3. deferred neighbor/redstone updates in batched passes (with a re-entrancy
   guard to prevent update storms);
4. lighting propagation queue;
5. weather/time;
6. entity ticks;
7. mob spawning.

This mirrors Poseidon's `World.doTick` split into deterministic, testable steps.

### 7.3 Storage

- Region files use the Beta `.mcr` format: 32×32 chunks, 4 KiB sectors,
  1024-entry header, zlib/GZIP payloads (Sector → header → NBT `Level`).
- NBT read/write is handled on the region I/O pool; the sim thread only sees
  decoded chunk data.
- `level.dat`, `session.lock` and player `.dat` files are managed in the same
  layer.
- An 8-slot (configurable) region-file cache bounds open file descriptors.

### 7.4 Generation pipeline

```
sim: mark chunk needed
  └─▶ gen pool: generate terrain (deterministic per chunk)
        └─▶ sim: integrate chunk, enqueue population
  └─▶ gen pool (or sim): populate (biomes, caves, ores, structures)
        └─▶ sim: finalize chunk, make visible
```

- Generation runs on Rayon with `thread_local` generator instances rebuilt only
  when the world seed changes (avoids reallocating noise permutation tables).
- A **per-player per-tick budget** (e.g. ~16 chunks) bounds generation work so it
  cannot starve the tick; fair-share cursors prevent one player from monopolizing
  the generator.
- A chunk produced by a worker is integrated only on the sim thread. Writes that
  arrived while generation was in flight are replayed onto the finished chunk
  ("bleed writes"), preserving correctness.

---

## 8. Entity tracking and visibility

- Tracking is driven by an **inverted index** `chunk → set of players` so that a
  block/entity change touches only interested connections.
- Per entity type, a tracking profile sets view range, update interval, and
  thresholds (position/rotation deltas), matching Poseidon's `EntityTracker`.
- Movement packets are chosen by delta: relative move, look, move+look, or
  teleport, with periodic forced resyncs.
- Chunk streaming uses the vanilla `Packet50PreChunk` + `Packet51MapChunk` flow,
  queued per player and throttled by how many chunk packets that player already
  has in flight (so slow clients do not force the server to buffer unboundedly).

---

## 9. Gameplay data

Block and item behavior follows Betrock++'s **table-of-behaviors** approach
instead of per-block virtual dispatch:

- A `BlockProperties` table indexed by block id holds material, hardness,
  resistance, light emission/opacity, step sound and boolean traits.
- A `BlockBehavior` table of function pointers holds hooks (`on_tick`,
  `on_neighbor_change`, `on_activated`, `on_placed`, `on_destroyed`, drops,
  collision shapes).
- Implementations are grouped by domain (fluids, rails, redstone, plants, doors,
  containers, falling blocks, portals, fire, ...).
- Items, tools and recipes follow the same pattern.

This is cache-friendly, branch-predictable, and avoids `dyn Trait` overhead in
hot paths. Beta-faithful constants (hardness, timings, drop tables) are copied
from the Poseidon source and covered by parity tests.

---

## 10. Plugins

Two backends, one host API, dispatched **only on the sim thread**:

- **Native** — Rust dynamic libraries exposing a stable `extern "C"` ABI
  (`plugin/abi.rs`), loaded with `libloading`. Fastest, fully trusted.
- **WASM** — sandboxed modules run by `wasmtime` with explicit capability limits
  (memory, fuel, time). For untrusted or portable plugins.

The host API is capability-based: plugins declare what they touch (world read,
block write, player data, commands), and events are delivered from the tick
loop. Event dispatch is bounded per tick so a misbehaving plugin cannot stall
the server; slow plugins are profiled and reported, mirroring Poseidon's slow
listener warnings.

The ABI is **versioned**; breaking changes bump the version and older plugins are
rejected with a clear error rather than crashing.

---

## 11. Invariants (must never be violated)

1. **Single writer.** Only the sim thread mutates game state. Network tasks and
   pools exchange owned messages over channels.
2. **No blocking on the sim thread.** File I/O, compression, network, and auth
   are always off-thread.
3. **No unbounded queues.** Every cross-thread channel is bounded and applies
   backpressure.
4. **Determinism.** Game RNG comes from the vanilla PRNG; generation is
   per-chunk deterministic; container iteration order never affects outcomes.
5. **Protocol 14 byte-exact.** The client must never desync. Any deviation is a
   bug.
6. **Poseidon is authority.** Behavioral mismatches are resolved toward Poseidon.
7. **No `Rc`/`RefCell`/`unsafe` in game logic.** Use arenas, IDs and clear
   ownership. `unsafe` is permitted only at the OS/FFI boundary, documented and
   encapsulated.
8. **Chunks are worker-produced, sim-integrated.** Workers never mutate a chunk
   that is live in the world.
9. **Plugin ABI stability.** Host and plugins version-check before use.
10. **Failure isolation.** A panic in a connection task or worker fails that unit,
    not the process; the sim thread is protected by a watchdog.

---

## 12. Reference mapping

Where to look when porting behavior from the reference implementations
(paths relative to `.references/`):

| Concern | Poseidon (`Project-Poseidon/src/main/java/`) | Betrock++ (`BetrockPlusPlus/src/`) |
|---|---|---|
| Tick loop | `net/minecraft/server/MinecraftServer.java` (`run`, `h`) | `bpp_server/server.cpp` (`Run`, `Tick`) |
| Packets | `net/minecraft/server/Packet*.java`, `NetHandler.java` | `bpp_shared/networking/packets.h` |
| Socket layer | `net/minecraft/server/NetworkManager.java` | `bpp_server/server_socket.h`, `network_stream.h` |
| Login flow | `NetLoginHandler.java`, `com/projectposeidon/.../LoginProcessHandler.java` | `bpp_server/player_conn/server_pconnstate_manager.cpp` |
| Chunks | `net/minecraft/server/Chunk.java` | `bpp_shared/world/chunk.h`, `subchunk.h` |
| Region storage | `RegionFile.java`, `ChunkRegionLoader.java` | `bpp_shared/world/storage/region.h` |
| Terrain gen | `ChunkProviderGenerate.java`, `NoiseGeneratorOctaves.java` | `bpp_shared/world/generator/overworld/chunk_gen.h` |
| Entities | `Entity.java`, `EntityTracker.java` | `bpp_shared/entities/`, `bpp_server/trackers/` |
| Blocks/items | `net/minecraft/server/Block*.java`, `Item*.java` | `bpp_shared/blocks/`, `bpp_shared/items/` |
| Plugins | `org/bukkit/plugin/`, `CraftScheduler.java` | `include/addon_api.h`, `bpp_server/addon/` |

Use this table to find the exact Java/C++ behavior to reproduce; then reproduce
that behavior behind the Rust module boundaries defined above.
