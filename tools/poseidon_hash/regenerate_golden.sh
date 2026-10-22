#!/bin/sh
# Regenerates the golden order of the iteration of the sets of the reference.
#
# The reference sources have to be compiled into target/poseidon_classes once
# (see DEVELOPMENT.md section 6); the script builds the probe, runs the
# scenarios and writes their orders.
#
#   sh tools/poseidon_hash/regenerate_golden.sh
set -e

cd "$(dirname "$0")/../.."

CLASSES="target/poseidon_classes"
LIBS=".references/Project-Poseidon/libs/*"
GOLDEN="tests/golden/poseidon_hash_order.txt"

javac -nowarn -cp "$CLASSES:$LIBS" -d "$CLASSES" \
  tools/poseidon_hash/PoseidonHashOrder.java 2>/dev/null

: > "$GOLDEN"
for s in chunks wire ints:1 ints:5 ints:12 ints:13 ints:24 ints:25 ints:48 \
    ints:49 ints:64 ints:100 collide:5 collide:8 collide:9 collide:16 collide:20 \
    random:20:1 random:50:2 random:100:3 random:200:4 random:400:5 random:500:6 \
    random:700:7 random:800:8 random:900:9 random:1000:10; do
  java -cp "$CLASSES:$LIBS" \
    flintcore.test.PoseidonHashOrder "$s" "/tmp/poseidon_hash_$s.txt" 2>/dev/null
  cat "/tmp/poseidon_hash_$s.txt" >> "$GOLDEN"
  echo "$s: $(grep -vc '^#' "/tmp/poseidon_hash_$s.txt") keys"
done
echo "$GOLDEN regenerated"
