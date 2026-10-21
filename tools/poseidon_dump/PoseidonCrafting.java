package flintcore.test;

import net.minecraft.server.*;

import java.io.*;
import java.lang.reflect.Field;
import java.util.*;

/**
 * Dumps the recipes of the crafting manager of the reference (Project-Poseidon,
 * Beta 1.7.3) in a normalized form.
 *
 * The output is the golden file the flintcore port is compared against: every
 * recipe with its result and its ingredients, so a recipe that the port does
 * not have (or that it has with another ingredient) is visible.
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_dump/PoseidonCrafting.java
 *   java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonCrafting &lt;out&gt;
 * </pre>
 *
 * The format of a line:
 *
 * <pre>
 *   shaped &lt;result id&gt;:&lt;count&gt;:&lt;damage&gt; &lt;width&gt;x&lt;height&gt; &lt;cell&gt; ...
 *   shapeless &lt;result id&gt;:&lt;count&gt;:&lt;damage&gt; &lt;ingredient&gt; ...
 * </pre>
 *
 * A cell is `-` (empty) or `&lt;id&gt;:&lt;damage&gt;`; the damage `-1` means "any
 * damage" ✓.
 */
public class PoseidonCrafting {

    public static void main(String[] args) throws Exception {
        String outPath = args.length > 0 ? args[0] : "crafting.txt";

        // The blocks are initialized first: the statistics of the reference
        // (`StatisticList.b()`) ask the crafting manager for its recipes
        // while the blocks are initialized (`Block.java:553`), and the
        // manager asks the blocks for the items of the recipes — the order of
        // the initialization decides whether the cycle breaks.
        int blockCount = Block.byId.length;

        CraftingManager manager = CraftingManager.getInstance();
        Field recipesField = CraftingManager.class.getDeclaredField("b");
        recipesField.setAccessible(true);
        List<?> recipes = (List<?>) recipesField.get(manager);

        PrintWriter out = new PrintWriter(new BufferedWriter(new FileWriter(outPath)));
        out.printf("# crafting recipes of the reference, %d recipes, %d blocks%n", recipes.size(), blockCount);

        for (Object recipe : recipes) {
            if (recipe instanceof ShapedRecipes) {
                out.println(shaped((ShapedRecipes) recipe));
            } else if (recipe instanceof ShapelessRecipes) {
                out.println(shapeless((ShapelessRecipes) recipe));
            }
        }

        out.flush();
        out.close();
        System.out.printf("%d recipes written to %s%n", recipes.size(), outPath);
    }

    private static String shaped(ShapedRecipes recipe) throws Exception {
        Field widthField = ShapedRecipes.class.getDeclaredField("b");
        Field heightField = ShapedRecipes.class.getDeclaredField("c");
        Field ingredientsField = ShapedRecipes.class.getDeclaredField("d");
        Field resultField = ShapedRecipes.class.getDeclaredField("e");
        widthField.setAccessible(true);
        heightField.setAccessible(true);
        ingredientsField.setAccessible(true);
        resultField.setAccessible(true);

        int width = widthField.getInt(recipe);
        int height = heightField.getInt(recipe);
        ItemStack[] ingredients = (ItemStack[]) ingredientsField.get(recipe);
        ItemStack result = (ItemStack) resultField.get(recipe);

        StringBuilder sb = new StringBuilder("shaped ");
        sb.append(stack(result)).append(' ').append(width).append('x').append(height);
        for (ItemStack ingredient : ingredients) {
            sb.append(' ').append(ingredient == null ? "-" : ingredient(ingredient));
        }
        return sb.toString();
    }

    private static String shapeless(ShapelessRecipes recipe) throws Exception {
        Field resultField = ShapelessRecipes.class.getDeclaredField("a");
        Field ingredientsField = ShapelessRecipes.class.getDeclaredField("b");
        resultField.setAccessible(true);
        ingredientsField.setAccessible(true);

        ItemStack result = (ItemStack) resultField.get(recipe);
        List<?> ingredients = (List<?>) ingredientsField.get(recipe);

        StringBuilder sb = new StringBuilder("shapeless ");
        sb.append(stack(result));
        for (Object ingredient : ingredients) {
            sb.append(' ').append(ingredient((ItemStack) ingredient));
        }
        return sb.toString();
    }

    /** The result of a recipe: the id, the number of the items and the damage. */
    private static String stack(ItemStack stack) {
        return stack.id + ":" + stack.count + ":" + stack.getData();
    }

    /**
     * An ingredient of a recipe: the id and the damage.
     *
     * The number of the items of an ingredient is always one ✓, so it is not
     * written ✓.
     */
    private static String ingredient(ItemStack stack) {
        return stack.id + ":" + stack.getData();
    }
}
