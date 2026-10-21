#!/bin/sh
# Regenerates the golden trace of the block writes of the reference.
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6); the script builds the instrumented copy,
# runs the generator and writes the trace of the reference.
#
#   sh tools/poseidon_trace/regenerate_golden.sh
set -e

cd "$(dirname "$0")/../.."

REPO=$(pwd)
CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
SRC="/tmp/pos_patch_src"
OUT="/tmp/pos_patch"
GOLDEN="tests/golden/poseidon_block_trace.txt"

# The scratch tree of the instrumented reference.
rm -rf "$SRC" "$OUT"
mkdir -p "$SRC/net/minecraft/server" "$SRC/flintcore/test" "$OUT"
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

# The trace of the reference: the window of the surface of the region.
java -Dflint.trace=0,60,0,15,80,15 -cp "$OUT:$CLASSES:$LIBS" \
  flintcore.test.PoseidonGenerator 42 /tmp/trace42 -1 1 -1 1 2>/dev/null \
  | grep -E "^(A4|A5|B|G|PHYS|FA|CONV|RNG|RNG2|RNG3|J|FLOW|DOWN) " > "$GOLDEN"

echo "$GOLDEN regenerated: $(wc -l < "$GOLDEN") lines"
