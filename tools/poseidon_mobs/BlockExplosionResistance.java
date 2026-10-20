package flintcore.test;

import net.minecraft.server.Block;

/**
 * Prints the table of the resistance to the explosion of the reference.
 *
 * The resistance is `Block.a(Entity)`: the durability of the block divided
 * by five.
 */
public final class BlockExplosionResistance {
    public static void main(String[] args) throws Exception {
        // The static fields of the class have to be initialised.
        Block[] byId = Block.byId;
        for (int i = 0; i < 256; i++) {
            Block block = Block.byId[i];
            if (block == null) {
                continue;
            }
            System.out.printf("%d %s %f%n", i, block.getClass().getSimpleName(), block.a((net.minecraft.server.Entity) null));
        }
    }
}
