# Development guide

This document is the **contract for contributors** (human or AI). It exists to
prevent the two failure modes that kill projects like this: **architectural
drift** and **behavioral divergence** from the reference server.

Read [README.md](README.md) and [ARCHITECTURE.md](ARCHITECTURE.md) first. If a
rule here conflicts with [ARCHITECTURE.md](ARCHITECTURE.md), architecture wins;
fix this document.

---

## 1. Prerequisites

- A recent stable Rust toolchain (edition 2021+).
- A Beta 1.7.3 client for manual testing.
- The reference sources under `.references/` for behavior lookups.

## 2. Commands

```
cargo build --release          # build
cargo run --release            # run with ./flintcore.toml
cargo test                     # unit + integration + parity tests
cargo fmt                      # format (run before committing)
cargo clippy --all-targets --all-features -- -D warnings
```

All three of `fmt`, `clippy -D warnings`, and `test` must pass before a change is
considered complete. CI runs them.

---

## 3. Rust style

- **Formatting:** `cargo fmt`, no exceptions.
- **Lints:** `cargo clippy -- -D warnings` must be clean.
- **Comments:** The project default is **no comments**. Write code that explains
  itself through naming and structure. Add a comment only to record a *non-obvious
  reason*: a vanilla quirk, a wire-format subtlety, a safety invariant, or a
  performance trap. Never narrate what the code does. Reference the source of a
  quirk (e.g. `// b1.7.3 sends pre-chunk before map-chunk`), not the mechanics.
- **Naming:** `snake_case` items, `PascalCase` types, `SCREAMING_SNAKE` consts.
  Use domain names from the references (`net_server_handler`, `pre_chunk`,
  `map_chunk`) so the mapping stays obvious.
- **Errors:** Use `Result` with a project error type. `unwrap`/`expect` are only
  allowed for programmer invariants that are truly unreachable; prefer explicit
  handling. In the tick loop, an error must never panic the process.
- **Ownership:** Prefer plain owned values and arenas. Do not use `Rc`,
  `RefCell`, or shared mutability in game logic. `Arc` is only for genuinely
  shared immutable data crossing threads.
- **Async:** `async` is confined to `net/` and the plugin/auth boundaries. Never
  make simulation code `async`. Never block inside an async task; use the
  provided pools.
- **`unsafe`:** only at the OS/FFI boundary (sockets, `libloading`, `wasmtime`,
  memory-mapped I/O). Every `unsafe` block needs a `// SAFETY:` comment stating
  the invariant that makes it sound. Keep it in the smallest possible scope.

---

## 4. Module rules

