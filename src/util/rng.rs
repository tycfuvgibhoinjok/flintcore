const MULTIPLIER: u64 = 0x5DEECE66D;
const ADDEND: u64 = 0xB;
const MASK: u64 = (1 << 48) - 1;

#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: u64,
    /// The second value of the normal distribution (`java.util.Random`).
    have_next_next_gaussian: bool,
    next_next_gaussian: f64,
    /// The number of the draws of the stream (a diagnostic counter).
    pub draws: u64,
    /// The raw values of the draws (a diagnostic trace).
    pub trace: bool,
    pub trace_values: Vec<i32>,
}

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        Self {
            seed: (seed as u64 ^ MULTIPLIER) & MASK,
            have_next_next_gaussian: false,
            next_next_gaussian: 0.0,
            draws: 0,
            trace: false,
            trace_values: Vec::new(),
        }
    }

    pub fn set_seed(&mut self, seed: i64) {
        self.seed = (seed as u64 ^ MULTIPLIER) & MASK;
        self.have_next_next_gaussian = false;
        self.next_next_gaussian = 0.0;
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.draws += 1;
        self.seed = self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND) & MASK;
        let value = (self.seed >> (48 - bits)) as i32;
        if self.trace {
            self.trace_values.push(value);
        }
        value
    }

    pub fn next_i32(&mut self) -> i32 {
        self.next(32)
    }

    pub fn next_i32_bound(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        if bound & -bound == bound {
            let product = bound as i64 * self.next(31) as i64;
            return (product >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let value = bits % bound;
            // Java computes the difference in a long, the sum overflows an
            // i32 for a large bound.
            if (bits as i64 - value as i64 + (bound as i64 - 1)) >= 0 {
                return value;
            }
        }
    }

    pub fn next_i64(&mut self) -> i64 {
        let high = self.next(32) as i64;
        let low = self.next(32) as i64;
        (high << 32).wrapping_add(low)
    }

    pub fn next_f32(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }

    pub fn next_f64(&mut self) -> f64 {
        let high = self.next(26) as i64;
        let low = self.next(27) as i64;
        ((high << 27) + low) as f64 / (1u64 << 53) as f64
    }

    pub fn next_bool(&mut self) -> bool {
        self.next(1) != 0
    }

    /// Java `java.util.Random.nextGaussian()`.
    ///
    /// The polar method of the reference: the values are drawn in pairs, the
    /// second one of the pair is kept for the next call. The logarithm and the
    /// square root are the ones of `StrictMath` (the `fdlibm` of the JVM), the
    /// port has to use the same ones, the values are compared bit by bit.
    pub fn next_gaussian(&mut self) -> f64 {
        if self.have_next_next_gaussian {
            self.have_next_next_gaussian = false;
            return self.next_next_gaussian;
        }
        loop {
            let v1 = 2.0 * self.next_f64() - 1.0;
            let v2 = 2.0 * self.next_f64() - 1.0;
            let s = v1 * v1 + v2 * v2;
            if s >= 1.0 || s == 0.0 {
                continue;
            }
            let multiplier = (-2.0 * crate::util::fdlibm::log(s) / s).sqrt();
            self.next_next_gaussian = v2 * multiplier;
            self.have_next_next_gaussian = true;
            return v1 * multiplier;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_util_random() {
        let mut random = JavaRandom::new(42);
        assert_eq!(random.next_i32(), -1170105035);

        let mut random = JavaRandom::new(0);
        assert_eq!(random.next_i32(), -1155484576);
    }

    #[test]
    fn bounded_values_stay_in_range() {
        let mut random = JavaRandom::new(1);
        for _ in 0..1000 {
            let value = random.next_i32_bound(16);
            assert!((0..16).contains(&value));
        }
    }

    #[test]
    fn floats_are_unit_interval() {
        let mut random = JavaRandom::new(7);
        for _ in 0..1000 {
            let value = random.next_f64();
            assert!((0.0..1.0).contains(&value));
        }
    }
}

#[cfg(test)]
mod rng_parity_tests {
    use super::*;

    #[test]
    fn the_random_stream_matches_the_reference() {
        let mut rng = JavaRandom::new(12345);
        // The first raw values of the reference (the stack trace of the
        // harness): 776966251, 1102109080, 2003588241, 15386631, 13976953.
        assert_eq!(rng.next(31), 776966251);
        assert_eq!(rng.next(31), 1102109080);
        assert_eq!(rng.next(31), 2003588241);
        assert_eq!(rng.next(24), 15386631);
        assert_eq!(rng.next(24), 13976953);
    }

    /// The normal distribution of `java.util.Random` (the reference values are
    /// produced by `tools/poseidon_mobs/GaussianReference.java`).
    const GAUSSIAN: [u64; 16] = [
        0xbfc80a1ffe3d6b0b,
        0x3fe2d4785fc80083,
        0x3fee5c9bd7c8e902,
        0xbfdfa24b9a887588,
        0xbff39318653b97f7,
        0xbfe655b243032927,
        0xbfe8df06be7513e9,
        0x40008b4825227c3f,
        0xbfe2cb8b63fc073b,
        0x3fdd93ca0fa04b52,
        0x3ff5fe497cd364c4,
        0xbfb90ce87f8aa22d,
        0xbff139155940f8af,
        0x3ff3ce9892461252,
        0x3fe520e34415334d,
        0xbfce482c0542ab2c,
    ];

    #[test]
    fn the_normal_distribution_matches_the_reference() {
        let mut rng = JavaRandom::new(12345);
        for expected in GAUSSIAN {
            let got = rng.next_gaussian();
            assert_eq!(
                got.to_bits(),
                expected,
                "nextGaussian() = {got} != {}",
                f64::from_bits(expected)
            );
        }
    }

    /// The logarithm and the square root of the polar method (the reference
    /// values are produced by `tools/poseidon_mobs/GaussianReference.java`).
    const STRICT_MATH: [(u64, u64); 8] = [
        (0xbfe2b8364a2ec462, 0x3ff72fc25113ac2d),
        (0xbf74489264763ca7, 0x3fb98a4018216ea2),
        (0xbff30ba1afe7c6d9, 0x400662102abd5e90),
        (0xbfe890804f832f91, 0x3ffd19cb734aa352),
        (0xbfddf9060efb7fd5, 0x3ff39211351f1e57),
        (0xbfd1a68f6a1618ab, 0x3feb47a597dd35ef),
        (0xbfb12ab660bffd2a, 0x3fd83cb6632f1e8a),
        (0xbfd52716cc4208d7, 0x3feeb130c4f7cc7f),
    ];

    #[test]
    fn the_logarithm_of_the_reference_matches_the_jvm() {
        // The values of the reference are the logarithms of the pairs of the
        // polar method, the pairs are drawn the same way.
        let mut rng = JavaRandom::new(987_654_321);
        for (log_bits, sqrt_bits) in STRICT_MATH {
            let (x, y) = loop {
                let x = 2.0 * rng.next_f64() - 1.0;
                let y = 2.0 * rng.next_f64() - 1.0;
                let s = x * x + y * y;
                if s < 1.0 && s != 0.0 {
                    break (x, y);
                }
            };
            let s = x * x + y * y;
            let log = crate::util::fdlibm::log(s);
            assert_eq!(log.to_bits(), log_bits, "log({s}) = {log}");
            let root = (-2.0 * log / s).sqrt();
            assert_eq!(root.to_bits(), sqrt_bits, "sqrt({s}) = {root}");
        }
    }
}
