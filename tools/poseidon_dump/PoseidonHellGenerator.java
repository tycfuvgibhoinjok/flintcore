package flintcore.test;

import java.io.*;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.*;
import sun.misc.Unsafe;
import net.minecraft.server.*;

/**
 * Nether ground-truth dumper.
 *
 * Mirrors `flintcore::world::world::World::populate_chunk` for `DIM-1`:
 * for every target chunk in the requested region (z-major, x-minor) it first
 * ensures the 5x5 base-chunk neighbourhood (dz outer, dx inner), then runs
 * `ChunkProviderHell.getChunkAt(IChunkProvider, i, j)`.
 *
 * The `ChunkProviderHell` instance is stateful: `getChunkAt(populate)` does
 * NOT reseed its `Random`, so the exact call order matters and must match the
 * Rust side byte for byte.
 *
 * Dump format per chunk: 32768 block bytes (wire order), 16384 nibble-packed
 * metadata bytes, 256 height-map bytes.
 */
public class PoseidonHellGenerator {
    public static class SimpleHellProvider implements IChunkProvider {
        public final World world;
        public final ChunkProviderHell cpg;
        public final Map<Long, Chunk> chunks = new HashMap<>();

        public SimpleHellProvider(World world) {
            this.world = world;
            this.cpg = new ChunkProviderHell(world, world.getSeed());
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
        world.worldProvider = WorldProvider.byDimension(-1);
        world.worldProvider.a(world);

        SimpleHellProvider provider = new SimpleHellProvider(world);
        world.chunkProvider = provider;
        return world;
    }

    public static void populateRegion(World world, int minCx, int maxCx, int minCz, int maxCz) {
        SimpleHellProvider provider = (SimpleHellProvider) world.chunkProvider;
        for (int cz = minCz; cz <= maxCz; cz++) {
            for (int cx = minCx; cx <= maxCx; cx++) {
                // World.populate_chunk: ensure 5x5 base chunks, dz outer/dx inner.
                for (int dz = -2; dz <= 2; dz++) {
                    for (int dx = -2; dx <= 2; dx++) {
                        provider.getOrCreateChunk(cx + dx, cz + dz);
                    }
                }
                provider.getChunkAt(provider, cx, cz);
            }
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 6) {
            System.err.println("Usage: PoseidonHellGenerator <seed> <out_dir> <min_cx> <max_cx> <min_cz> <max_cz>");
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
                    dos.write(chunk.b);
                    dos.write(chunk.e.a);
                    dos.write(chunk.heightMap);
                }
                System.out.printf("Exported chunk (%d, %d) for seed %d to %s%n", cx, cz, seed, outFile.getPath());
            }
        }
    }
}