- One responsibility per module (see the map in
  [ARCHITECTURE.md §4](ARCHITECTURE.md#4-module-map)).
- Depend inward: `net` and `plugin` depend on `world`/`entity`/`gameplay`, never
  the reverse. `world`, `entity` and `gameplay` must not reference Tokio.
- No reaching into another module's internals; go through its public API.
- New cross-cutting behavior (logging, metrics) is added once, in `util` or
  `server`, not copied per module.
- If you find yourself needing a new top-level module, stop and revisit
  [ARCHITECTURE.md](ARCHITECTURE.md) first.

---

## 5. Behavioral parity

Poseidon is the authority. The workflow for any gameplay feature:

1. **Find the source.** Use the reference table in
   [ARCHITECTURE.md §12](ARCHITECTURE.md#12-reference-mapping) to locate the
   exact Java class/method (and Betrock++ equivalent, if useful).
2. **Write down the observable behavior** — inputs, outputs, packet effects,
   edge cases, RNG use, rounding.
3. **Implement it behind the Rust module boundaries** defined in architecture.
   Do not copy Java structure into Rust; copy *behavior*.
4. **Prove it** with a test (see §6), ideally against a golden value or a
   differential run.

When in doubt, **do what Poseidon does**, even if it looks wrong. Compatibility
is the product; "improvements" that change observable behavior are bugs unless
explicitly gated behind config.

Known Beta quirks that must be preserved (non-exhaustive, add as discovered):

- Fixed packet sizes / no framing negotiation; partial reads are rolled back.
- `Packet51MapChunk` uses a specific `17 + data` size and zlib payload.
- Block metadata and lighting are nibble-packed.
- Vanilla PRNG sequence and iteration order affect worldgen and mob behavior.
- Entity movement quantization (`*32` position, `*8000` velocity, `*256/360`
  rotation) and the thresholds that select which movement packet is sent.
- Login flow, including the login packet's abused `dimension` byte used by
  proxies.

World-generation quirks found while reaching block-for-block Overworld parity
with Project-Poseidon (verification harness in §6):

- `Block.o` (`opaqueCubeLookup`) decides whether tree leaves replace a cell
  and whether a mushroom can sit on it. It is *not* "solidity": `LEAVES` and
  `CHEST` are opaque; `STEP`/`DOUBLE_STEP`, `GLASS`, `BED`, `CACTUS`, `ICE`,
  `SNOW`, fences etc. are not (`blocks::is_opaque_cube`).
- Tree leaf writes skip cells with `Block.o[id]` set, so trees never
  overwrite leaves. `WorldGenTrees`, `WorldGenTaiga1` and `WorldGenTaiga2`
  also honour the configured `block-tree-growth` blacklist (`poseidon.yml`,
  default `54,63,68`) — `Taiga1`/`Taiga2` query it at quirky coordinates
  (the ground block id is used as x, loop variables as z/y) and
  `WorldGenForest` never consults it. Preserve all three behaviours.
- Decoration consumes `JavaRandom` in exact order, including draws whose
  result is invisible (leaf-blob "corner" coin flips happen before the
  replaceability check).
- Fluid instant-tick cascades run to completion inside `World.a`/`World.c`;
  `BlockFlowing`'s per-side flags live on a singleton and are clobbered by
  nested ticks, so the flow tail must re-read them every iteration
  (`World::fluid_sides`).
- `BlockFace` in this build: NORTH=(-X), EAST=(-Z), SOUTH=(+X), WEST=(+Z).
- The snow pass writes flow through the physics path (`set_block_physics`).

---

## 6. Testing

Layers, from cheapest to strongest:

1. **Unit tests** — pure functions: codec round-trips, NBT round-trips, palette
   set/get/edges, recipe matching, RNG vectors, math semantics.
2. **Packet round-trip tests** — every packet encodes and decodes to the same
   value and to the exact expected bytes for known fixtures.
3. **Determinism tests** — generating a chunk twice (or in parallel with a
   shuffled schedule) yields identical output for the same seed.
4. **Integration tests** — spin the server in-process, connect a scripted
   protocol-14 client, and assert the login → spawn → movement → interaction
   sequence.
5. **Differential/parity tests** — run the same scenario against Poseidon and
   `flintcore`, compare observable output (chunks, packets, entity states).
6. **Replay tests** — record a session's inbound packets and assert the recorded
   outbound packets reproduce on replay. This catches regressions broadly.

Rules:

- A gameplay change without a test is incomplete.
- New block/item behaviors need at least a drop/placement/use test.
- New worldgen features need a fixed-seed golden-chunk comparison.
- Never weaken or delete an existing parity test to make a change pass. If a test
  is wrong, fix it deliberately and explain why in the commit.

Worldgen parity harness (Overworld):

- Ground truth: `tools/poseidon_dump/PoseidonGenerator.java` runs the
  reference server headless and writes `chunk_<seed>_<cx>_<cz>.bin` per chunk
  (32768 block bytes, 16384 nibble-packed meta bytes, 256 height bytes).
  Compile it into the reference class directory:
  `javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" -d target/poseidon_classes tools/poseidon_dump/PoseidonGenerator.java`
  then run it from the repo root (it reads `./poseidon.yml`):
  `java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" flintcore.test.PoseidonGenerator <seed> /tmp/poseidon_dumps -1 1 -1 1`
  (prerequisite: the reference Poseidon sources compiled once into
  `target/poseidon_classes`).
- `tests/poseidon_compare.rs` regenerates the same 3x3 populated region and
  compares block/meta/height cell by cell. `POSEIDON_DUMPS` selects the dump
  dir, `POSEIDON_SEEDS` a comma-separated seed list (the defaults cover 18
  seeds including tundra/taiga/desert/forest regions).
- `tests/poseidon_golden.rs` is the JVM-free regression pin: it rebuilds each
  chunk's full dump image and compares an FNV-1a 64 hash against the
  committed `tests/golden/poseidon_region_hashes.txt` (derived from Java
  dumps; regeneration command in the file header). Regenerate from fresh Java
  dumps only when behavior intentionally changes — never hand-edit hashes.
- `tests/debug_cube.rs` prints raw id/meta cubes (`DEBUG_SEED`, window envs).
  With `FLINTCORE_TRACE=<x0,y0,z0,x1,y1,z1>` it emits a write-level event
  trace (`A4`/`A5`/`B`/`G`/... lines) that can be diffed against a patched
  reference run for byte-level staged comparison.

---

## 7. Performance discipline

- Measure before optimizing. Use a profiler; do not guess.
- Performance work must not change observable behavior. If it must, gate it.
- Respect the invariants in [ARCHITECTURE.md §11](ARCHITECTURE.md#11-invariants-must-never-be-violated):
  no unbounded queues, no blocking on the sim thread, no shared mutability.
- Hot paths (tick, encode, chunk access, entity iteration): avoid allocation,
  avoid `dyn`, prefer arrays/tables and reused buffers.
- When you add a benchmark, commit the numbers and the method so others can
  reproduce them.

---

## 8. Anti-patterns (explicitly forbidden)

- Touching `World`/`EntityStore` from a Tokio task or worker pool.
- `Rc`/`RefCell`/interior mutability in game logic.
- `unwrap()` on network input, file I/O, or anything a client can influence.
- Unbounded channels, `Vec` growth without a cap on client-driven sizes.
- `rand` for gameplay randomness; must use the vanilla-faithful PRNG.
- Using `HashMap` iteration order in a way that affects game outcomes.
- Substituting `std::thread::sleep` busy-waits or blocking calls in the tick.
- "Improving" vanilla behavior without a config gate and a test.
- Re-encoding the same broadcast packet per recipient when it can be shared.
- Editing the docs to match buggy code instead of fixing the code.

---

## 9. Workflow

- **Commits:** small, focused, one logical change. Message format:
  `area: imperative summary` (e.g. `net: roll back partial packet reads`).
  Reference the Poseidon class/method or issue when relevant.
- **Branches:** `feat/…`, `fix/…`, `perf/…`, `docs/…`, `refactor/…`.
- **Do not** reformat unrelated code in a change; keep diffs reviewable.
- **Do not** commit generated files, build artifacts, or local config.
- **Do not** commit secrets or server credentials.
- **Docs:** update [ARCHITECTURE.md](ARCHITECTURE.md) when you change an
  invariant, a module boundary, or the concurrency model. Documentation changes
  ride with the code change, not in a later "docs" commit.

---

## 10. Review checklist

Before requesting review, confirm:

- [ ] `cargo fmt` clean; `cargo clippy -D warnings` clean; `cargo test` green.
- [ ] Change respects module boundaries and connecting only through public APIs.
- [ ] No game state touched off the sim thread; no blocking on the sim thread.
- [ ] All new channels bounded; client-controlled sizes capped.
- [ ] Behavior matches Poseidon; quirks preserved; config-gated if changed.
- [ ] Tests added/updated (unit + parity where applicable).
- [ ] No forbidden anti-patterns from §8.
- [ ] Comments only where a non-obvious reason must be recorded; otherwise none.
- [ ] Docs updated if an invariant/boundary/concurrency detail changed.

---

## 11. Getting unstuck

- Can't tell what the correct behavior is? **Look it up in Poseidon**, then
  Betrock++, then the Beta protocol/wiki references. Do not invent behavior.
- Design question about threading or data layout? Re-read
  [ARCHITECTURE.md](ARCHITECTURE.md); if the answer isn't there, that's a signal
  the architecture doc needs an addition — make it deliberately.
- Tempted to add a crate, a trait-object layer, or an ECS framework? Don't. The
  core is meant to stay small; solve it with the existing modules and plain data.
