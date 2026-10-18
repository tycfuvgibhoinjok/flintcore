package flintcore.test;

import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.*;
import sun.misc.Unsafe;
import net.minecraft.server.*;

public class PoseidonGenerator {
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
                // Ensure 3x3 neighbor chunks are generated before populating (i, j)
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
        public boolean saveChunks(boolean flag, IProgressUpdate progress) { return true; }
        @Override
        public boolean unloadChunks() { return false; }
        @Override
        public boolean canSave() { return true; }
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

    public static World createWorld(long seed) throws Exception {
        Field f = Unsafe.class.getDeclaredField("theUnsafe");
        f.setAccessible(true);
        Unsafe unsafe = (Unsafe) f.get(null);

        World world = (World) unsafe.allocateInstance(WorldServer.class);
        initObjectFields(world, WorldServer.class);

        world.worldData = new WorldData(seed, "world");
        world.worldProvider = WorldProvider.byDimension(0);
        world.worldProvider.a(world);

        SimpleChunkProvider provider = new SimpleChunkProvider(world);
        world.chunkProvider = provider;
        return world;
    }

    public static void populateRegion(World world, int minCx, int maxCx, int minCz, int maxCz) {
        SimpleChunkProvider provider = (SimpleChunkProvider) world.chunkProvider;
        // Pre-generate base chunks in range with border +1
        for (int cz = minCz - 1; cz <= maxCz + 1; cz++) {
            for (int cx = minCx - 1; cx <= maxCx + 1; cx++) {
                provider.getOrCreateChunk(cx, cz);
            }
        }
        // Populate in order
        for (int cz = minCz; cz <= maxCz; cz++) {
            for (int cx = minCx; cx <= maxCx; cx++) {
                provider.getChunkAt(provider, cx, cz);
            }
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 6) {
            System.err.println("Usage: PoseidonGenerator <seed> <out_dir> <min_cx> <max_cx> <min_cz> <max_cz>");
            System.exit(1);
        }
        long seed = Long.parseLong(args[0]);
        File outDir = new File(args[1]);
        outDir.mkdirs();
        int minCx = Integer.parseInt(args[2]);
        int maxCx = Integer.parseInt(args[3]);
        int minCz = Integer.parseInt(args[4]);
        int maxCz = Integer.parseInt(args[5]);

        World world = createWorld(seed);
        populateRegion(world, minCx, maxCx, minCz, maxCz);

        for (int cz = minCz; cz <= maxCz; cz++) {
            for (int cx = minCx; cx <= maxCx; cx++) {
                Chunk chunk = world.getChunkAt(cx, cz);
                File outFile = new File(outDir, String.format("chunk_%d_%d_%d.bin", seed, cx, cz));
                try (DataOutputStream dos = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(outFile)))) {
                    // Write 32768 bytes block IDs
                    dos.write(chunk.b);
                    // Write 16384 bytes metadata
                    dos.write(chunk.e.a);
                    // Write 256 bytes heightMap
                    dos.write(chunk.heightMap);
                }
                System.out.printf("Exported chunk (%d, %d) for seed %d to %s%n", cx, cz, seed, outFile.getPath());
            }
        }
    }
}
