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

Mob movement parity harness:

- Ground truth: `tools/poseidon_mobs/PoseidonMobTrace.java` runs a single mob
  of the reference in a handcrafted world and dumps its position, velocity,
  rotation, ground flag, fall distance, target and path after every tick.
  Build and run it as described in `tools/poseidon_mobs/README.md`.
- The traces are the golden file `tests/golden/poseidon_mob_traces.txt`; the
  test `the_mob_movement_matches_the_poseidon_reference` in `src/server.rs`
  reproduces the sixteen scenarios and compares them tick by tick. The test is
  `#[ignore]`d (it is slow), the test
  `the_mob_scenarios_of_the_reference_match` compares the same sixteen
  scenarios and runs in the normal test suite.
- The port follows the reference classes one by one: `AxisAlignedBB` and
  `Entity.move` (`src/entity/physics.rs`), `EntityLiving.a(float, float)`
  (the acceleration, the drag and the jump), `Pathfinder`/`Path` (the A* with
  the tie breaking of the reference heap) and `EntityCreature.c_()` (the
  target, the path following and the random look around).
- The reference shares **one** `Random` between all entities
  (`Entity.SHARED_RANDOM`); the flintcore uses the same model, seeded from the
  seed of the world, so the behaviour is reproducible (the reference takes the
  seed from the system clock, which is a documented deviation).
- The movement of the mobs is the most expensive part of the tick: the
  benchmark `the_mob_tick_is_fast_enough` (100 mobs on a flat floor, 100
  ticks) measures **0.4 ms per tick** in the release build and 3.6 ms in the
  debug build.
