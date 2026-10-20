package flintcore.test;

import net.minecraft.server.*;
import org.bukkit.Bukkit;
import org.bukkit.craftbukkit.CraftServer;
import org.bukkit.craftbukkit.CraftWorld;
import org.bukkit.craftbukkit.generator.InternalChunkGenerator;
import org.bukkit.craftbukkit.generator.NormalChunkGenerator;
import org.bukkit.plugin.SimplePluginManager;

import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.*;
import sun.misc.Unsafe;

/**
 * Dumps the per-tick movement state of a single mob of the reference
 * implementation (Project-Poseidon, Beta 1.7.3).
 *
 * The output is the golden trace the flintcore port is compared against: the
 * position, the velocity, the rotation, the ground flag, the fall distance,
 * the target and the path of the mob after every tick.
 *
 * <pre>
 *   javac -nowarn -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       -d target/poseidon_classes tools/poseidon_mobs/PoseidonMobTrace.java
 *   java -cp "target/poseidon_classes:.references/Project-Poseidon/libs/*" \
 *       flintcore.test.PoseidonMobTrace &lt;scenario&gt; &lt;seed&gt; &lt;out&gt; [ticks]
 * </pre>
 *
 * Poseidon shares a single {@link Random} between all entities
 * ({@code Entity.SHARED_RANDOM}). The first {@code setSeed()} call wins and
 * locks the stream, so the trace is reproducible.
 */
public class PoseidonMobTrace {

    private static Unsafe unsafe;

