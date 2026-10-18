# Reference-side event tracing (optional diagnostics)

Rust emits write-level event traces (see `tests/debug_cube.rs` and the
`FLINTCORE_TRACE` gating in `src/world/world.rs`). To diff them against the
real reference server, compile a scratch copy of the reference sources with
the instrumentation in `reference-trace.patch` + `Trace.java`.

## Build the instrumented reference

Run from the flintcore repo root (the reference classes must already be
compiled into `target/poseidon_classes`, see `DEVELOPMENT.md` §6):

```sh
REPO=$(pwd)

# 1. scratch tree
rm -rf /tmp/pos_patch_src && mkdir -p /tmp/pos_patch_src/net/minecraft/server /tmp/pos_patch_src/flintcore/test
cp .references/Project-Poseidon/src/main/java/net/minecraft/server/*.java /tmp/pos_patch_src/net/minecraft/server/
cp tools/poseidon_dump/PoseidonGenerator.java /tmp/pos_patch_src/flintcore/test/
cp tools/poseidon_trace/Trace.java /tmp/pos_patch_src/flintcore/test/

# 2. apply instrumentation
(cd /tmp/pos_patch_src && patch -p0 -s < "$REPO/tools/poseidon_trace/reference-trace.patch")

# 3. compile into /tmp/pos_patch
rm -rf /tmp/pos_patch && mkdir -p /tmp/pos_patch
javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" -d /tmp/pos_patch \
  /tmp/pos_patch_src/flintcore/test/Trace.java \
  /tmp/pos_patch_src/flintcore/test/PoseidonGenerator.java \
  /tmp/pos_patch_src/net/minecraft/server/{Chunk,World,BlockFlowing,BlockStationary,Block,BlockFluids}.java
```

## Run a window and diff against Rust

The window `x0,y0,z0,x1,y1,z1` is armed only around the populate pass so the
noise from chunk loading is skipped. Run both sides from the repo root (the
reference run reads `./poseidon.yml`):

```sh
# Java side
java -Dflint.trace=0,60,0,15,80,15 \
  -cp "/tmp/pos_patch:target/poseidon_classes:.references/Project-Poseidon/libs/*" \
  flintcore.test.PoseidonGenerator 42 /tmp/trace42 -1 1 -1 1 \
  | grep -E "^(A4|A5|B|G|PHYS|FA|CONV|RNG|RNG2|RNG3|J|FLOW|DOWN) " > /tmp/java_trace.txt

# Rust side
DEBUG_SEED=42 FLINTCORE_TRACE="0,60,0,15,80,15" \
  cargo test --release --test debug_cube -- --nocapture \
  | grep -E "^(A4|A5|B|G|PHYS|FA|CONV|RNG|RNG2|RNG3|J|FLOW|DOWN) " > /tmp/rust_trace.txt

diff /tmp/java_trace.txt /tmp/rust_trace.txt   # must be empty
```

Event format (see `Trace.java`):

- `A4 x y z old->new` — 4-arg raw type write (`setRawTypeId`);
- `A5 x y z old.oldd->new.newd` — 5-arg raw type+data write; old data is
  captured before the old block's `remove()`, new data after the write;
- `B x y z oldd->newd` — raw metadata change (`Chunk.b`);
- `G x z oldh->newh` — height-map update;
- `PHYS` / `FA` / `CONV` / `J` / `FLOW` / `DOWN` / `RNG` / `RNG2` / `RNG3` —
  fluid-cascade and RNG probes in `Block`/`BlockFlowing`/`BlockFluids`.
