package flintcore.test;

import net.minecraft.server.Block;
import net.minecraft.server.Material;

/**
 * Prints the tables of the blocks of the reference.
 *
 * The tables are the ones the world and the blocks use:
 * - `Block.o[]`: the block is an opaque cube,
 * - `Block.q[]`: the block hides the light,
 * - `Block.t[]`: the block notifies the clients when its data changes,
 * - `World.e()`: the block is solid (the material is solid and the block
 *   is one),
 * - `Block.e()`: the mobility of the block for the pistons.
 */
public final class BlockTable {
    public static void main(String[] args) throws Exception {
        // The static fields of the class have to be initialised.
        Block[] byId = Block.byId;
        for (int i = 0; i < 256; i++) {
            Block block = Block.byId[i];
            if (block == null) {
                continue;
            }
            boolean opaque = Block.o[i];
            int light = Block.q[i];
            boolean notify = Block.t[i];
            boolean solid = block.material.h() && block.b();
            int mobility = block.e();
            System.out.printf(
                "%d %s opaque=%s light=%d notify=%s solid=%s mobility=%d%n",
                i, block.getClass().getSimpleName(), opaque, light, notify, solid, mobility);
        }
    }
}
