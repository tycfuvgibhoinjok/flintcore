#!/bin/sh
# Regenerates the golden files of the tables of the reference:
#
#   tests/golden/poseidon_crafting.txt — the recipes of the crafting manager
#   tests/golden/poseidon_drops.txt    — the drops of the blocks
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6), the script does the rest:
#
#   sh tools/poseidon_dump/regenerate_tables.sh
set -e

cd "$(dirname "$0")/../.."

CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
OUT="${TMPDIR:-/tmp}/reference_tables"

javac -nowarn -cp "$CLASSES:$LIBS" -d "$CLASSES" \
  tools/poseidon_dump/PoseidonCrafting.java \
  tools/poseidon_dump/PoseidonDrops.java

mkdir -p "$OUT"
java -cp "$CLASSES:$LIBS" flintcore.test.PoseidonCrafting "$OUT/crafting.txt" > /dev/null
java -cp "$CLASSES:$LIBS" flintcore.test.PoseidonDrops 12345 "$OUT/drops.txt" > /dev/null

cp "$OUT/crafting.txt" tests/golden/poseidon_crafting.txt
cp "$OUT/drops.txt" tests/golden/poseidon_drops.txt
echo "tests/golden/poseidon_crafting.txt and tests/golden/poseidon_drops.txt regenerated"
