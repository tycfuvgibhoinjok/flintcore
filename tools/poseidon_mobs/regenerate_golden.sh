#!/bin/sh
# Regenerates the golden movement traces of the mobs.
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6), the script does the rest:
#
#   sh tools/poseidon_mobs/regenerate_golden.sh
set -e

cd "$(dirname "$0")/../.."

CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
OUT="${TMPDIR:-/tmp}/mob_traces"

javac -nowarn -cp "$CLASSES:$LIBS" -d "$CLASSES" tools/poseidon_mobs/PoseidonMobTrace.java

mkdir -p "$OUT"
for s in flat chase water wall lava slime squid ghast; do
  java -cp "$CLASSES:$LIBS" flintcore.test.PoseidonMobTrace "$s" 12345 "$OUT/$s.txt" 300 \
    > /dev/null
done

# The header of the golden file.
head -n 27 tests/golden/poseidon_mob_traces.txt > "$OUT/golden.txt"

for s in flat chase water wall lava slime squid ghast; do
  echo "=== scenario $s ===" >> "$OUT/golden.txt"
  cat "$OUT/$s.txt" >> "$OUT/golden.txt"
done

cp "$OUT/golden.txt" tests/golden/poseidon_mob_traces.txt
echo "tests/golden/poseidon_mob_traces.txt regenerated"
