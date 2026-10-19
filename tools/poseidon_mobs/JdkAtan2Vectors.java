import java.util.Random;

/**
 * Prints the values of `Math.atan2` of the JVM, so that the port of the
 * `fdlibm` library can be checked against them.
 *
 * The pairs of the arguments are produced with `java.util.Random`, the same
 * generator the port uses, so that the test of the port can reproduce them.
 */
public final class JdkAtan2Vectors {
    public static void main(String[] args) {
        Random random = new Random(0x5EED5EEDL);
        for (int i = 0; i < 64; i++) {
            double y = (random.nextDouble() - 0.5) * 40.0;
            double x = (random.nextDouble() - 0.5) * 40.0;
            double atan2 = Math.atan2(y, x);
            System.out.printf(
                    "%016x %016x %016x%n",
                    Double.doubleToRawLongBits(y),
                    Double.doubleToRawLongBits(x),
                    Double.doubleToRawLongBits(atan2));
        }
    }
}
