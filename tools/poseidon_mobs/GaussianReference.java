package flintcore.test;

import java.util.Random;

/**
 * The reference values of the normal distribution of the standard library of
 * Java (`java.util.Random.nextGaussian`) and of the elementary functions of
 * `StrictMath` that it uses.
 *
 * The port has to reproduce the values bit by bit: the gaussian is used for
 * the spread of the arrows, the fireballs and the eggs and for the velocity of
 * the items of the chests, the furnaces and the dispensers.
 *
 * The output is the test data of the tests of `src/util/rng.rs`.
 */
public final class GaussianReference {

    public static void main(String[] args) {
        // The normal distribution of a seeded stream.
        Random random = new Random(12345);
        System.out.println("# gaussian seed=12345 count=16");
        for (int i = 0; i < 16; i++) {
            System.out.printf("g %016x%n", Double.doubleToRawLongBits(random.nextGaussian()));
        }

        // The elementary functions of the polar method.
        System.out.println("# strictmath");
        Random values = new Random(987654321L);
        for (int i = 0; i < 64; i++) {
            double x, y, s;
            do {
                x = values.nextDouble() * 2.0 - 1.0;
                y = values.nextDouble() * 2.0 - 1.0;
                s = x * x + y * y;
            } while (s >= 1.0 || s == 0.0);
            System.out.printf(
                    "l %016x %016x%n",
                    Double.doubleToRawLongBits(StrictMath.log(s)),
                    Double.doubleToRawLongBits(StrictMath.sqrt(-2.0 * StrictMath.log(s) / s)));
        }

        // The logarithm of the whole range of the mantissa.
        System.out.println("# log");
        for (int i = 0; i < 256; i++) {
            double x = (i + 0.5) / 256.0 * 8.0;
            System.out.printf(
                    "L %016x %016x%n",
                    Double.doubleToRawLongBits(StrictMath.log(x)),
                    Double.doubleToRawLongBits(StrictMath.log(x * 1.0e-300)));
        }

        // The square root of the whole range of the mantissa.
        System.out.println("# sqrt");
        for (int i = 0; i < 256; i++) {
            double x = (i + 0.5) / 256.0 * 1024.0;
            System.out.printf(
                    "S %016x %016x%n",
                    Double.doubleToRawLongBits(StrictMath.sqrt(x)),
                    Double.doubleToRawLongBits(StrictMath.sqrt(x * 1.0e-8)));
        }
    }
}
