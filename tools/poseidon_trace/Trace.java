package flintcore.test;

public class Trace {
    public static boolean enabled = false;
    public static boolean armed = false;
    public static int x0, y0, z0, x1, y1, z1;

    static {
        String s = System.getProperty("flint.trace");
        if (s != null && !s.isEmpty()) {
            String[] p = s.split(",");
            enabled = true;
            x0 = Integer.parseInt(p[0].trim());
            y0 = Integer.parseInt(p[1].trim());
            z0 = Integer.parseInt(p[2].trim());
            x1 = Integer.parseInt(p[3].trim());
            y1 = Integer.parseInt(p[4].trim());
            z1 = Integer.parseInt(p[5].trim());
        }
    }

    private static boolean in(int x, int y, int z) {
        return enabled && armed && x >= x0 && x <= x1 && y >= y0 && y <= y1 && z >= z0 && z <= z1;
    }

    private static boolean inG(int x, int z) {
        return enabled && armed && x >= x0 && x <= x1 && z >= z0 && z <= z1;
    }

    public static void a4(int x, int y, int z, int old, int nw) {
        if (in(x, y, z)) System.out.println("A4 " + x + " " + y + " " + z + " " + old + "->" + nw);
    }

    public static void a5(int x, int y, int z, int old, int oldd, int nw, int nwd) {
        if (in(x, y, z)) System.out.println("A5 " + x + " " + y + " " + z + " " + old + "." + oldd + "->" + nw + "." + nwd);
    }

    public static void b(int x, int y, int z, int oldd, int nwd) {
        if (in(x, y, z)) System.out.println("B " + x + " " + y + " " + z + " " + oldd + "->" + nwd);
    }

    public static void g(int x, int z, int oldh, int newh) {
        if (inG(x, z)) System.out.println("G " + x + " " + z + " " + oldh + "->" + newh);
    }

    public static void phys(int x, int y, int z, int l, int id) {
        if (in(x, y, z)) System.out.println("PHYS " + x + " " + y + " " + z + " l=" + l + " id=" + id);
    }

    public static void j(int x, int y, int z, int c0, int c1, int c2, int c3, boolean b0, boolean b1, boolean b2, boolean b3) {
        if (in(x, y, z)) System.out.println("J " + x + " " + y + " " + z + " c=" + c0 + "," + c1 + "," + c2 + "," + c3 + " b=" + (b0?1:0) + (b1?1:0) + (b2?1:0) + (b3?1:0));
    }

    public static void flow(int x, int y, int z, int l) {
        if (in(x, y, z)) System.out.println("FLOW " + x + " " + y + " " + z + " l=" + l);
    }

    public static void down(int x, int y, int z) {
        if (in(x, y, z)) System.out.println("DOWN " + x + " " + y + " " + z);
    }

    public static void rng(int x, int y, int z) {
        if (in(x, y, z)) System.out.println("RNG " + x + " " + y + " " + z);
    }

    public static void rng2(int x, int y, int z) {
        if (in(x, y, z)) System.out.println("RNG2 " + x + " " + y + " " + z);
    }

    public static void rng3(int x, int y, int z) {
        if (in(x, y, z)) System.out.println("RNG3 " + x + " " + y + " " + z);
    }

    public static void fa(int x, int y, int z, int id, int data) {
        if (in(x, y, z)) System.out.println("FA " + x + " " + y + " " + z + " id=" + id + " d=" + data);
    }

    public static void conv(int x, int y, int z, int oldd, int nw) {
        if (in(x, y, z)) System.out.println("CONV " + x + " " + y + " " + z + " d=" + oldd + " ->" + nw);
    }
}
