package flintcore.test;

import net.minecraft.server.*;

import java.io.*;
import java.util.Random;

/**
 * Dumps the drops of the blocks of the reference (Project-Poseidon, Beta 1.7.3).
 *
 * The output is the golden file the flintcore port is compared against: for
 * every block and every metadata the number of the dropped items and the
 * item that is dropped ✓ — the two values are drawn from the random of the
 * world in this order ✓ (`Block.dropNaturally`), so the seed decides them ✓.
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_dump/PoseidonDrops.java
 *   java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonDrops &lt;seed&gt; &lt;out&gt;
 * </pre>
 *
 * The format of a line: `&lt;block id&gt; &lt;metadata&gt; &lt;count&gt; &lt;item id&gt;` ✓.
 */
public class PoseidonDrops {

    public static void main(String[] args) throws Exception {
        long seed = args.length > 0 ? Long.parseLong(args[0]) : 12345L;
        String outPath = args.length > 1 ? args[1] : "drops.txt";

        // The blocks are initialized first (see PoseidonCrafting).
        int blockCount = Block.byId.length;

        PrintWriter out = new PrintWriter(new BufferedWriter(new FileWriter(outPath)));
        out.printf("# drops of the blocks of the reference, seed=%d, %d blocks%n", seed, blockCount);

        for (int id = 1; id < blockCount; ++id) {
            Block block = Block.byId[id];
            if (block == null) {
                continue;
            }
            for (int metadata = 0; metadata < 16; ++metadata) {
                // The number of the items and the item itself are drawn from
                // the random of the world, in this order.
                Random random = new Random(seed);
                int count = block.a(random);
                int item = block.a(metadata, random);
                out.printf("%d %d %d %d%n", id, metadata, count, item);
            }
        }

        out.flush();
        out.close();
        System.out.printf("drops written to %s%n", outPath);
    }
}
