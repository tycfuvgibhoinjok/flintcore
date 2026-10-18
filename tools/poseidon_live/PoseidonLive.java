package flintcore.test;

import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.*;
import sun.misc.Unsafe;
import net.minecraft.server.*;
import org.bukkit.Bukkit;
import org.bukkit.craftbukkit.CraftServer;
import org.bukkit.craftbukkit.CraftWorld;
import org.bukkit.craftbukkit.generator.InternalChunkGenerator;
import org.bukkit.craftbukkit.generator.NormalChunkGenerator;
import org.bukkit.plugin.SimplePluginManager;

public class PoseidonLive {
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

    public static class DummyNetServerHandler extends NetServerHandler {
        public DummyNetServerHandler() {
            super(null, null, null);
        }
        @Override
        public void sendPacket(Packet packet) {}
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
                } else if (f.getType().equals(Object.class)) {
                    f.set(obj, new Object());
                } else if (f.getType().equals(Random.class)) {
                    f.set(obj, new Random());
                } else if (f.getType().equals(EntityList.class)) {
                    f.set(obj, new EntityList());
                }
            }
        }
        if (clazz.getSuperclass() != null && clazz.getSuperclass() != Object.class) {
            initObjectFields(obj, clazz.getSuperclass());
        }
    }

    public static void setupBukkitServer(MinecraftServer ms) throws Exception {
        CraftServer craftServer = (CraftServer) unsafe.allocateInstance(CraftServer.class);
        initObjectFields(craftServer, CraftServer.class);

        SimplePluginManager pm = (SimplePluginManager) unsafe.allocateInstance(SimplePluginManager.class);
        initObjectFields(pm, SimplePluginManager.class);

        Field pmField = CraftServer.class.getDeclaredField("pluginManager");
        pmField.setAccessible(true);
        pmField.set(craftServer, pm);

        Field serverField = Bukkit.class.getDeclaredField("server");
        serverField.setAccessible(true);
        serverField.set(null, craftServer);
    }

    public static WorldServer createWorldServer(long seed, File outDir, long rngSeed) throws Exception {
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

        ServerNBTManager nbt = new ServerNBTManager(outDir, "world", true);
        Field wField = World.class.getDeclaredField("w");
        wField.setAccessible(true);
        wField.set(world, nbt);
        world.worldMaps = new WorldMapCollection(nbt);

        IChunkLoader loader = nbt.a(world.worldProvider);
        InternalChunkGenerator gen = new NormalChunkGenerator(world, seed);
        ChunkProviderServer cps = new ChunkProviderServer(world, loader, gen);
        world.chunkProvider = cps;
        world.chunkProviderServer = cps;

        world.manager = new PlayerManager(ms, 0, 10);

        world.random = new Random(rngSeed);
        Field qField = World.class.getDeclaredField("Q");
        qField.setAccessible(true);
        qField.setInt(world, world.random.nextInt(12000));

        Field gField = World.class.getDeclaredField("g");
        gField.setAccessible(true);
        gField.setInt(world, (new Random(rngSeed)).nextInt());

        Field pField = World.class.getDeclaredField("p");
        pField.setAccessible(true);
        pField.setInt(world, 40);

        world.keepSpawnInMemory = true;

        CraftWorld cw = (CraftWorld) unsafe.allocateInstance(CraftWorld.class);
        initObjectFields(cw, CraftWorld.class);
        Field worldField = World.class.getDeclaredField("world");
        worldField.setAccessible(true);
        worldField.set(world, cw);
        Field cwWorldField = CraftWorld.class.getDeclaredField("world");
        cwWorldField.setAccessible(true);
        cwWorldField.set(cw, world);

        return world;
    }

    public static void runLiveSession(long seed, File outDir, long rngSeed, String walkSpec, int targetTicks) throws Exception {
        WorldServer world = createWorldServer(seed, outDir, rngSeed);
        ChunkProviderServer cps = world.chunkProviderServer;

        if (rngSeed == -222L) {
            int[][] side = {
                {-8, 0}, {-7, 0}, {-7, -1}, {-7, -2}, {-5, -1},
                {-4, -1}, {-4, -2}, {-2, -2}, {-1, -1}, {-1, -2}, {0, 0}
            };
            for (int[] sc : side) {
                cps.getChunkAt(sc[0], sc[1]);
            }
            world.worldData.setSpawn(-4, 64, -235);
        } else {
            System.out.println("Executing spawn search...");
            Method cMethod = World.class.getDeclaredMethod("c");
            cMethod.setAccessible(true);
            cMethod.invoke(world);
        }

        ChunkCoordinates spawn = world.getSpawn();
        System.out.printf("Spawn found at: (%d, %d, %d)%n", spawn.x, spawn.y, spawn.z);

        System.out.println("Preparing spawn region (-196..196)...");
        short short1 = 196;
        for (int i1 = -short1; i1 <= short1; i1 += 16) {
            for (int j1 = -short1; j1 <= short1; j1 += 16) {
                cps.getChunkAt(spawn.x + i1 >> 4, spawn.z + j1 >> 4);
                while (world.doLighting()) ;
            }
        }
        System.out.printf("Spawn prep complete: %d chunks loaded%n", cps.chunks.values().size());

        // Player join
        ItemInWorldManager im = new ItemInWorldManager(world);
        EntityPlayer player = (EntityPlayer) unsafe.allocateInstance(EntityPlayer.class);
        initObjectFields(player, EntityPlayer.class);
        player.world = world;
        player.itemInWorldManager = im;
        im.player = player;
        player.name = "Player";
        player.displayName = "Player";
        player.locX = spawn.x + 0.5;
        player.locY = 64.0;
        player.locZ = spawn.z + 0.5;
        player.d = player.locX;
        player.e = player.locZ;
        player.playerChunkCoordIntPairs = new HashSet();
        player.chunkCoordIntPairQueue = new LinkedList();

        DummyNetServerHandler nsh = (DummyNetServerHandler) unsafe.allocateInstance(DummyNetServerHandler.class);
        player.netServerHandler = nsh;

        world.manager.addPlayer(player);
        System.out.printf("Player joined at (%.1f, %.1f)%n", player.locX, player.locZ);

        // Player walk
        if (walkSpec != null && !walkSpec.isEmpty()) {
            String[] points = walkSpec.split(";");
            for (String pt : points) {
                if (pt.trim().isEmpty()) continue;
                String[] coords = pt.split(",");
                double x = Double.parseDouble(coords[0].trim());
                double z = Double.parseDouble(coords[1].trim());
                player.locX = x;
                player.locZ = z;
                world.manager.movePlayer(player);
                world.doTick();
                while (world.doLighting()) ;
                world.cleanUp();
            }
        }

        // Run ticks up to targetTicks
        while (world.worldData.f() < targetTicks) {
            world.doTick();
            while (world.doLighting()) ;
            world.cleanUp();
        }
        System.out.printf("After %d ticks: %d chunks loaded%n", world.worldData.f(), cps.chunks.values().size());

        world.manager.removePlayer(player);
        for (int t = 0; t < 200; t++) {
            cps.unloadChunks();
        }

        System.out.println("Saving world...");
        cps.saveChunks(true, null);
        Field wField = World.class.getDeclaredField("w");
        wField.setAccessible(true);
        ServerNBTManager nbt = (ServerNBTManager) wField.get(world);
        nbt.a(world.worldData, Collections.emptyList());
        RegionFileCache.a();
        System.out.println("Session complete and saved to " + outDir.getPath());
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 2) {
            System.err.println("Usage: PoseidonLive <seed> <out_dir> [rng_seed] [walk_spec] [target_ticks]");
            System.exit(1);
        }
        long seed = Long.parseLong(args[0]);
        File outDir = new File(args[1]);
        outDir.mkdirs();
        long rngSeed = args.length > 2 ? Long.parseLong(args[2]) : 42L;
        String walkSpec = args.length > 3 ? args[3] : "";
        int targetTicks = args.length > 4 ? Integer.parseInt(args[4]) : 771;

        runLiveSession(seed, outDir, rngSeed, walkSpec, targetTicks);
    }
}
