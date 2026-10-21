package flintcore.test;

import net.minecraft.server.*;
import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.*;

/**
 * Dumps the draws of the random of the world (`World.random`) of a scripted
 * sequence of the calls that use it.
 *
 * The output is the golden trace the flintcore port is compared against: the
 * numbers that every step of the sequence draws from the random of the world,
 * with the number of the bits of every draw (`nextInt(31)` of a bound and
 * `nextInt(32)` of a plain number give different values ✓).
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_worldrand/PoseidonWorldRandomTrace.java
 *   java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonWorldRandomTrace &lt;seed&gt; &lt;out&gt;
 * </pre>
 *
 * Java `World.java:51`: `public Random random = new Random();` — the seed
 * comes from the system clock, so the harness sets it and the trace is
 * reproducible ✓.
 */
public class PoseidonWorldRandomTrace {

    /** The draws of the random of the world since the last step. */
    private static final List<String> DRAWS = new ArrayList<String>();

    public static void main(String[] args) throws Exception {
        long seed = args.length > 0 ? Long.parseLong(args[0]) : 12345L;
        String outPath = args.length > 1 ? args[1] : "world_random.txt";

        File worldDir = new File(System.getProperty("java.io.tmpdir"),
                "poseidon_worldrandom_world_" + seed);
        worldDir.mkdirs();
        WorldServer world = PoseidonMobTrace.createWorld(seed, worldDir);

        // The random of the world is seeded like the harness of the mobs.
        world.random = new Random(seed) {
            @Override
            protected int next(int bits) {
                int value = super.next(bits);
                DRAWS.add(bits + ":" + value);
                return value;
            }
        };

        PrintWriter out = new PrintWriter(new BufferedWriter(new FileWriter(outPath)));
        out.printf("# world random trace, seed=%d%n", seed);
        out.printf("# step draws (bits:value)%n");

        step(out, "start");

        // Java `Block.dropNaturally(World, int, int, int, int, float)`: the
        // number of the items, the chance of every item, the item itself
        // and the position of the item entity.
        Block.byId[1].dropNaturally(world, 0, 64, 0, 0, 1.0F);
        step(out, "drop_stone");

        Block.byId[13].dropNaturally(world, 1, 64, 0, 0, 1.0F);
        step(out, "drop_gravel");

        Block.byId[18].dropNaturally(world, 2, 64, 0, 0, 1.0F);
        step(out, "drop_leaves");

        Block.byId[59].dropNaturally(world, 3, 64, 0, 7, 1.0F);
        step(out, "drop_crops");

        Block.byId[17].dropNaturally(world, 4, 64, 0, 0, 1.0F);
        step(out, "drop_log");

        // Java `World.i()`: the durations of the weather and of the thunder
        // are drawn from the random of the world.
        world.worldData.setThunderDuration(0);
        world.worldData.setWeatherDuration(0);
        Method weather = World.class.getDeclaredMethod("i");
        weather.setAccessible(true);
        weather.invoke(world);
        step(out, "weather");

        out.flush();
        out.close();
        System.out.printf("world random trace written to %s%n", outPath);
    }

    /** Writes the draws of the random of the world since the last step. */
    private static void step(PrintWriter out, String name) {
        StringBuilder sb = new StringBuilder(name);
        for (String draw : DRAWS) {
            sb.append(' ').append(draw);
        }
        out.println(sb);
        DRAWS.clear();
    }
}
