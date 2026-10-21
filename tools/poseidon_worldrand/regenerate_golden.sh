#!/bin/sh
# Regenerates the golden trace of the random of the world.
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6), the script does the rest:
#
#   sh tools/poseidon_worldrand/regenerate_golden.sh
set -e

cd "$(dirname "$0")/../.."

CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
OUT="${TMPDIR:-/tmp}/world_random"

javac -nowarn -cp "$CLASSES:$LIBS" -d "$CLASSES" \
  tools/poseidon_worldrand/PoseidonWorldRandomTrace.java

mkdir -p "$OUT"
java -cp "$CLASSES:$LIBS" flintcore.test.PoseidonWorldRandomTrace 12345 "$OUT/golden.txt" \
  > /dev/null

cp "$OUT/golden.txt" tests/golden/poseidon_world_random.txt
echo "tests/golden/poseidon_world_random.txt regenerated"
