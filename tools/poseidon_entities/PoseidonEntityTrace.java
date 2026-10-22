package flintcore.test;

import net.minecraft.server.*;

import java.io.BufferedWriter;
import java.io.File;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.Locale;
import java.util.Random;

/**
 * Runs a scripted scenario of an entity in the reference (Project-Poseidon,
 * Beta 1.7.3) and dumps the state of the entity after every tick.
 *
 * The harness of the mobs (`tools/poseidon_mobs/PoseidonMobTrace.java` ✓)
 * covers the mobs; this one covers the other kinds of the entities ✓
 * (the projectiles ✓, the dropped item ✓, the primed TNT ✓, the boat ✓).
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_entities/PoseidonEntityTrace.java
 *   java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonEntityTrace &lt;scenario&gt; &lt;seed&gt; &lt;out&gt; [ticks]
 * </pre>
 *
 * The scenarios are the same as the ones of the port
 * (`tests/entity_trace.rs` ✓).
 */
public class PoseidonEntityTrace {

    /** The floor of the scenarios: the grass at the height 63. */
    private static final int FLOOR_Y = 63;
    /** The radius of the floor of the scenarios. */
    private static final int RADIUS = 24;

    public static void main(String[] args) throws Exception {
        if (args.length < 3) {
            System.err.println("Usage: PoseidonEntityTrace <scenario> <seed> <out> [ticks]");
            System.err.println("Scenarios: arrow, snowball, egg, item, tnt, boat");
            System.exit(2);
        }
        String scenario = args[0];
        long seed = Long.parseLong(args[1]);
        String outPath = args[2];
        int ticks = args.length > 3 ? Integer.parseInt(args[3]) : 100;

        // The shared random of the entities (`Entity.SHARED_RANDOM` ✓): the
        // reference takes the seed from the system clock, the harness sets it
        // so the traces are reproducible ✓.
        Entity.SHARED_RANDOM = new Random(seed);

        File worldDir = new File(System.getProperty("java.io.tmpdir"),
                "poseidon_entity_world_" + scenario + "_" + seed);
        worldDir.mkdirs();
        WorldServer world = PoseidonMobTrace.createWorld(seed, worldDir);
        flatFloor(world);
        while (world.doLighting()) {
            ;
        }

        Entity entity = createEntity(world, scenario);
        entity.setLocation(0.5D, 66.0D, 0.5D, 0.0F, 0.0F);
        world.addEntity(entity);

        File outFile = new File(outPath);
        File parent = outFile.getParentFile();
        if (parent != null) {
            parent.mkdirs();
        }
        try (PrintWriter out = new PrintWriter(new BufferedWriter(new FileWriter(outFile)))) {
            out.printf(Locale.ROOT, "# scenario=%s seed=%d ticks=%d%n", scenario, seed, ticks);
            out.printf("# tick x y z motX motY motZ yaw pitch onGround%n");
            for (int t = 0; t < ticks; t++) {
                // The tick of the entity (`Entity.m_()` ✓): the world of the
                // harness has no player, so the entities are ticked by hand ✓.
                // The world removes the dead entities at the end of its tick
                // (`World.tickEntities`), so a dead entity is not ticked any
                // more ✓ — its state is frozen ✓.
                if (!entity.dead) {
                    entity.m_();
                }
                dump(out, t, entity);
            }
        }
        System.out.printf("%s: %d ticks written to %s%n", scenario, ticks, outPath);
    }

    /** The entity of a scenario. */
    private static Entity createEntity(World world, String scenario) {
        switch (scenario) {
            case "arrow":
                return new EntityArrow(world, 0.5D, 66.0D, 0.5D);
            case "snowball":
                return new EntitySnowball(world, 0.5D, 66.0D, 0.5D);
            case "egg":
                return new EntityEgg(world, 0.5D, 66.0D, 0.5D);
            case "item":
                return new EntityItem(world, 0.5D, 66.0D, 0.5D,
                        new ItemStack(3, 1, 0));
            case "tnt":
                return new EntityTNTPrimed(world, 0.5D, 66.0D, 0.5D);
            case "boat":
                return new EntityBoat(world, 0.5D, 66.0D, 0.5D);
            default:
                throw new IllegalArgumentException("unknown scenario " + scenario);
        }
    }

    /** The flat floor of the scenarios. */
    private static void flatFloor(World world) {
        for (int x = -RADIUS; x <= RADIUS; x++) {
            for (int z = -RADIUS; z <= RADIUS; z++) {
                for (int y = FLOOR_Y + 1; y <= FLOOR_Y + 14; y++) {
                    world.setTypeId(x, y, z, 0);
                }
                world.setTypeId(x, FLOOR_Y, z, Block.GRASS.id);
            }
        }
    }

    /** The state of the entity after a tick. */
    private static void dump(PrintWriter out, int tick, Entity entity) {
        out.printf(Locale.ROOT, "%d %.9f %.9f %.9f %.9f %.9f %.9f %.7f %.7f %d%n",
                tick,
                entity.locX, entity.locY, entity.locZ,
                entity.motX, entity.motY, entity.motZ,
                entity.yaw, entity.pitch,
                entity.onGround ? 1 : 0);
    }
}
