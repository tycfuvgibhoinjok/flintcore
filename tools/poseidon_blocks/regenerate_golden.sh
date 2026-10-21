#!/bin/sh
# Regenerates the golden traces of the scenarios of the blocks.
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6); the script builds the instrumented copy,
# runs the scenarios and writes their traces.
#
#   sh tools/poseidon_blocks/regenerate_golden.sh
set -e

cd "$(dirname "$0")/../.."

REPO=$(pwd)
CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
SRC="/tmp/pos_patch_src"
OUT="/tmp/pos_patch"
GOLDEN="tests/golden/poseidon_block_traces"

rm -rf "$SRC" "$OUT"
mkdir -p "$SRC/net/minecraft/server" "$SRC/flintcore/test" "$OUT" "$GOLDEN"
cp .references/Project-Poseidon/src/main/java/net/minecraft/server/*.java "$SRC/net/minecraft/server/"
cp tools/poseidon_dump/PoseidonGenerator.java tools/poseidon_trace/Trace.java "$SRC/flintcore/test/"
(cd "$SRC" && patch -p0 -s < "$REPO/tools/poseidon_trace/reference-trace.patch")

javac -nowarn -cp "$CLASSES:$LIBS" -d "$OUT" \
  "$SRC/flintcore/test/Trace.java" \
  "$SRC/flintcore/test/PoseidonGenerator.java" \
  "$SRC/net/minecraft/server/Chunk.java" \
  "$SRC/net/minecraft/server/World.java" \
  "$SRC/net/minecraft/server/BlockFlowing.java" \
  "$SRC/net/minecraft/server/BlockStationary.java" \
  "$SRC/net/minecraft/server/Block.java" \
  "$SRC/net/minecraft/server/BlockFluids.java" 2>/dev/null

# The harness of the mobs builds the world of the scenarios: it has the
# server of CraftBukkit ✓ (the events of the items ✓) and the chunks of the
# generator of the dumps ✓ (the terrain of the port ✓).
javac -nowarn -cp "$OUT:$CLASSES:$LIBS" -d "$OUT" \
  tools/poseidon_mobs/PoseidonMobTrace.java 2>/dev/null

javac -nowarn -cp "$OUT:$CLASSES:$LIBS" -d "$OUT" \
  tools/poseidon_blocks/PoseidonBlockTrace.java 2>/dev/null

for s in water_spread lava_into_water sand_fall; do
  java -Dflint.trace=0,60,0,15,80,15 -cp "$OUT:$CLASSES:$LIBS" \
    flintcore.test.PoseidonBlockTrace "$s" 42 20 2>/dev/null \
    | awk '/^=== scenario ===$/ { found = 1; next } found' \
    | grep -E "^(A4|A5|B|G|PHYS|FA|CONV|RNG|RNG2|RNG3|J|FLOW|DOWN) " > "$GOLDEN/$s.txt"
  echo "$GOLDEN/$s.txt regenerated: $(wc -l < "$GOLDEN/$s.txt") lines"
done
