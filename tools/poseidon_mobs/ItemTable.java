package flintcore.test;

import net.minecraft.server.Item;

/** Prints the table of the items of the reference (`Item.byId`). */
public final class ItemTable {
    public static void main(String[] args) throws Exception {
        // The static fields of the class have to be initialised.
        Item[] byId = Item.byId;
        for (int i = 0; i < byId.length; i++) {
            if (byId[i] == null) {
                continue;
            }
            System.out.printf("%d %s%n", i, byId[i].getClass().getSimpleName());
        }
    }
}
