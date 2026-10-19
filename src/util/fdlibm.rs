//! The `fdlibm` implementation of `atan` and `atan2`.
//!
//! The JVM computes `Math.atan2` with the `fdlibm` library (the `StrictMath`
//! one), and its result differs from the result of the system library in the
//! last bit. The port has to use the same algorithm, otherwise the angles of
//! the mobs differ by one unit in the last place.
//!
//! The code is a direct translation of the `s_atan.c` and `e_atan2.c` files of
//! the `libfdlibm` of the OpenJDK.

#[allow(clippy::excessive_precision, clippy::approx_constant)]
const ATANHI: [f64; 4] = [
    4.63647609000806093515e-01,
    7.85398163397448278999e-01,
    9.82793723247329054082e-01,
    1.57079632679489655800e+00,
];

#[allow(clippy::excessive_precision)]
const ATANLO: [f64; 4] = [
    2.26987774529616870924e-17,
    3.06161699786838301793e-17,
    1.39033110312309984516e-17,
    6.12323399573676603587e-17,
];

#[allow(clippy::excessive_precision)]
const AT: [f64; 11] = [
    3.33333333333329318027e-01,
    -1.99999999998764832476e-01,
    1.42857142725034663711e-01,
    -1.11111104054623557880e-01,
    9.09088713343650656196e-02,
    -7.69187620504482999495e-02,
    6.66107313738753120669e-02,
    -5.83357013379057348645e-02,
    4.97687799461593236017e-02,
    -3.65315727442169155270e-02,
    1.62858201153657823623e-02,
];

#[allow(clippy::excessive_precision, clippy::approx_constant)]
const TINY: f64 = 1.0e-300;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
const PI_O_4: f64 = 7.8539816339744827900E-01;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
const PI_O_2: f64 = 1.5707963267948965580E+00;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
const PI: f64 = 3.1415926535897931160E+00;
#[allow(clippy::excessive_precision)]
const PI_LO: f64 = 1.2246467991473531772E-16;

/// The high word of the bits of the value.
fn hi(value: f64) -> u32 {
    (value.to_bits() >> 32) as u32
}

/// The low word of the bits of the value.
fn lo(value: f64) -> u32 {
    value.to_bits() as u32
}

/// Java `StrictMath.atan(double)`.
pub fn atan(x: f64) -> f64 {
    let hx = hi(x) as i32;
    let ix = hx & 0x7fffffff;
    if ix >= 0x44100000 {
        // |x| >= 2^66
        if ix > 0x7ff00000 || (ix == 0x7ff00000 && lo(x) != 0) {
            return x + x; // NaN
        }
        return if hx > 0 {
            ATANHI[3] + ATANLO[3]
        } else {
            -ATANHI[3] - ATANLO[3]
        };
    }
    let mut id: i32 = -1;
    let mut x = x;
    if ix >= 0x3fdc0000 {
        // |x| >= 0.4375
        x = x.abs();
        id = if ix < 0x3ff30000 {
            if ix < 0x3fe60000 {
                // 7/16 <= |x| < 11/16
                0
            } else {
                // 11/16 <= |x| < 19/16
                1
            }
        } else if ix < 0x40038000 {
            // |x| < 2.4375
            2
        } else {
            // 2.4375 <= |x| < 2^66
            3
        };
        x = match id {
            0 => (2.0 * x - 1.0) / (2.0 + x),
            1 => (x - 1.0) / (x + 1.0),
            2 => (x - 1.5) / (1.0 + 1.5 * x),
            _ => -1.0 / x,
        };
    } else if ix < 0x3e200000 && (1.0e300 + x) > 1.0 {
        // |x| < 2^-29
        return x; // raises the inexact flag
    }
    // The end of the reduction of the argument.
    let z = x * x;
    let w = z * z;
    // The sum from i=0 to 10 of aT[i]*z**(i+1), split into an odd and an
    // even polynomial.
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id < 0 {
        return x - x * (s1 + s2);
    }
    let z = ATANHI[id as usize] - ((x * (s1 + s2) - ATANLO[id as usize]) - x);
    if hx < 0 {
        -z
    } else {
        z
    }
}