    static {
        try {
            Field f = Unsafe.class.getDeclaredField("theUnsafe");
            f.setAccessible(true);
            unsafe = (Unsafe) f.get(null);
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    // ------------------------------------------------------------------
    // World setup (the same trick as tools/poseidon_dump/PoseidonGenerator)
    // ------------------------------------------------------------------

    /** A chunk provider that generates the chunks in memory, without a disk. */
    public static class SimpleChunkProvider implements IChunkProvider {
        public final World world;
        public final ChunkProviderGenerate cpg;
        public final Map<Long, Chunk> chunks = new HashMap<>();

        public SimpleChunkProvider(World world) {
            this.world = world;
            this.cpg = new ChunkProviderGenerate(world, world.getSeed());
        }

        public long key(int x, int z) {
            return (((long) x) << 32) | (z & 0xFFFFFFFFL);
        }

        @Override
        public boolean isChunkLoaded(int i, int j) {
            return chunks.containsKey(key(i, j));
        }

        @Override
        public Chunk getOrCreateChunk(int i, int j) {
            long k = key(i, j);
            Chunk c = chunks.get(k);
            if (c == null) {
                c = cpg.getOrCreateChunk(i, j);
                chunks.put(k, c);
            }
            return c;
        }

        @Override
        public Chunk getChunkAt(int i, int j) {
            return getOrCreateChunk(i, j);
        }

        @Override
        public void getChunkAt(IChunkProvider provider, int i, int j) {
            Chunk c = getOrCreateChunk(i, j);
            if (!c.done) {
                for (int dx = -1; dx <= 1; dx++) {
                    for (int dz = -1; dz <= 1; dz++) {
                        getOrCreateChunk(i + dx, j + dz);
                    }
                }
                c.done = true;
                cpg.getChunkAt(provider, i, j);
            }
        }

        @Override
        public boolean saveChunks(boolean flag, IProgressUpdate progress) {
            return true;
        }

        @Override
        public boolean unloadChunks() {
            return false;
        }

        @Override
        public boolean canSave() {
            return true;
        }
    }

    private static void initObjectFields(Object obj, Class<?> clazz) throws Exception {
        for (Field f : clazz.getDeclaredFields()) {
            if (Modifier.isStatic(f.getModifiers())) continue;
            f.setAccessible(true);
            if (f.get(obj) == null) {
                if (f.getType().isAssignableFrom(ArrayList.class)) {
                    f.set(obj, new ArrayList());
                } else if (f.getType().isAssignableFrom(HashSet.class)) {
                    f.set(obj, new HashSet());
                } else if (f.getType().isAssignableFrom(TreeSet.class)) {
                    f.set(obj, new TreeSet());
                } else if (f.getType().isAssignableFrom(HashMap.class)) {
                    f.set(obj, new HashMap());
                } else if (f.getType().isAssignableFrom(Random.class)) {
                    f.set(obj, new Random());
                } else if (f.getType().isAssignableFrom(EntityList.class)) {
                    f.set(obj, new EntityList());
                } else if (f.getType().equals(Object.class)) {
                    f.set(obj, new Object());
                }
            }
        }
        if (clazz.getSuperclass() != null && clazz.getSuperclass() != Object.class) {
            initObjectFields(obj, clazz.getSuperclass());
        }
    }

    /** The Bukkit server is needed by the CraftBukkit events of the AI. */
    private static void setupBukkitServer(MinecraftServer ms) throws Exception {
        // The configuration manager answers the permission checks of the
        // CraftBukkit entities (isOp). The constructor of the real class
        // starts the whole server, so the object is created without it.
        ServerConfigurationManager scm =
                (ServerConfigurationManager) unsafe.allocateInstance(ServerConfigurationManager.class);
        initObjectFields(scm, ServerConfigurationManager.class);
        Field scmServerField = ServerConfigurationManager.class.getDeclaredField("server");
        scmServerField.setAccessible(true);
        scmServerField.set(scm, ms);
        ms.serverConfigurationManager = scm;

        CraftServer craftServer = (CraftServer) unsafe.allocateInstance(CraftServer.class);
        initObjectFields(craftServer, CraftServer.class);

        SimplePluginManager pm = (SimplePluginManager) unsafe.allocateInstance(SimplePluginManager.class);
        initObjectFields(pm, SimplePluginManager.class);

        // The default permissions of the plugin manager (the constructor of
        // the real class fills them).
        Field defaultPermsField = SimplePluginManager.class.getDeclaredField("defaultPerms");
        defaultPermsField.setAccessible(true);
        Map defaultPerms = (Map) defaultPermsField.get(pm);
        defaultPerms.put(true, new HashSet());
        defaultPerms.put(false, new HashSet());

        Field pmField = CraftServer.class.getDeclaredField("pluginManager");
        pmField.setAccessible(true);
        pmField.set(craftServer, pm);

        Field scmField = CraftServer.class.getDeclaredField("server");
        scmField.setAccessible(true);
        scmField.set(craftServer, scm);

        Field consoleField = CraftServer.class.getDeclaredField("console");
        consoleField.setAccessible(true);
        consoleField.set(craftServer, ms);

        Field serverField = Bukkit.class.getDeclaredField("server");
        serverField.setAccessible(true);
        serverField.set(null, craftServer);
    }

    public static WorldServer createWorld(long seed, File worldDir) throws Exception {
        MinecraftServer ms = (MinecraftServer) unsafe.allocateInstance(MinecraftServer.class);
        initObjectFields(ms, MinecraftServer.class);
        ms.worlds = new ArrayList<WorldServer>();
        setupBukkitServer(ms);

        WorldServer world = (WorldServer) unsafe.allocateInstance(WorldServer.class);
        initObjectFields(world, WorldServer.class);
        ms.worlds.add(world);

        Field serverField = WorldServer.class.getDeclaredField("server");
        serverField.setAccessible(true);
        serverField.set(world, ms);

        world.worldData = new WorldData(seed, "world");
        world.worldProvider = WorldProvider.byDimension(0);
        world.worldProvider.a(world);

        // The chunk provider of the server: the CraftBukkit world needs it
        // and it keeps the generated chunks in the world directory.
        ServerNBTManager nbt = new ServerNBTManager(worldDir, "world", true);
        Field nbtField = World.class.getDeclaredField("w");
        nbtField.setAccessible(true);
        nbtField.set(world, nbt);
        world.worldMaps = new WorldMapCollection(nbt);

        IChunkLoader loader = nbt.a(world.worldProvider);
        InternalChunkGenerator generator = new NormalChunkGenerator(world, seed);
        ChunkProviderServer provider = new ChunkProviderServer(world, loader, generator);
        world.chunkProvider = provider;
        world.chunkProviderServer = provider;
        // While the world is loading, CraftBukkit hands out the real chunks
        // instead of the empty chunk (ChunkProviderServer.getOrCreateChunk).
        world.isLoading = true;
        world.manager = new PlayerManager(ms, 0, 10);
        world.random = new Random(seed);
        // The entity tracker broadcasts the damage and the death packets.
        world.tracker = new EntityTracker(ms, 0);

        // The CraftBukkit world of the entities.
        CraftWorld craftWorld = (CraftWorld) unsafe.allocateInstance(CraftWorld.class);
        initObjectFields(craftWorld, CraftWorld.class);
        Field worldField = World.class.getDeclaredField("world");
        worldField.setAccessible(true);
        worldField.set(world, craftWorld);
        Field craftWorldField = CraftWorld.class.getDeclaredField("world");
        craftWorldField.setAccessible(true);
        craftWorldField.set(craftWorld, world);
        return world;
    }

    /** A player that stands still and is the target of the mob. */
    private static EntityPlayer createPlayer(WorldServer world, double x, double y, double z) throws Exception {
        ItemInWorldManager im = new ItemInWorldManager(world);
        EntityPlayer player = (EntityPlayer) unsafe.allocateInstance(EntityPlayer.class);
        initObjectFields(player, EntityPlayer.class);
        player.world = world;
        player.itemInWorldManager = im;
        im.player = player;
        player.name = "Player";
        player.displayName = "Player";
        // The constructor is bypassed, so the bounding box has to be created.
        setBoundingBox(player, AxisAlignedBB.a(0.0D, 0.0D, 0.0D, 0.0D, 0.0D, 0.0D));
        player.setLocation(x, y, z, 0.0F, 0.0F);
        player.d = player.locX;
        player.e = player.locZ;
        player.playerChunkCoordIntPairs = new HashSet();
        player.chunkCoordIntPairQueue = new LinkedList();
        // The constructor is bypassed: the id and the health have to be set,
        // a mob ignores a target without health.
        player.id = 1000;
        player.health = 20;
        world.addEntity(player);
        return player;
    }

    /** The bounding box of an entity is final, so it has to be written by reflection. */
    private static void setBoundingBox(Entity entity, AxisAlignedBB box) throws Exception {
        Field f = Entity.class.getDeclaredField("boundingBox");
        f.setAccessible(true);
        f.set(entity, box);
    }

    // ------------------------------------------------------------------
    // The scenarios
    // ------------------------------------------------------------------

    /** The terrain of a scenario and the mob that walks through it. */
    public static class Scenario {
        public final String name;
        public final String mobClass;
        public final double spawnX, spawnY, spawnZ;
        /** The player target or null. */
        public final double[] player;
        public final Terrain terrain;
        /** The floor of the scenario: the light above it is the sky light. */
        public final int floorY;
        public final int radius;

        Scenario(String name, String mobClass, double spawnX, double spawnY, double spawnZ,
                 double[] player, int floorY, int radius, Terrain terrain) {
            this.name = name;
            this.mobClass = mobClass;
            this.spawnX = spawnX;
            this.spawnY = spawnY;
            this.spawnZ = spawnZ;
            this.player = player;
            this.floorY = floorY;
            this.radius = radius;
            this.terrain = terrain;
        }
    }

    /** The terrain builder of a scenario. */
    public interface Terrain {
        void build(World world);
    }

    private static final int STONE = 1;
    private static final int GRASS = 2;
    private static final int WATER = 9;   // Block.STATIONARY_WATER
    private static final int LAVA = 11;   // Block.STATIONARY_LAVA

    /** A flat stone floor at `floorY` and air above it. */
    private static void flatFloor(World world, int floorY, int radius) {
        for (int x = -radius; x <= radius; x++) {
            for (int z = -radius; z <= radius; z++) {
                for (int y = floorY + 1; y <= floorY + 14; y++) {
                    world.setTypeId(x, y, z, 0);
                }
                world.setTypeId(x, floorY, z, GRASS);
            }
        }
    }

    private static final Map<String, Scenario> SCENARIOS = new LinkedHashMap<>();

    static {
        // A pig that wanders on a flat floor (the random walk of EntityCreature.B).
        SCENARIOS.put("flat", new Scenario("flat", "EntityPig", 0.5, 64.0, 0.5, null, 63, 24,
                world -> flatFloor(world, 63, 24)));

        // A zombie that climbs a one block step towards the player.
        SCENARIOS.put("chase", new Scenario("chase", "EntityZombie", 0.5, 64.0, 0.5,
                new double[]{10.5, 65.0, 0.5}, 63, 24, world -> {
                    flatFloor(world, 63, 24);
                    for (int x = 4; x <= 24; x++) {
                        for (int z = -12; z <= 12; z++) {
                            world.setTypeId(x, 64, z, STONE);
                        }
                    }
                }));

        // A zombie that walks into a pool of water.
        SCENARIOS.put("water", new Scenario("water", "EntityZombie", 0.5, 64.0, 0.5,
                new double[]{12.5, 64.0, 0.5}, 63, 24, world -> {
                    flatFloor(world, 63, 24);
                    for (int x = 2; x <= 8; x++) {
                        for (int z = -3; z <= 3; z++) {
                            world.setTypeId(x, 64, z, WATER);
                            world.setTypeId(x, 63, z, WATER);
                        }
                    }
                }));

        // A spider that climbs a wall towards the player.
        SCENARIOS.put("wall", new Scenario("wall", "EntitySpider", 0.5, 64.0, 0.5,
                new double[]{10.5, 64.0, 0.5}, 63, 24, world -> {
                    flatFloor(world, 63, 24);
                    for (int y = 64; y <= 67; y++) {
                        for (int z = -6; z <= 6; z++) {
                            world.setTypeId(4, y, z, STONE);
                        }
                    }
                }));

        // A zombie that walks into a pool of lava.
        SCENARIOS.put("lava", new Scenario("lava", "EntityZombie", 0.5, 64.0, 0.5,
                new double[]{12.5, 64.0, 0.5}, 63, 24, world -> {
                    flatFloor(world, 63, 24);
                    for (int x = 2; x <= 8; x++) {
                        for (int z = -3; z <= 3; z++) {
                            world.setTypeId(x, 64, z, LAVA);
                            world.setTypeId(x, 63, z, LAVA);
                        }
                    }
                }));

        // A slime that hops around.
        SCENARIOS.put("slime", new Scenario("slime", "EntitySlime", 0.5, 64.0, 0.5, null, 63, 24,
                world -> flatFloor(world, 63, 24)));

        // A squid that swims in a tank of water.
        SCENARIOS.put("squid", new Scenario("squid", "EntitySquid", 0.5, 66.0, 0.5, null, 60, 12, world -> {
            flatFloor(world, 60, 12);
            for (int x = -8; x <= 8; x++) {
                for (int z = -8; z <= 8; z++) {
                    for (int y = 61; y <= 74; y++) {
                        world.setTypeId(x, y, z, WATER);
                    }
                }
            }
        }));

        // A ghast that flies over a flat floor.
        SCENARIOS.put("ghast", new Scenario("ghast", "EntityGhast", 0.5, 70.0, 0.5, null, 63, 24,
                world -> flatFloor(world, 63, 24)));

        // A creeper that walks towards a player.
        SCENARIOS.put("creeper", new Scenario("creeper", "EntityCreeper", 0.5, 64.0, 0.5,
                new double[]{10.5, 64.0, 0.5}, 63, 24, world -> flatFloor(world, 63, 24)));

        // A skeleton that shoots at a player.
        SCENARIOS.put("skeleton", new Scenario("skeleton", "EntitySkeleton", 0.5, 64.0, 0.5,
                new double[]{10.5, 64.0, 0.5}, 63, 24, world -> flatFloor(world, 63, 24)));

        // A chicken that wanders on a flat floor.
        SCENARIOS.put("chicken", new Scenario("chicken", "EntityChicken", 0.5, 64.0, 0.5, null, 63, 24,
                world -> flatFloor(world, 63, 24)));

        // A cow that wanders on a flat floor.
        SCENARIOS.put("cow", new Scenario("cow", "EntityCow", 0.5, 64.0, 0.5, null, 63, 24,
                world -> flatFloor(world, 63, 24)));

        // A sheep that wanders on a flat floor.
        SCENARIOS.put("sheep", new Scenario("sheep", "EntitySheep", 0.5, 64.0, 0.5, null, 63, 24,
                world -> flatFloor(world, 63, 24)));

        // A wolf that walks towards a player.
        SCENARIOS.put("wolf", new Scenario("wolf", "EntityWolf", 0.5, 64.0, 0.5,
                new double[]{10.5, 64.0, 0.5}, 63, 24, world -> flatFloor(world, 63, 24)));

        // A pig zombie that walks towards a player.
        SCENARIOS.put("pigzombie", new Scenario("pigzombie", "EntityPigZombie", 0.5, 64.0, 0.5,
                new double[]{10.5, 64.0, 0.5}, 63, 24, world -> flatFloor(world, 63, 24)));

        // A giant zombie that walks towards a player.
        SCENARIOS.put("giant", new Scenario("giant", "EntityGiantZombie", 0.5, 64.0, 0.5,
                new double[]{10.5, 64.0, 0.5}, 63, 24, world -> flatFloor(world, 63, 24)));
    }

    /**
     * The light of a scenario: the sky light is 15 above the floor of the
     * scenario and 0 below it. The mobs score the blocks of their walk with
     * the light (`EntityCreature.B()` and `EntityAnimal.a`).
     */
    private static void scenarioLight(World world, Scenario scenario) {
        for (int x = -scenario.radius; x <= scenario.radius; x++) {
            for (int z = -scenario.radius; z <= scenario.radius; z++) {
                Chunk chunk = world.getChunkAt(x >> 4, z >> 4);
                for (int y = 0; y < 128; y++) {
                    chunk.f.a(x & 15, y, z & 15, y > scenario.floorY ? 15 : 0);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // The trace
    // ------------------------------------------------------------------

    private static EntityLiving createMob(World world, String className) throws Exception {
        switch (className) {
            case "EntityPig": return new EntityPig(world);
            case "EntityZombie": return new EntityZombie(world);
            case "EntitySpider": return new EntitySpider(world);
            case "EntitySlime": return new EntitySlime(world);
            case "EntitySquid": return new EntitySquid(world);
            case "EntityGhast": return new EntityGhast(world);
            case "EntityWolf": return new EntityWolf(world);
            case "EntityCreeper": return new EntityCreeper(world);
            case "EntitySkeleton": return new EntitySkeleton(world);
            case "EntityChicken": return new EntityChicken(world);
            case "EntityCow": return new EntityCow(world);
            case "EntitySheep": return new EntitySheep(world);
            case "EntityPigZombie": return new EntityPigZombie(world);
            case "EntityGiantZombie": return new EntityGiantZombie(world);
            default: throw new IllegalArgumentException("unknown mob " + className);
        }
    }

    /** The path of the mob, hashed so a different path is detected. */
    private static long pathHash(EntityLiving mob) {
        if (!(mob instanceof EntityCreature)) {
            return 0L;
        }
        EntityCreature creature = (EntityCreature) mob;
        PathEntity path = creature.pathEntity;
        if (path == null) {
            return 0L;
        }
        long hash = 0xcbf29ce484222325L;
        try {
            Field pointsField = PathEntity.class.getDeclaredField("b");
            pointsField.setAccessible(true);
            PathPoint[] points = (PathPoint[]) pointsField.get(path);
            Field indexField = PathEntity.class.getDeclaredField("c");
            indexField.setAccessible(true);
            int index = indexField.getInt(path);
            for (int i = 0; i < points.length; i++) {
                long v = ((long) points[i].a << 40) | ((long) points[i].b << 20) | points[i].c;
                if (i == index) {
                    v ^= 0x5DEECE66DL;
                }
                hash ^= v;
                hash *= 0x100000001b3L;
            }
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        return hash;
    }

    private static int pathLength(EntityLiving mob) {
        if (!(mob instanceof EntityCreature)) {
            return 0;
        }
        EntityCreature creature = (EntityCreature) mob;
        PathEntity path = creature.pathEntity;
        return path == null ? 0 : path.a;
    }

    /** The tick of the loop, the trace of the random prints it. */
    static int TICK = 0;

    private static void dump(PrintWriter out, int tick, EntityLiving mob) {
        if (System.getenv("FLINT_IDLE") != null) {
            try {
                Field fa = EntityLiving.class.getDeclaredField("a");
                fa.setAccessible(true);
                System.err.printf(java.util.Locale.ROOT, "IDLE %d a=%d%n", tick, fa.getInt(mob));
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        if (System.getenv("FLINT_HEALTH") != null) {
            System.err.printf(java.util.Locale.ROOT, "HEALTH %d health=%d noDamage=%d lastDamage=%d fire=%d%n",
                tick, mob.health, mob.noDamageTicks, mob.lastDamage, mob.fireTicks);
        }
        if (System.getenv("FLINT_JUMP") != null) {
            try {
                Field ac = EntityLiving.class.getDeclaredField("aC");
                ac.setAccessible(true);
                Field ay = EntityLiving.class.getDeclaredField("ay");
                ay.setAccessible(true);
                System.err.printf(java.util.Locale.ROOT,
                    "JUMP %d ad=%s ae=%s aC=%s ac=%s ay=%d motY=%.9f y=%.9f onGround=%s fire=%d%n",
                    tick, mob.ad(), mob.ae(), ac.getBoolean(mob), mob.ac(),
                    ay.getInt(mob), mob.motY, mob.locY,
                    mob.onGround, mob.fireTicks);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        if (System.getenv("FLINT_WATER") != null) {
            System.err.printf(java.util.Locale.ROOT, "WATER %d ad=%s f_=%s y=%.4f br=%.4f box=%s type=%d data=%d%n",
                tick, mob.ad(), mob.f_(), mob.locY, mob.br, mob.boundingBox.toString(),
                mob.world.getTypeId(2, 64, 0), mob.world.getData(2, 64, 0));
        }
        if (System.getenv("FLINT_PRECISE") != null) {
            System.err.printf(java.util.Locale.ROOT,
                "PRECISE %d x=%.10f y=%.10f z=%.10f motX=%.10f motY=%.10f motZ=%.10f yaw=%.10f pitch=%.10f%n",
                tick, mob.locX, mob.locY, mob.locZ, mob.motX, mob.motY, mob.motZ, mob.yaw, mob.pitch);
        }
        if (System.getenv("FLINT_BLOCK_DEBUG") != null) {
            StringBuilder sb = new StringBuilder();
            sb.append("BLOCK ").append(tick).append(" ");
            int bx = (int) Math.floor(mob.locX);
            int bz = (int) Math.floor(mob.locZ);
            for (int x = bx - 6; x <= bx + 6; x++) {
                for (int z = bz - 6; z <= bz + 6; z++) {
                    sb.append(x).append(",").append(z).append(":");
                    for (int y = 55; y <= 66; y++) {
                        sb.append(mob.world.getTypeId(x, y, z)).append(",");
                    }
                    sb.append(" ");
                }
            }
            System.err.println(sb.toString());
        }
        if (System.getenv("FLINT_LIGHT_DEBUG") != null) {
            StringBuilder sb = new StringBuilder();
            sb.append("LIGHT ").append(tick).append(" ");
            int bx = (int) Math.floor(mob.locX);
            int by = (int) Math.floor(mob.locY);
            int bz = (int) Math.floor(mob.locZ);
            for (int y = by - 3; y <= by + 3; y++) {
                for (int z = bz - 6; z <= bz + 6; z++) {
                    for (int x = bx - 6; x <= bx + 6; x++) {
                        net.minecraft.server.Chunk chunk = mob.world.getChunkAt(x >> 4, z >> 4);
                        sb.append(x).append(",").append(y).append(",").append(z).append("=")
                          .append((int) (mob.world.n(x, y, z) * 100.0F))
                          .append("/").append(chunk.f.a(x & 15, y, z & 15))
                          .append("/").append(chunk.g.a(x & 15, y, z & 15))
                          .append(" ");
                    }
                }
            }
            System.err.println(sb.toString());
        }
        if (System.getenv("FLINT_PATH_DEBUG") != null) {
            StringBuilder sb = new StringBuilder();
            sb.append("PATH ").append(tick).append(" ");
            if (mob instanceof EntityCreature) {
                PathEntity path = ((EntityCreature) mob).pathEntity;
                if (path != null) {
                    try {
                        Field pointsField = PathEntity.class.getDeclaredField("b");
                        pointsField.setAccessible(true);
                        PathPoint[] points = (PathPoint[]) pointsField.get(path);
                        Field indexField = PathEntity.class.getDeclaredField("c");
                        indexField.setAccessible(true);
                        int index = indexField.getInt(path);
                        for (int i = 0; i < points.length; i++) {
                            sb.append("[").append(points[i].a).append(",").append(points[i].b)
                              .append(",").append(points[i].c).append(i == index ? "*" : "").append("]");
                        }
                    } catch (Exception e) {
                        sb.append("ERR").append(e);
                    }
                }
            }
            System.err.println(sb.toString());
        }
        int target = -1;
        if (mob instanceof EntityCreature) {
            Entity t = ((EntityCreature) mob).target;
            if (t != null) {
                target = t.id;
            }
        }
        out.printf(Locale.ROOT, "%d %.9f %.9f %.9f %.9f %.9f %.9f %.7f %.7f %d %.7f %d %d %016x%n",
                tick,
                mob.locX, mob.locY, mob.locZ,
                mob.motX, mob.motY, mob.motZ,
                mob.yaw, mob.pitch,
                mob.onGround ? 1 : 0,
                mob.fallDistance,
                target,
                pathLength(mob),
                pathHash(mob));
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 3) {
            System.err.println("Usage: PoseidonMobTrace <scenario> <seed> <out> [ticks]");
            System.err.println("Scenarios: " + SCENARIOS.keySet());
            System.exit(2);
        }
        String scenarioName = args[0];
        long seed = Long.parseLong(args[1]);
        String outPath = args[2];
        int ticks = args.length > 3 ? Integer.parseInt(args[3]) : 300;

        Scenario scenario = SCENARIOS.get(scenarioName);
        if (scenario == null) {
            System.err.println("unknown scenario " + scenarioName);
            System.err.println("Scenarios: " + SCENARIOS.keySet());
            System.exit(2);
        }

        // Poseidon shares one Random between all entities; the first setSeed()
        // call wins and locks the stream.
        // The shared random of the entities (`Entity.SHARED_RANDOM`). The
        // reference takes the seed from the system clock, the harness sets it
        // so the traces are reproducible.
        final boolean trace = System.getProperty("flint.trace.rng") != null;
        Entity.SHARED_RANDOM = new Random(seed) {
            @Override
            protected int next(int bits) {
                if (trace) {
                    int value = super.next(bits);
                    StackTraceElement[] frames = new Throwable().getStackTrace();
                    StringBuilder where = new StringBuilder();
                    for (int i = 2; i < Math.min(frames.length, 6); i++) {
                        where.append(frames[i].getClassName())
                            .append('#')
                            .append(frames[i].getMethodName())
                            .append(':')
                            .append(frames[i].getLineNumber())
                            .append(" <- ");
                    }
                    System.out.println("RNG t=" + TICK + " " + bits + " " + value + " " + where);
                    return value;
                }
                return super.next(bits);
            }
        };

        File worldDir = new File(System.getProperty("java.io.tmpdir"),
                "poseidon_mob_world_" + scenarioName + "_" + seed);
        worldDir.mkdirs();
        WorldServer world = createWorld(seed, worldDir);
        scenario.terrain.build(world);
        // The light of the scenario (the mobs score the light of a block).
        while (world.doLighting()) {
            ;
        }
        // The light of the scenario is set by hand: the world of the harness
        // is generated with the normal generator, so the light of the terrain
        // would depend on the hills around the floor. The port builds the
        // same floor, so both worlds need the same light.
        scenarioLight(world, scenario);

        if (System.getenv("FLINT_LIGHT_DEBUG") != null) {
            int[][] cands = {{-5, 66, -2}, {-5, 64, -5}, {0, 67, 1}, {0, 64, -6}, {-1, 66, 0},
                {-6, 62, 2}, {-4, 67, 0}, {-4, 65, -3}, {6, 64, 0}, {-2, 67, 2}};
            for (int[] c : cands) {
                System.out.println("LIGHT " + c[0] + "," + c[1] + "," + c[2]
                        + " below=" + world.getTypeId(c[0], c[1] - 1, c[2])
                        + " level=" + world.getLightLevel(c[0], c[1], c[2])
                        + " n=" + world.n(c[0], c[1], c[2])
                        + " blockLight=" + world.getChunkAt(c[0] >> 4, c[2] >> 4).f.a(c[0] & 15, c[1], c[2] & 15)
                        + " skyLight=" + world.getChunkAt(c[0] >> 4, c[2] >> 4).g.a(c[0] & 15, c[1], c[2] & 15)
                        + " type=" + world.getTypeId(c[0], c[1], c[2]));
            }
        }

        // A sanity check of the scenario terrain.
        int floorId = world.getTypeId((int) Math.floor(scenario.spawnX), (int) Math.floor(scenario.spawnY) - 1, (int) Math.floor(scenario.spawnZ));
        if (floorId == 0 && !scenario.name.equals("squid")) {
            System.out.printf("WARNING: the floor under the spawn of %s is air (id 0)%n", scenario.name);
        }

        EntityLiving mob = createMob(world, scenario.mobClass);
        mob.setLocation(scenario.spawnX, scenario.spawnY, scenario.spawnZ, 0.0F, 0.0F);
        if (scenario.player != null) {
            createPlayer(world, scenario.player[0], scenario.player[1], scenario.player[2]);
        }
        world.addEntity(mob);

        File outFile = new File(outPath);
        File parent = outFile.getParentFile();
        if (parent != null) {
            parent.mkdirs();
        }
        try (PrintWriter out = new PrintWriter(new BufferedWriter(new FileWriter(outFile)))) {
            boolean in_world = true;
            out.printf("# scenario=%s seed=%d ticks=%d mob=%s%n",
                    scenario.name, seed, ticks, scenario.mobClass);
            out.printf("# tick x y z motX motY motZ yaw pitch onGround fallDistance target pathLen pathHash%n");
            for (int t = 0; t < ticks; t++) {
                if (System.getenv("FLINT_SQUID_DEBUG") != null && t >= 50 && t < 55) {
                    System.out.printf(
                        "SDEBUG t=%d pre motY=%.9f ad=%s ae=%s box=[%.3f,%.3f,%.3f]-[%.3f,%.3f,%.3f]%n",
                        t, mob.motY, mob.ad(), mob.ae(),
                        mob.boundingBox.a, mob.boundingBox.b, mob.boundingBox.c,
                        mob.boundingBox.d, mob.boundingBox.e, mob.boundingBox.f);
                }
                if (System.getenv("FLINT_ZOMBIE_DEBUG") != null && t < 3) {
                    System.out.printf(
                        "ZDEBUG t=%d world.d=%s light=%.7f worldTime=%d loaded=%s%n",
                        t, world.d(), mob.c(1.0F), world.worldData.f(),
                        world.isChunkLoaded(MathHelper.floor(mob.locX),
                                MathHelper.floor(mob.locY),
                                MathHelper.floor(mob.locZ)));
                }
                TICK = t;
                // The world removes the dead entities at the end of its tick
                // (`World.tickEntities`), so the mob is not updated any more
                // once it left the world.
                if (in_world) {
                    mob.m_();
                    if (mob.deathTicks > 20) {
                        // `EntityLiving.die()`: the world removes the entity
                        // at the end of its tick.
                        in_world = false;
                    }
                }
                if (System.getenv("FLINT_SQUID_DEBUG") != null && t >= 50 && t < 55) {
                    System.out.printf("SDEBUG t=%d post motY=%.9f ad=%s%n", t, mob.motY, mob.ad());
                }
                dump(out, t, mob);
            }
        }
        System.out.printf("%s: %d ticks written to %s%n", scenario.name, ticks, outPath);
    }
}
