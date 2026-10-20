package flintcore.test;

import net.minecraft.server.Block;

/**
 * Prints the material of every block of the reference and whether the
 * material is solid.
 */
public final class BlockMaterial {
    public static void main(String[] args) throws Exception {
        // The static fields of the class have to be initialised.
        Block[] byId = Block.byId;
        for (int i = 0; i < 256; i++) {
            Block block = Block.byId[i];
            if (block == null) {
                continue;
            }
            System.out.printf(
                "%d %s %s%n", i, block.getClass().getSimpleName(), block.material.isSolid());
        }
    }
}
