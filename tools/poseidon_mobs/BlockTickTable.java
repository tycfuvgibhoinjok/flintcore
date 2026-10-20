package flintcore.test;

import net.minecraft.server.Block;

/**
 * Prints the table of the random ticks of the reference (`Block.n[]`).
 *
 * The flag is set by `Block.a(boolean)` in the constructor of every block, so
 * the table is the list of the blocks that the world ticks randomly.
 */
public final class BlockTickTable {
    public static void main(String[] args) throws Exception {
        // The static fields of the class have to be initialised.
        Block[] byId = Block.byId;
        StringBuilder ticking = new StringBuilder();
        StringBuilder plain = new StringBuilder();
        for (int i = 0; i < 256; i++) {
            if (Block.byId[i] == null) {
                continue;
            }
            if (Block.n[i]) {
                ticking.append(i).append(' ').append(Block.byId[i].getClass().getSimpleName()).append('\n');
            } else {
                plain.append(i).append(' ').append(Block.byId[i].getClass().getSimpleName()).append('\n');
            }
        }
        System.out.println("TICKING");
        System.out.print(ticking);
        System.out.println("PLAIN");
        System.out.print(plain);
    }
}
