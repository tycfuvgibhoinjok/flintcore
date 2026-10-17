use crate::util::rng::JavaRandom;

const GRADIENTS: [[i32; 3]; 12] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
];

pub struct SimplexNoise {
    permutation: [u8; 512],
    x_offset: f64,
    y_offset: f64,
}

impl SimplexNoise {
    pub fn new(random: &mut JavaRandom) -> Self {
        let x_offset = random.next_f64() * 256.0;
        let y_offset = random.next_f64() * 256.0;
        // b1.7.3 NoiseGenerator2 consumes a z offset it never reads; keep the RNG draw.
        let _z_offset = random.next_f64() * 256.0;

        let mut permutation = [0u8; 512];
        for (index, slot) in permutation.iter_mut().take(256).enumerate() {
            *slot = index as u8;
        }
        for index in 0..256 {
            let other = random.next_i32_bound((256 - index) as i32) as usize + index;
            permutation.swap(index, other);
            permutation[index + 256] = permutation[index];
        }

        Self {
            permutation,
            x_offset,
            y_offset,
        }
    }

    fn p(&self, index: usize) -> i32 {
        self.permutation[index] as i32
    }

    fn fast_floor(value: f64) -> i32 {
        let truncated = value as i32;
        if value > 0.0 {
            truncated
        } else {
            truncated - 1
        }
    }

    fn dot(gradient: [i32; 3], x: f64, y: f64) -> f64 {
        gradient[0] as f64 * x + gradient[1] as f64 * y
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_noise(
        &self,
        out: &mut [f64],
        x: f64,
        y: f64,
        count_x: usize,
        count_y: usize,
        scale_x: f64,
        scale_y: f64,
        amplitude: f64,
    ) {
        // f = 0.5*(sqrt(3)-1), g = (3-sqrt(3))/6 (b1.7.3 2D simplex skew factors)
        let f = 0.5 * (3.0f64.sqrt() - 1.0);
        let g = (3.0 - 3.0f64.sqrt()) / 6.0;

        let mut cursor = 0usize;
        for l in 0..count_x {
            let dx = (x + l as f64) * scale_x + self.x_offset;
            for i1 in 0..count_y {
                let dz = (y + i1 as f64) * scale_y + self.y_offset;
                let skew = (dx + dz) * f;
                let j1 = Self::fast_floor(dx + skew);
                let k1 = Self::fast_floor(dz + skew);
                let unskew = (j1 + k1) as f64 * g;
                let d9 = j1 as f64 - unskew;
                let d10 = k1 as f64 - unskew;
                let d11 = dx - d9;
                let d12 = dz - d10;
                let (b0, b1) = if d11 > d12 { (1, 0) } else { (0, 1) };
                let d13 = d11 - b0 as f64 + g;
                let d14 = d12 - b1 as f64 + g;
                let d15 = d11 - 1.0 + 2.0 * g;
                let d16 = d12 - 1.0 + 2.0 * g;
                let l1 = (j1 & 255) as usize;
                let i2 = (k1 & 255) as usize;
                let j2 = self.p(l1 + self.p(i2) as usize) % 12;
                let k2 = self.p(l1 + b0 as usize + self.p(i2 + b1 as usize) as usize) % 12;
                let l2 = self.p(l1 + 1 + self.p(i2 + 1) as usize) % 12;

                let d17 = 0.5 - d11 * d11 - d12 * d12;
                let third = if d17 < 0.0 {
                    0.0
                } else {
                    let squared = d17 * d17;
                    squared * squared * Self::dot(GRADIENTS[j2 as usize], d11, d12)
                };

                let d19 = 0.5 - d13 * d13 - d14 * d14;
                let half = if d19 < 0.0 {
                    0.0
                } else {
                    let squared = d19 * d19;
                    squared * squared * Self::dot(GRADIENTS[k2 as usize], d13, d14)
                };

                let d21 = 0.5 - d15 * d15 - d16 * d16;
                let other = if d21 < 0.0 {
                    0.0
                } else {
                    let squared = d21 * d21;
                    squared * squared * Self::dot(GRADIENTS[l2 as usize], d15, d16)
                };

                out[cursor] += 70.0 * (third + half + other) * amplitude;
                cursor += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_simplex() {
        let mut random = JavaRandom::new(7);
        let simplex = SimplexNoise::new(&mut random);
        let mut out = vec![0.0; 4];
        simplex.add_noise(&mut out, 0.0, 0.0, 2, 2, 0.1, 0.1, 1.0);
        let expected: [i64; 4] = [
            4601179631147750963,
            4603171882516192883,
            4602990949950669483,
            4603984963865709362,
        ];
        for (index, value) in out.iter().enumerate() {
            assert_eq!(value.to_bits() as i64, expected[index], "index {index}");
        }
    }
}
