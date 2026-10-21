package flintcore.test;

import net.minecraft.server.*;

import java.io.File;
import java.lang.reflect.Method;

/**
 * Runs a scripted scenario of blocks in the reference (Project-Poseidon, Beta
 * 1.7.3) and prints the trace of the block writes.
 *
 * The reference is instrumented with
 * `tools/poseidon_trace/reference-trace.patch` ✓: every block write is printed
 * with the coordinates, the old and the new value ✓.  The port prints the same
 * lines from its own write pipeline ✓, so the two traces can be compared
 * event by event ✓.
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_blocks/PoseidonBlockTrace.java
 *   java -Dflint.trace=0,60,0,15,80,15 \
 *       -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonBlockTrace &lt;scenario&gt; &lt;seed&gt; &lt;ticks&gt;
 * </pre>
 *
 * The scenarios are the same as the ones of the port
 * (`tests/block_trace_scenarios.rs` ✓).
 */
public class PoseidonBlockTrace {

    /** The scenarios of the harness. */
    public static final String[] SCENARIOS = {"water_spread", "lava_into_water", "water_into_lava",
        "sand_fall", "lava_spread", "fire_spread", "redstone_torch", "piston_push"};

    /** Removes a directory with its content. */
    private static void deleteRecursive(File file) {
        File[] children = file.listFiles();
        if (children != null) {
            for (File child : children) {
                deleteRecursive(child);
            }
        }
        file.delete();
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 3) {
            System.err.println("Usage: PoseidonBlockTrace <scenario> <seed> <ticks>");
            System.err.println("Scenarios: water_spread, lava_into_water, sand_fall");
            System.exit(2);
        }
        String scenario = args[0];
        long seed = Long.parseLong(args[1]);
        int ticks = Integer.parseInt(args[2]);

        // The world of the generator of the dumps with the server of
        // CraftBukkit ✓: the terrain of the generator is the terrain of the
        // port ✓ (the same trace of the generation ✓) and the server is
        // needed by the events of CraftBukkit ✓ (the items that drop ✓ and
        // the leaves that decay ✓).
        File worldDir = new File(System.getProperty("java.io.tmpdir"),
                "poseidon_block_world_" + scenario + "_" + seed);
        deleteRecursive(worldDir);
        worldDir.mkdirs();
        WorldServer world = PoseidonMobTrace.createWorldWithGeneratorProvider(seed, worldDir);
        PoseidonGenerator.populateRegion(world, -1, 1, -1, 1);

        // The generation of the world is not a part of the scenario ✓: the
        // trace of the writes is armed after the floor is made ✓.
        flintcore.test.Trace.armed = false;
        // The marker of the scenario: the trace of the generation is above it ✓.
        System.out.println("=== scenario ===");

        // The floor of the scenario: the same on both sides ✓.
        //
        // The physics is suppressed while the floor is made: the blocks of
        // the terrain that die would drop an item ✓ and the spawn of the item
        // needs the server of CraftBukkit ✓ — the world of the generator has
        // no server ✓.
        world.suppressPhysics = true;
        flintcore.test.Trace.armed = true;
        for (int x = 0; x < 16; ++x) {
            for (int z = 0; z < 16; ++z) {
                for (int y = 64; y < 80; ++y) {
                    world.setTypeIdAndData(x, y, z, 0, 0);
                }
                world.setTypeIdAndData(x, 63, z, 1, 0);
            }
        }
        world.suppressPhysics = false;

        if (scenario.equals("water_spread")) {
            world.setTypeIdAndData(8, 64, 8, 9, 0);
        } else if (scenario.equals("lava_into_water")) {
            world.setTypeIdAndData(8, 64, 8, 11, 0);
            world.setTypeIdAndData(9, 64, 8, 9, 0);
        } else if (scenario.equals("water_into_lava")) {
            world.setTypeIdAndData(8, 64, 8, 9, 0);
            world.setTypeIdAndData(9, 64, 8, 11, 0);
        } else if (scenario.equals("sand_fall")) {
            world.setTypeIdAndData(8, 64, 8, 0, 0);
            for (int y = 65; y <= 67; ++y) {
                world.setTypeIdAndData(8, y, 8, 12, 0);
            }
        } else if (scenario.equals("lava_spread")) {
            world.setTypeIdAndData(8, 64, 8, 11, 0);
        } else if (scenario.equals("fire_spread")) {
            // The planks of the floor and the fire above them.
            for (int x = 5; x <= 11; ++x) {
                world.setTypeIdAndData(x, 64, 8, 5, 0);
            }
            world.setTypeIdAndData(8, 65, 8, 51, 0);
        } else if (scenario.equals("redstone_torch")) {
            // The torch of the redstone on the floor and the wire next to it.
            world.setTypeIdAndData(8, 64, 8, 75, 5);
            world.setTypeIdAndData(9, 64, 8, 55, 0);
        } else if (scenario.equals("piston_push")) {
            // The piston, the block above it and the lever that powers it.
            world.setTypeIdAndData(8, 64, 8, 33, 0);
            world.setTypeIdAndData(8, 65, 8, 1, 0);
            world.setTypeIdAndData(9, 64, 8, 69, 9);
        } else {
            System.err.println("unknown scenario " + scenario);
            System.exit(2);
        }

        // The ticks of the world: the scheduled ticks and the random ticks
        // (`World.doTick()`).
        Method randomTicks = World.class.getDeclaredMethod("j");
        randomTicks.setAccessible(true);
        for (int t = 0; t < ticks; ++t) {
            // Java `World.doTick()`: the time of the world is advanced before
            // the scheduled ticks are processed ✓.
            world.worldData.a(world.worldData.f() + 1L);
            world.a(false);
            randomTicks.invoke(world);
        }

        System.out.printf("%s: %d ticks of seed %d%n", scenario, ticks, seed);
    }
}