/// Java `Math.atan2(double, double)`.
pub fn atan2(y: f64, x: f64) -> f64 {
    let hx = hi(x) as i32;
    let ix = hx & 0x7fffffff;
    let lx = lo(x);
    let hy = hi(y) as i32;
    let iy = hy & 0x7fffffff;
    let ly = lo(y);
    // x or y is NaN
    if (ix == 0x7ff00000 && lx != 0) || (iy == 0x7ff00000 && ly != 0) {
        return x + y;
    }
    if ((hx as u32).wrapping_sub(0x3ff00000) | lx) == 0 {
        return atan(y); // x == 1.0
    }
    // 2*sign(x)+sign(y)
    let m = ((hy >> 31) & 1) | ((hx >> 30) & 2);
    // y == 0
    if (iy as u32 | ly) == 0 {
        return match m {
            0 | 1 => y,
            2 => PI + TINY,
            _ => -PI - TINY,
        };
    }
    // x == 0
    if (ix as u32 | lx) == 0 {
        return if hy < 0 {
            -PI_O_2 - TINY
        } else {
            PI_O_2 + TINY
        };
    }
    // x is infinite
    if ix == 0x7ff00000 {
        if iy == 0x7ff00000 {
            return match m {
                0 => PI_O_4 + TINY,
                1 => -PI_O_4 - TINY,
                2 => 3.0 * PI_O_4 + TINY,
                _ => -3.0 * PI_O_4 - TINY,
            };
        }
        return match m {
            0 => 0.0,
            1 => -0.0,
            2 => PI + TINY,
            _ => -PI - TINY,
        };
    }
    // y is infinite
    if iy == 0x7ff00000 {
        return if hy < 0 {
            -PI_O_2 - TINY
        } else {
            PI_O_2 + TINY
        };
    }
    // The computation of y/x.
    let k = (iy - ix) >> 20;
    let z = if k > 60 {
        PI_O_2 + 0.5 * PI_LO
    } else if hx < 0 && k < -60 {
        0.0
    } else {
        atan((y / x).abs())
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The special cases of `Math.atan2` of the JVM.
    const SPECIAL: &[(f64, f64, u64)] = &[
        (0.0, 1.0, 0x0000000000000000),
        (1.0, 1.0, 0x3fe921fb54442d18),
        (-1.0, 1.0, 0xbfe921fb54442d18),
        (1.0, -1.0, 0x4002d97c7f3321d2),
        (-1.0, -1.0, 0xc002d97c7f3321d2),
        (0.0, -1.0, 0x400921fb54442d18),
        (1.0, 0.0, 0x3ff921fb54442d18),
        (-1.0, 0.0, 0xbff921fb54442d18),
    ];

    /// The values of `Math.atan2` of the JVM for the pairs of the arguments
    /// that `tools/poseidon_mobs/JdkAtan2Vectors.java` produces.
    const VECTORS: &[(u64, u64, u64)] = &[
        (0xc031e6ca9c0132a4, 0xc03096bda62585fc, 0xc0028b93cdb95eb9),
        (0x401f372318ea8f53, 0xc0279b6aeeac5fee, 0x4004759a560b5a64),
        (0x402a5554308c2634, 0xc0323e3b09e1f408, 0x400421a546718a07),
        (0x4000dbb1e51fd532, 0xc01804d7bd70ee85, 0x40066ec3ec9d1812),
        (0x40125d1d23e5d107, 0x4015d5ec8acfbf59, 0x3fe660361ebfd52f),
        (0x40305a80a300b2da, 0xc02ed87e4126803c, 0x40029d853155000c),
        (0xc032d3f6631a0289, 0x402e4712ec726a82, 0xbfec983f6cdc9a5d),
        (0x4000dbb1f6eecdfe, 0x400f6aa089c39b56, 0x3fdf851b5e9fce02),
        (0xc021ecff2a6f7043, 0xc0334677c0e0749b, 0xc005a695803f542e),
        (0x400e61ac15414222, 0xc01c10554f34bdc4, 0x400529e5f1a93b76),
        (0xc014068483a7f743, 0x401b11cb7a45323e, 0xbfe461b5fd985972),
        (0xc0303813657717b4, 0x4032add59bf01211, 0xbfe6e1797e5b35ec),
        (0xc024e4e487b4b122, 0xc027c366c482ff27, 0xc0035ce306833e8a),
        (0xc03185dee80dbe43, 0xc00f1c5eb92bd880, 0xbffca081a522fac0),
        (0xc02f41654a1f1186, 0x4024551231fe11d2, 0xbfefcf4d30526def),
        (0x4012f541ed110c88, 0xc032e01aaee679ec, 0x40072a280f21c700),
        (0x40313ef8649943d0, 0x403205497a08ada8, 0x3fe86e10391e3955),
        (0x402c3f725694f3df, 0x402bbca989cf512e, 0x3fe96cbbe0bd121a),
        (0x403268f6fa212985, 0xc02d09033bcb8bad, 0x4001e88209ceb3c1),
        (0x402af5b731f0dcd6, 0x402afbeac330f169, 0x3fe91e4d91dd2e34),
        (0xc022631baf7c77cb, 0xc005600e4c84ec32, 0xbffda87a4bbf4937),
        (0xc01aceee74f8a349, 0xbfaa913eae614c00, 0xbff941b188bef3d5),
        (0x402c2cb0e285eafe, 0xc004e302ab62eb20, 0x3ffc10973464fa4b),
        (0x4032802e9a31d84a, 0xc0221a68b9c1ab64, 0x400034dcc2af8720),
        (0x401e1507a7434923, 0x402430d2a6799900, 0x3fe47d054a7d6fe8),
        (0x4033e08ea9fb2211, 0x4024de060677f9fd, 0x3ff1661614204a95),
        (0x3fe2f8bcca98f980, 0x402d33ca1670f142, 0x3fa4c71c4a3dc817),
        (0xbffe2ea475ebc880, 0xc02503cb31f7241c, 0xc007b62e6c98aba4),
        (0x4033fcd032238fd0, 0x40324ea8a0aa0d42, 0x3fea8932b6a446f8),
        (0x403062bd9412041b, 0xc03283fcb8738b2f, 0x4003564fd46ecba5),
        (0x401efe1da3c01781, 0x3fe124496b320930, 0x3ff8073fc7bdbcb1),
        (0xc0208925d9e7cc38, 0x402460714e1f9a24, 0xbfe5d09ad1e62aea),
        (0xc0016d62f4f46dc2, 0xc005822fa6d041ee, 0xc003af6056b9c3e7),
        (0x402fe08a140e1ca0, 0xc030dfcf3d1c4eb6, 0x400313df3dd68416),
        (0x4030ce95716a5e9d, 0xc01566a238bf4c20, 0x3ffe1054eb00d52b),
        (0xc03094da7d8958ad, 0xc020760a5fd459d2, 0xc00040960841f39f),
        (0xbfd8589d7d1dff70, 0x4024e2b79e8cec5c, 0xbfa2a48e63eb7b63),
        (0xc00b50220a2ac118, 0xc02ba5cc83b0f8a6, 0xc007321a1f4ca64a),
        (0xc032168ad972fd0e, 0x402bf84252b4f60b, 0xbfed345800e63127),
        (0xbff87975b2dc3ffc, 0x402b4b56257d0208, 0xbfbc932284d69d04),
        (0xc0270bf34e33b698, 0xc0050796b50b447a, 0xbffcb8a4403c56c6),
        (0xc0285360f7fe3696, 0xc025502b094148b4, 0xc002527f4953e6b9),
        (0xc03382cab1f4b985, 0x402ce6928b1362b2, 0xbfedddc1a0a693a2),
        (0x400e9eedafc9b9fe, 0x3ff5a21e74fe9198, 0x3ff3b3224db63ccf),
        (0xc016ae17c36427ca, 0xc0334bd134ccbb2a, 0xc006d8ab436e86cb),
        (0xc02db4cbbe4f7798, 0x400c28aeecd5b774, 0xbff568e694c5a934),
        (0x3feb914ecc5504c0, 0x3ff60777bfbdb0ec, 0x3fe1e43562a036fa),
        (0x402a2a1bdc85f435, 0x403070feabd6f26f, 0x3fe581e718e96468),
        (0xc00cb2d88973fd12, 0x40157d0cdb227ec1, 0xbfe2d72864d757ae),
        (0x403224956b4d57b8, 0x3fd3c773f4ae3a10, 0x3ff8dc3779658eb1),
        (0x4015742e25390b34, 0xc02641fd0fc96dd7, 0x40058a3bd493736e),
        (0x401b7ba4a1e707bb, 0xc03118ffff77bb67, 0x4006136f6980a46e),
        (0xc025cd8098a7f996, 0x402eac9f2f840de6, 0xbfe3c60bf2e66288),
        (0x4020c1e03a2aa291, 0xc0322f843c1f15c0, 0x4005adc67e1295cc),
        (0xc0061b8d0c42d130, 0xc003f921ff12a1aa, 0xc00271b208eef0b5),
        (0x4022dcd3f2f8d628, 0xc01faa0e953292eb, 0x4002270daf6adcac),
        (0xc02f290710927408, 0x400122dfaa7626a0, 0xbff6f25a70a79867),
        (0xc0319bfddf9685cd, 0x4032b8a95f25208c, 0xbfe82752be8b1921),
        (0xc024d2b7b6e1d435, 0x4023b75400f9a94a, 0xbfea019f437175a2),
        (0xc01f9294dea79228, 0x40322ffa9f21c1dd, 0xbfda3497ca8c7a55),
        (0xc015d9b585c0cd79, 0x4023b027423ac8d4, 0xbfe0362328c45727),
        (0x402faf424e5b2354, 0xc0192edd1c2dc83c, 0x3fff2f5ac77027f5),
        (0xc02e9b9409159556, 0x402af5df0b463f8e, 0xbfeb284d4e180b3f),
        (0x40124ce7085754e2, 0x401963a91cf1dc8a, 0x3fe3fc4478c3eacf),
    ];

    #[test]
    fn atan2_matches_the_jvm() {
        for (y, x, expected) in SPECIAL {
            let got = atan2(*y, *x);
            assert_eq!(
                got.to_bits(),
                *expected,
                "atan2({y}, {x}) = {got} != {}",
                f64::from_bits(*expected)
            );
        }
    }

    #[test]
    fn atan2_matches_the_jvm_on_random_values() {
        let mut random = crate::util::rng::JavaRandom::new(0x5EED5EED);
        for (y_bits, x_bits, expected) in VECTORS {
            let y = (random.next_f64() - 0.5) * 40.0;
            let x = (random.next_f64() - 0.5) * 40.0;
            assert_eq!(y.to_bits(), *y_bits, "the arguments of the test differ");
            assert_eq!(x.to_bits(), *x_bits, "the arguments of the test differ");
            let got = atan2(y, x);
            assert_eq!(
                got.to_bits(),
                *expected,
                "atan2({y}, {x}) = {got} != {}",
                f64::from_bits(*expected)
            );
        }
    }
}
