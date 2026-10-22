package flintcore.test;

import net.minecraft.server.ChunkCoordIntPair;
import net.minecraft.server.ChunkPosition;

import java.util.HashSet;
import java.util.Set;

/**
 * Prints the order of the iteration of the sets of the reference
 * (`java.util.HashSet` ✓).
 *
 * The port iterates its own sets ✓ (a `HashSet` of Rust ✓ — the order is
 * random for every process ✓) ⇒ the order of the iteration of the reference
 * has to be reproduced ✓ (see W-53 of the plan ✓).
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_hash/PoseidonHashOrder.java
 *   java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonHashOrder &lt;scenario&gt; &lt;out&gt;
 * </pre>
 *
 * The scenarios are the two sets of the reference ✓:
 * <ul>
 *   <li>`chunks` ✓ — the chunks of the random ticks ✓ (`World.j()` ✓ — the
 *       set of the `ChunkCoordIntPair` ✓ around the player ✓);</li>
 *   <li>`wire` ✓ — the positions of the notifications of the redstone wire
 *       ✓ (`BlockRedstoneWire.b` ✓ — the set of the `ChunkPosition` ✓).</li>
 * </ul>
 */
public class PoseidonHashOrder {

    /** The scenario of the chunks of the random ticks. */
    public static void main(String[] args) throws Exception {
        if (args.length < 2) {
            System.err.println("Usage: PoseidonHashOrder <scenario> <out>");
            System.err.println("Scenarios: chunks, wire");
            System.exit(2);
        }
        String scenario = args[0];
        String outPath = args[1];

        StringBuilder out = new StringBuilder();
        out.append("# scenario=").append(scenario).append('\n');

        if (scenario.equals("chunks")) {
            // Java `World.j()` (`World.java:1878-1890` ✓): the chunks around
            // the player ✓ — the set of the `ChunkCoordIntPair` ✓.
            Set<ChunkCoordIntPair> chunks = new HashSet<ChunkCoordIntPair>();
            for (int k = -9; k <= 9; ++k) {
                for (int l = -9; l <= 9; ++l) {
                    chunks.add(new ChunkCoordIntPair(k, l));
                }
            }
            out.append("# x z\n");
            for (ChunkCoordIntPair chunk : chunks) {
                out.append(chunk.x).append(' ').append(chunk.z).append('\n');
            }
        } else if (scenario.equals("wire")) {
            // Java `BlockRedstoneWire.b` (`BlockRedstoneWire.java:175-181` ✓):
            // the seven positions around every wire that changes ✓.
            Set<ChunkPosition> wire = new HashSet<ChunkPosition>();
            int[][] wires = {{8, 64, 8}, {9, 64, 8}, {8, 64, 9}, {10, 64, 8}, {8, 64, 7}};
            for (int[] w : wires) {
                wire.add(new ChunkPosition(w[0], w[1], w[2]));
                wire.add(new ChunkPosition(w[0] - 1, w[1], w[2]));
                wire.add(new ChunkPosition(w[0] + 1, w[1], w[2]));
                wire.add(new ChunkPosition(w[0], w[1] - 1, w[2]));
                wire.add(new ChunkPosition(w[0], w[1] + 1, w[2]));
                wire.add(new ChunkPosition(w[0], w[1], w[2] - 1));
                wire.add(new ChunkPosition(w[0], w[1], w[2] + 1));
            }
            out.append("# x y z\n");
            for (ChunkPosition position : wire) {
                out.append(position.x).append(' ').append(position.y).append(' ').append(position.z).append('\n');
            }
        } else if (scenario.startsWith("ints:")) {
            // The keys `0..n-1` ✓ — the hash of the `Integer` ✓ is its value ✓.
            int n = Integer.parseInt(scenario.substring("ints:".length()));
            Set<Integer> keys = new HashSet<Integer>();
            for (int i = 0; i < n; ++i) {
                keys.add(Integer.valueOf(i));
            }
            out.append("# key\n");
            for (Integer key : keys) {
                out.append(key).append('\n');
            }
        } else if (scenario.startsWith("collide:")) {
            // The keys `i * 64` ✓ — they all fall into the same bucket of a
            // table of sixty-four buckets ✓ ⇒ the bucket becomes a tree ✓.
            int n = Integer.parseInt(scenario.substring("collide:".length()));
            Set<Integer> keys = new HashSet<Integer>();
            for (int i = 0; i < n; ++i) {
                keys.add(Integer.valueOf(i * 64));
            }
            out.append("# key\n");
            for (Integer key : keys) {
                out.append(key).append('\n');
            }
        } else if (scenario.startsWith("random:")) {
            // The random keys ✓ — the stream of the reference ✓ (`java.util.Random` ✓).
            String[] parts = scenario.split(":");
            int n = Integer.parseInt(parts[1]);
            long seed = Long.parseLong(parts[2]);
            Set<Integer> keys = new HashSet<Integer>();
            java.util.Random random = new java.util.Random(seed);
            for (int i = 0; i < n; ++i) {
                keys.add(Integer.valueOf(random.nextInt()));
            }
            out.append("# key\n");
            for (Integer key : keys) {
                out.append(key).append('\n');
            }
        } else {
            System.err.println("unknown scenario " + scenario);
            System.exit(2);
        }

        java.io.File outFile = new java.io.File(outPath);
        java.io.File parent = outFile.getParentFile();
        if (parent != null) {
            parent.mkdirs();
        }
        java.io.PrintWriter writer = new java.io.PrintWriter(
                new java.io.BufferedWriter(new java.io.FileWriter(outFile)));
        writer.print(out);
        writer.close();
        System.out.printf("%s: the order is written to %s%n", scenario, outPath);
    }
}
