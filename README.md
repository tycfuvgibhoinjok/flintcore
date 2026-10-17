# flintcore

A high-performance Minecraft Beta 1.7.3 server core written in Rust.

`flintcore` reimplements the CraftBukkit `CB1060` (Beta 1.7.3, wire protocol **14**)
server from scratch, aiming to be a **drop-in replacement** for the standard
Beta 1.7.3 client while running dramatically faster and leaner than the original
Java implementation.

The project is deliberately small: **one binary crate, flat modules, no plugin
JVM, no data-driven registries that rot**. Performance comes from architecture
(data-oriented layout, async I/O, off-thread heavy work, zero-copy networking),
not from micro-tricks.

> Not an official Minecraft product. Not approved by or associated with Mojang or
> Microsoft. See [Legal](#legal-and-references).

---

## Goals

1. **Byte-exact protocol compatibility** with the standard Beta 1.7.3 client
   (protocol 14). A vanilla client must connect, play, and never desync.
2. **Behavioral parity** with Project Poseidon / CraftBukkit 1060 for gameplay
   semantics: world generation, block/entity behavior, inventory, crafting.
3. **Maximum performance** on commodity multi-core hardware: high player counts
   per node, stable 20 TPS under load, low memory and bandwidth.
4. **A clean, modern plugin API** — native Rust dynamic libraries and sandboxed
   WASM — instead of the legacy Java plugin model.
5. **A tiny, auditable core.** Every module has one clear responsibility.

## Non-goals

- Running existing Java/Bukkit plugins. Plugin compatibility is provided through
  a **new native + WASM API**, not an embedded JVM.
- Supporting post-Beta Minecraft versions or clients. Target is exactly Beta
  1.7.3 / protocol 14.
- Multi-version protocol negotiation in the core. (The architecture leaves room
  for it; it is not a goal.)
- Regionized multi-thread world simulation. The simulation is single-threaded
  by design (see [ARCHITECTURE.md](ARCHITECTURE.md)); parallelism is applied to
  I/O, generation, compression and serialization.

## Fixed technical decisions

These are constraints, not suggestions. They are referenced throughout the docs.

| Area | Decision |
|---|---|
| Async runtime | **Tokio** (multi-threaded), used for all socket I/O |
| Simulation | **Single deterministic sim thread** at a fixed 20 TPS |
| Heavy work | Off-thread pools: **Rayon** (worldgen), dedicated region-I/O and **libdeflate** compression pools |
| Inter-thread messaging | **Bounded** channels with explicit backpressure |
| Entity storage | Data-oriented **arena / generational IDs** (SoA where hot) |
| Chunk storage | **Sub-chunk palettes** with lazy fill layers |
| Compression | **libdeflate** (region files and chunk packets) |
| Plugins | Native **C ABI** (`libloading`) + sandboxed **WASM** (`wasmtime`) |
| Config | Single human-editable file, schema-versioned |

## Compatibility target

| Dimension | Target |
|---|---|
| Wire protocol | Beta 1.7.3, protocol **14** |
| Login / handshake | `Packet1Login` / `Packet2Handshake`, offline + online mode |
| Chunk format | `Packet50PreChunk` + `Packet51MapChunk` (zlib) |
| World storage | `.mcr` region files, NBT (`level.dat`, player `.dat`) |
| Semantics reference | Project Poseidon (`CB1060`) |
| Port reference | Betrock++ (C++ reimplementation) |

The **behavioral authority** is Project Poseidon. When this document, the code,
and Poseidon disagree, **Poseidon wins**, and the discrepancy is a bug in
`flintcore` (or in these docs) that must be fixed.

## Architecture summary

```
        ┌──────────────────────── Tokio multi-thread runtime ────────────────────────┐
        │  acceptor · per-connection read/decode · write/encode · timeouts · auth    │
        └───────────────▲───────────────────────────────────────────┬───────────────┘
                        │ decoded inbound packets                    │ encoded outbound
              bounded   │ (crossbeam)                                │ (bounded, per-conn)
                        │                                            ▼
        ┌───────────────┴────────────────────────────────────────────────────────────┐
        │                        SINGLE SIMULATION THREAD                            │
        │   fixed 20 TPS tick loop · worlds · entities · blocks · inventories ·       │
        │   event dispatch · plugin host                                              │
        └───────┬─────────────────────────┬──────────────────────────┬───────────────┘
                │ generation jobs         │ region I/O jobs          │ compression jobs
                ▼                         ▼                          ▼
        ┌───────────────┐        ┌──────────────────┐        ┌───────────────────────┐
        │ Rayon pool    │        │ Region I/O pool  │        │ Compression pool      │
        │ (worldgen)    │        │ (NBT / .mcr)     │        │ (libdeflate)          │
        └───────────────┘        └──────────────────┘        └───────────────────────┘
```

The simulation thread is the **only** owner of mutable game state. Network
threads and worker pools exchange data with it strictly through channels. This is
what makes the core both fast (parallel I/O and compute) and correct (no data
races in the game model).

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design.

## Project layout

```
flintcore/
  Cargo.toml
  README.md
  ARCHITECTURE.md      # concurrency model, tick loop, module map, invariants
  DEVELOPMENT.md       # coding standards, workflow, parity/testing rules
  src/
    main.rs            # config load, startup, shutdown
    server.rs          # orchestration + 20 TPS tick loop
    config.rs          # configuration schema + versioned migration
    net/               # listener, connection FSM, codec, packets, protocol
    world/             # palette chunks, region/NBT storage, generator, lighting
    entity/            # data-oriented arena, player/mob, tracking
    gameplay/          # blocks, items, inventory, crafting, tile entities
    plugin/            # native dylib ABI + WASM host
    util/              # shared math, RNG, byte helpers
```

## Quickstart

> The core is under active development; commands describe the intended workflow.

```
# build (release, LTO, native CPU tuning)
cargo build --release

# run
cargo run --release -- --config flintcore.toml

# tests (unit + integration + parity)
cargo test

# lint + format (required before every commit)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

A running server listens on `25565` by default and accepts an unmodified Beta
1.7.3 client.

## Documentation

| File | Contents |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Concurrency model, tick loop, modules, data structures, invariants |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Rust standards, workflow, testing, parity rules, anti-patterns |

These two documents are the source of truth for *how* the server is built.
Protocol constants and behavioral details live in code, not in prose, so that
they cannot drift out of sync.

## Legal and references

`flintcore` is an independent reimplementation. It reproduces the observable
behavior of Minecraft Beta 1.7.3 (protocol 14) for interoperability.

Reference implementations studied while building this project:

- **Project Poseidon** — `CB1060` fork; behavioral authority.
- **UberBukkit** — Poseidon fork; source of the protocol-capability and
  config-migration patterns.
- **Betrock++** — C++23 reimplementation; source of the threading, palette-chunk
  and off-thread-pipeline patterns.

CraftBukkit and Bukkit are licensed under GPLv3. Minecraft is copyright Mojang AB.
This project is not affiliated with Mojang or Microsoft.