- The state of the port (the divergence of the trace against the reference,
  the comparison lives in `the_mob_movement_matches_the_poseidon_reference`
  (it is `#[ignore]`d, it is slow) and in
  `the_mob_scenarios_of_the_reference_match` (the scenarios that must not
  diverge), the golden traces live in
  `tests/golden/poseidon_mob_traces.txt`):

  | scenario  | the trace matches for | the difference |
  |-----------|-----------------------|----------------|
  | `slime`   | **300 ticks (all)**   | - |
  | `flat`    | **300 ticks (all)**   | - |
  | `ghast`   | **300 ticks (all)**   | - |
  | `chase`   | **300 ticks (all)**   | - |
  | `wall`    | **300 ticks (all)**   | - |
  | `squid`   | **300 ticks (all)**   | - |
  | `water`   | **300 ticks (all)**   | - |
  | `lava`    | **300 ticks (all)**   | - |
  | `chicken` | **300 ticks (all)**   | - |
  | `cow`     | **300 ticks (all)**   | - |
  | `sheep`   | **300 ticks (all)**   | - |
  | `wolf`    | **300 ticks (all)**   | - |
  | `pigzombie` | **300 ticks (all)** | - |
  | `giant`   | **300 ticks (all)**   | - |
  | `creeper` | **300 ticks (all)**   | - |
  | `skeleton`| **300 ticks (all)**   | - |

  The harness has the scenarios of every mob of the reference: the pig, the
  zombie, the spider, the slime, the squid, the ghast, the creeper, the
  skeleton, the chicken, the cow, the sheep, the wolf, the pig zombie and the
  giant zombie.

  - The random stream of the entities is identical to the reference: the
    numbers are drawn in the same order and with the same values (the
    comparison of the traces with `-Dflint.trace.rng=1` shows it tick by
    tick). The reference draws one number per tick in `EntityLiving.R()`
    (the idle sound of the mob), the number of the walk of
    `EntityCreature.c_()` and the two numbers of the base
    `EntityLiving.c_()`. The sound of the mob (`EntityLiving.Q()`) and the
    sound of the damage (`EntityLiving.damageEntity`) draw two numbers
    more.
  - The order of the draws is the order of the reference: the splash of the
    water (`Entity.m_()`), the damage of the fire and of the lava
    (`Entity.R()`), the idle sound (`EntityLiving.R()`), the counters, the
    target, the path to the target, the walk of the mob, the AI of the mob
    and the movement.
  - The arithmetic of the reference is ported exactly: the constants of
    the single precision (`0.9800000190734863D`, `0.41999998688697815D`),
    the sine and the cosine of the table of `MathHelper`, the square root
    of `MathHelper.a()` (the double precision, the single precision one
    can differ in the last bit) and the arcus tangens of
    `org.bukkit.craftbukkit.TrigMath` (`util::math::trig`).
  - The reference uses **two** arcus tangens: the one of
    `org.bukkit.craftbukkit.TrigMath` (the walk of the mobs) and the one of
    the JVM (`Math.atan2`, the look at the target, the flight of the ghast
    and the swim of the squid). The JVM implements it with the `fdlibm`
    library, whose result differs from the one of the system library in the
    last bit, so the port has its own copy (`util::fdlibm`), checked
    against the JVM bit by bit (`util::fdlibm::tests`).
  - The interval of the idle sound of a mob is the one of the reference:
    80 ticks for the mobs and 120 ticks for the animals and the water
    animals (`EntityLiving.e()`, `EntityAnimal.e()` and
    `EntityWaterAnimal.e()`). The counter of the sound is reset with it,
    so a wrong interval shifts the whole stream of the random numbers.
  - The check of the daylight of the zombies and the skeletons
    (`EntityZombie.v()`) has three conditions in the reference: the world
    is bright enough, the light of the block of the mob is above 0.5 and
    the block of the mob is above the terrain (`World.isChunkLoaded`).
    Only the last one draws a number, so the port has to check it too.
  - The light of the scenario is set by hand (the harness and the port set
    the sky light of the floor to 15): the world of the reference is
    generated with the normal generator, so the light of the hills around
    the floor would be an artefact of the terrain. The harness writes the
    light of the whole column (`y > floorY`), the port does the same.
  - The counter of the ticks without a target (`EntityLiving.ay`) grows by
    one in `EntityLiving.c_()` and by two in the light
    (`EntityMonster.v()`), the despawn of the mob (`EntityLiving.U()`) draws
    a number when the counter passes 600. The port has the same counter, so
    the number is drawn in the same tick.
  - The corpse of a dead mob lives twenty ticks (`EntityLiving.R()`), the
    mob has no artificial intelligence while it is dead
    (`EntityLiving.v()`: the input of the movement is zero) and it leaves the
    world when the counter of the death passes twenty. The harness of the
    reference prints the frozen state of the mob after that, the port does
    the same.
  - The flag `EntityLiving.e` ("the mob sees its target") is reset at the
    beginning of the artificial intelligence (`EntityCreature.c_()`) and the
    attack of the creeper and of the skeleton sets it. It has two effects: the
    path of the mob is not computed again while the flag is set, and the mob
    that follows a path looks straight at its target and walks sideways
    (`EntityCreature.c_()`: the yaw is the direction of the target and the
    inputs of the movement are the projection of the forward input on the new
    yaw). The attack of the mob happens **before** the path of the mob, so
    the order of the draws of the random numbers depends on it.
  - The arrow of the skeleton draws six normal values: three in the
    constructor of the arrow and three in its direction (`EntityArrow.a`).
    The values of the constructor are overwritten by the direction, but they
    are drawn, so they shift the stream of the random numbers.
  - The explosion of the creeper happens inside its attack, before the
    movement of the mob (`EntityCreeper.a`): the mob falls into the hole of
    the explosion in the same tick. The source of the explosion is not
    damaged by it (`World.b(Entity, AxisAlignedBB)` skips the source) and
    the creeper dies without a corpse: the world removes it at the beginning
    of its next tick (`World.tickEntities`).
  - The target of a mob is searched only when the mob has no target
    (`EntityCreature.c_()`): a target that was found in this tick is not
    attacked yet, and a target that died is cleared without searching a new
    one. The line of sight of the reference is the ray from the eyes of the
    mob to the eyes of the target (`EntityLiving.e(Entity)`).
  - The pig zombie is angry until it forgets its target, the anger does not
    run out, and the angry pig zombie has a sound of its own
    (`EntityPigZombie.m_()`), the counter of the sound is set when the pig
    zombie is hurt.
  - The sheep has ten points of health (`EntityLiving`), the wolf has eight
    (`EntityWolf`), and the giant has two hundred (`EntityGiantZombie`). The
    chicken and the ghast do not take the damage of a fall
    (`EntityChicken.a(float)` and `EntityFlying.a(float)`).
  - The death of a mob draws the number of the items **always**
    (`EntityLiving.q()` draws `nextInt(3)` even when the mob drops
    nothing), and the children of a slime are created when the corpse
    leaves the world (`EntitySlime.die()` is called by `EntityLiving.R()`),
    not at the moment of the death. Every child draws the size and the
    delay of its constructor and its own yaw.
  - The natural spawn draws the yaw of the mob and the colour of a sheep
    from the stream of the **world** (`SpawnerCreature`), not from the
    stream of the entities. The skeleton that rides a spider inherits the
    yaw of the spider.
  - The explosion of the creeper drops the blocks always
    (`Block.dropNaturally`: the yield of the explosion is one) and the
    fire of the explosion is set **before** the destruction of the blocks
    (`Explosion.a()`), with a random of its own. The reference checks the
    air of the block before the destruction, so the fire never starts:
    the port copies that behaviour.
  - The blocks that tick randomly are the ones whose flag `Block.n[]` is
    set (`World.java:1978-1988`). The flag is set by `Block.a(boolean)` in
    the constructor of the block, so the table of the reference is the
    list of the blocks whose constructor calls it. The classes of the
    compiled reference gave the list
    (`tools/poseidon_mobs/BlockTickTable.java` prints it), and the port
    ticks exactly those blocks.
  - The sets of the chunks of the natural spawn and of the random ticks
    are **sorted**: the order of the chunks decides the draws of the
    random of the world, and the order of a hash table of Rust is random
    for every process, so the same world and the same seed gave different
    worlds from one run to the next.
  - The mobs push each other away at the end of their update
    (`Entity.collide`): the distance is the biggest of the absolute
    differences (`MathHelper.a(d0, d1)` is not the Euclidean distance)
    and the square root of it is rounded to a single precision value.
    The players are pushed too.
  - The mob swims up when it is stuck only when the box it would move
    to is free of the entities and of the liquids (`Entity.d`).
  - The mob suffocates when one of the eight probes around its head holds
    a solid block (`Entity.K()`); the leaves, the ice, the cactus, the
    layer of the snow and the TNT are not solid.
  - The damage of a mob is applied only when it is not in the window of
    the invulnerability: the packet of the status 2 and the knockback of
    the attacker happen only then, and the two numbers of the stream are
    the pitch and the volume of the sound of the hit or of the death.
  - The arrow of the skeleton starts one block above the mob and flies at
    the height of the eyes of the mob minus 0.2 (`EntitySkeleton.a`).
  - The attack of the slime is only made when the player is visible and
    it draws two numbers for the tone of its sound (`EntitySlime.b`); the
    sound itself is not sent to the client (see above).
  - The named sounds of the entities never reach the client: the method
    of the world that sends them in the reference has an empty body
    (`WorldManager.a(String, double, double, double, float, float)`) and
    the packet 41 is not even registered in `Packet.java`. The port
    follows the reference: the sounds are not sent, but the draws of the
    random of the reference (the tone and the volume) are made all the
    same, because they are a part of the stream of the world. The only
    sound that reaches the client is the Play Sound by its number
    (`Packet61`).
  - The counter of the steps of the mob is raised when the distance the
    mob walked passes it (`Entity.move()`); the sound of the step itself
    is not sent to the client (see above).
  - The flow of the water pushes the mob: the flows of the blocks of the
    liquid in the box of the mob are summed, the sum is normalised and
    the mob is pushed by 0.014 of it (`World.a`).
  - The mob in a web is slowed down: the flag of the web is set at the
    end of the movement and the movement of the next tick is damped
    (`Entity.move()`).
  - The tables of the blocks are the tables of the reference: the opaque
    cube, the opacity of the light, the solid block, the mobility for the
    pistons, the resistance to the explosion and the flag of the
    notification of the data. The tools of `tools/poseidon_mobs/` print
    them, the tables are in `tests/golden/` and the test of the tables
    compares them with the port.
  - The time of the world is sent to the players every twenty ticks
    (`MinecraftServer.tick`).
  - The light of the sky is reduced by the time of the day
    (`World.a(i, j, k, true)`), the light of the blocks is not.
  - The light of the sky is dimmed by the rain and by the thunder: the
    factor is multiplied by `1 - intensity * 5 / 16` for each of them, the
    intensities rise by one hundredth every tick while the weather lasts
    (`World.a(float)` and `World.g()`).
  - The explosion of the reference: the ray is not interrupted at the
    borders of the world, the blocks of the explosion are destroyed in the
    order of the iteration of the hash set of the reference, the entity is
    thrown away from the centre of the explosion, and the exposure of the
    entity is the share of the points of its box from which the centre is
    visible (`World.a(Vec3D, AxisAlignedBB)`).
  - The blocks with a tile entity create it when they are written
    (`BlockContainer.c`), the tile entity of the old block is removed, and
    the direction of the furnace and of the dispenser is set from the
    neighbours (`BlockFurnace.g`).
  - The liquid that spreads drops the block it displaces and the lava
    makes the sound of the fizz (`BlockFlowing.flow`).
  - The fuse of the TNT destroyed by an explosion is drawn from the random
    of the world (`BlockTNT.d`).
  - The tags of the NBT keep the order of their insertion: the files of
    the world are the same byte for byte on every run.
  - The file `level.dat` is written when the server stops, by the command
    `save-all` and by the periodic saving of the world, not every five
    seconds.
  - The tracker of the entities follows the reference: every type of
    entity has its own interval of the update of the position (the player
    2, the animals 3, the boat and the minecart 5, the snowball, the egg,
    the fireball and the primed TNT 10, the arrow and the item 20, the
    painting never), the packet is sent at once when the entity was
    pushed, jumped or hurt, the radius of the tracking is a square in XZ
    and is limited by the distance of the view, the threshold of the
    rotation is one, and the tracker is updated every tick.
  - The world is saved every forty ticks: the data of the level, the data
    of the players and up to twenty four chunks. The keep-alive is not
    sent (the reference never sends it), the connection of the client is
    kept alive by the packet of the time.
  - The connection that received nothing for twelve hundred ticks is
    disconnected, and the player that sends more than a thousand packets
    in five seconds is disconnected with a message.
  - The answers of the commands go to the executor and to the operators,
    the command `say` goes to everybody; the texts of the commands are the
    texts of the reference.
  - The chat of the player is checked like in the reference: the length
    before the trimming, the characters of the file `font.txt` (the
    colour character is not in it) and no translation of the `&`.
  - The chunks are stored like in the reference: the file of the chunk is
    `<world>/<x & 63 in base 36>/<z & 63 in base 36>/c.<x>.<z>.dat` with
    the NBT compressed by gzip; the file is written as `tmp_chunk.dat`
    and then renamed. The worlds of the port that used the regions of
    Anvil are imported at the start of the server.
  - The file of the player holds the UUID of the world and of the player,
    the sleep of the player and the world of the spawn point; the file of
    the level holds the compound `Player` with the data of the first
    player of the world. The UUIDs are random in the reference, the port
    derives them from the seed of the world and from the name of the
    player, so that the files stay the same from run to run.
  - The player of the reference: any damage is reduced by the armour,
    the player that has just appeared cannot be hurt for sixty ticks, the
    player regains health when the monsters are switched off, the damage
    of the mobs and of the arrows is scaled by the difficulty, the stance
    is checked, the bed must be near, and the healing sets the window of
    the invulnerability.
  - The dropped item of the reference: it does not take damage in the
    lava, it is spread in the lava, it is pushed out of the block through
    the nearest free face and it bounces off the ground.
  - The primed TNT of the reference explodes on the eighty-first tick; the
    angle of the hit comes from its own stream, the reference draws it
    from `Math.random()` and not from the shared stream of the entities.
  - The painting of the reference is checked every hundred ticks, it
    breaks when any entity intersects its box and it drops the item at
    its own position.
  - The arrow of the reference checks the hit along the segment of the
    movement, the arrow that hit a block shakes for seven ticks and the
    fireball does no direct damage.
  - The entities are saved in the chunk: the list `Entities` holds them in
    the layout of the reference (the name of the type, the tags of
    `Entity.d()` and the tags of the type). The entities appear in the
    world when the chunk is loaded. The egg and the fireball are not saved
    — they are not in the table `EntityTypes`.
  - The trace is compared as text, so the port prints the numbers the way
    of Java: Java rounds half up, Rust rounds half to even and the same
    value can be printed with different digits.
  - The player of the reference harness is created without the constructor,
    so it has no box at all: the mob walks through it. The scenario of the
    port uses a player without a box too (`Player::box_size`).
  - The differences that are left are the parts of the reference that are
    not ported yet: the damage of the mobs from the other sources than the
    attacks, the items of the dead mobs and the packets of the death. The
    movement of the mobs, the fire, the lava, the corpse of the dead mob and
    the stream of the random numbers are the same as in the reference.

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
