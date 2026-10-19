package flintcore.test;

import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.*;
import sun.misc.Unsafe;
import net.minecraft.server.*;
import org.bukkit.Bukkit;
import org.bukkit.craftbukkit.CraftServer;
import org.bukkit.craftbukkit.generator.InternalChunkGenerator;
import org.bukkit.craftbukkit.generator.NormalChunkGenerator;
import org.bukkit.plugin.SimplePluginManager;

/**
 * Nether live-order ground truth.
 *
 * Loads chunks through `ChunkProviderServer.getChunkAt` in the exact order
 * supplied on the command line (`x,z;x,z;...`), which exercises the 2x2
 * populate trigger block and the stateful `ChunkProviderHell` RNG. Dumps every
 * loaded chunk in the same wire format as the other dumpers.
 */
public class PoseidonHellLive {
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
                }
            }
        }
        if (clazz.getSuperclass() != null && clazz.getSuperclass() != Object.class) {
            initObjectFields(obj, clazz.getSuperclass());
        }
    }

    private static void setupBukkitServer(MinecraftServer ms) throws Exception {
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

    public static WorldServer createWorldServer(long seed, File outDir) throws Exception {
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
        world.worldProvider = WorldProvider.byDimension(-1);
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
        world.keepSpawnInMemory = false;
        return world;
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 3) {
            System.err.println("Usage: PoseidonHellLive <seed> <out_dir> <x,z;x,z;...>");
            System.exit(1);
        }
        long seed = Long.parseLong(args[0]);
        File outDir = new File(args[1]);
        outDir.mkdirs();
        String[] requests = args[2].split(";");

        WorldServer world = createWorldServer(seed, outDir);
        ChunkProviderServer cps = world.chunkProviderServer;
        for (String req : requests) {
            if (req.trim().isEmpty()) continue;
            String[] xy = req.split(",");
            int cx = Integer.parseInt(xy[0].trim());
            int cz = Integer.parseInt(xy[1].trim());
            cps.getChunkAt(cx, cz);
        }

        List<Chunk> chunks = new ArrayList<Chunk>(cps.chunks.values());
        chunks.sort(Comparator.comparingInt((Chunk c) -> c.x).thenComparingInt(c -> c.z));
        for (Chunk chunk : chunks) {
            File outFile = new File(outDir, String.format("chunk_%d_%d.bin", chunk.x, chunk.z));
            try (DataOutputStream dos = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(outFile)))) {
                dos.write(chunk.b);
                dos.write(chunk.e.a);
                dos.write(chunk.heightMap);
            }
            System.out.printf("loaded (%d, %d) done=%s%n", chunk.x, chunk.z, chunk.done);
        }
    }
}
