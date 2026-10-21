#!/bin/sh
# Regenerates the golden traces of the scenarios of the entities.
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6); the script builds the harnesses, runs the
# scenarios and writes their traces.
#
#   sh tools/poseidon_entities/regenerate_golden.sh
set -e

cd "$(dirname "$0")/../.."

REPO=$(pwd)
CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
GOLDEN="tests/golden/poseidon_entity_traces.txt"

javac -nowarn -cp "$CLASSES:$LIBS" -d "$CLASSES" \
  tools/poseidon_mobs/PoseidonMobTrace.java \
  tools/poseidon_entities/PoseidonEntityTrace.java 2>/dev/null

: > "$GOLDEN"
# The dropped item and the primed TNT take their first velocity from
# `Math.random()` in the reference (`EntityItem.java:41-43` ✓,
# `EntityTNTPrimed.java:29-32` ✓) => their traces are not reproducible ✓ =>
# they are not in the golden file ✓.
for s in arrow snowball egg boat; do
  java -cp "$CLASSES:$LIBS" \
    flintcore.test.PoseidonEntityTrace "$s" 12345 "/tmp/poseidon_entity_$s.txt" 100 2>/dev/null
  cat "/tmp/poseidon_entity_$s.txt" >> "$GOLDEN"
  echo "$s: $(grep -vc '^#' "/tmp/poseidon_entity_$s.txt") ticks"
done
echo "$GOLDEN regenerated"
