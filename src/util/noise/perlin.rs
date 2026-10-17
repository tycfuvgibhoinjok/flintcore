use crate::util::math::{floor_double, lerp};
use crate::util::noise::fade;
use crate::util::rng::JavaRandom;

pub struct PerlinNoise {
    permutation: [u8; 512],
    x_offset: f64,
    y_offset: f64,
    z_offset: f64,
}

impl PerlinNoise {
    pub fn new(random: &mut JavaRandom) -> Self {
        let x_offset = random.next_f64() * 256.0;
        let y_offset = random.next_f64() * 256.0;
        let z_offset = random.next_f64() * 256.0;

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
            z_offset,
        }
    }

    fn p(&self, index: usize) -> i32 {
        self.permutation[index] as i32
    }

    pub fn noise3(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut dx = x + self.x_offset;
        let mut dy = y + self.y_offset;
        let mut dz = z + self.z_offset;
        let i = floor_double(dx);
        let j = floor_double(dy);
        let k = floor_double(dz);
        let l = (i & 255) as usize;
        let i1 = (j & 255) as usize;
        let j1 = (k & 255) as usize;
        dx -= i as f64;
        dy -= j as f64;
        dz -= k as f64;
        let f6 = fade(dx);
        let f7 = fade(dy);
        let f8 = fade(dz);
        let k1 = self.p(l) + i1 as i32;
        let l1 = self.p(k1 as usize) + j1 as i32;
        let i2 = self.p(k1 as usize + 1) + j1 as i32;
        let j2 = self.p(l + 1) + i1 as i32;
        let k2 = self.p(j2 as usize) + j1 as i32;
        let l2 = self.p(j2 as usize + 1) + j1 as i32;

        lerp(
            f8,
            lerp(
                f7,
                lerp(
                    f6,
                    self.grad3(self.p(l1 as usize), dx, dy, dz),
                    self.grad3(self.p(k2 as usize), dx - 1.0, dy, dz),
                ),
                lerp(
                    f6,
                    self.grad3(self.p(i2 as usize), dx, dy - 1.0, dz),
                    self.grad3(self.p(l2 as usize), dx - 1.0, dy - 1.0, dz),
                ),
            ),
            lerp(
                f7,
                lerp(
                    f6,
                    self.grad3(self.p(l1 as usize + 1), dx, dy, dz - 1.0),
                    self.grad3(self.p(k2 as usize + 1), dx - 1.0, dy, dz - 1.0),
                ),
                lerp(
                    f6,
                    self.grad3(self.p(i2 as usize + 1), dx, dy - 1.0, dz - 1.0),
                    self.grad3(self.p(l2 as usize + 1), dx - 1.0, dy - 1.0, dz - 1.0),
                ),
            ),
        )
    }

    pub fn noise2(&self, x: f64, y: f64) -> f64 {
        self.noise3(x, y, 0.0)
    }

    fn grad2(&self, hash: i32, x: f64, y: f64) -> f64 {
        let j = hash & 15;
        let dx = (1 - ((j & 8) >> 3)) as f64 * x;
        let dy = if j < 4 {
            0.0
        } else if j != 12 && j != 14 {
            y
        } else {
            x
        };
        (if j & 1 == 0 { dx } else { -dx }) + (if j & 2 == 0 { dy } else { -dy })
    }

    fn grad3(&self, hash: i32, x: f64, y: f64, z: f64) -> f64 {
        let j = hash & 15;
        let dx = if j < 8 { x } else { y };
        let dy = if j < 4 {
            y
        } else if j != 12 && j != 14 {
            z
        } else {
            x
        };
        (if j & 1 == 0 { dx } else { -dx }) + (if j & 2 == 0 { dy } else { -dy })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_noise(
        &self,
        out: &mut [f64],
        x: f64,
        y: f64,
        z: f64,
        count_x: usize,
        count_y: usize,
        count_z: usize,
        scale_x: f64,
        scale_y: f64,
        scale_z: f64,
        amplitude: f64,
    ) {
        if count_y == 1 {
            self.add_noise_2d(out, x, z, count_x, count_z, scale_x, scale_z, amplitude);
            return;
        }

        let inverse = 1.0 / amplitude;
        let mut cache = -1i32;
        let mut d16 = 0.0;
        let mut d7 = 0.0;
        let mut d17 = 0.0;
        let mut d8 = 0.0;
        let mut cursor = 0usize;

        for j1 in 0..count_x {
            let mut dx = (x + j1 as f64) * scale_x + self.x_offset;
            let k1 = floor_double(dx);
            let l1 = (k1 & 255) as usize;
            dx -= k1 as f64;
            let f10 = fade(dx);

            for l3 in 0..count_z {
                let mut dz = (z + l3 as f64) * scale_z + self.z_offset;
                let i4 = floor_double(dz);
                let j4 = (i4 & 255) as usize;
                dz -= i4 as f64;
                let f19 = fade(dz);

                for k4 in 0..count_y {
                    let mut dy = (y + k4 as f64) * scale_y + self.y_offset;
                    let l4 = floor_double(dy);
                    let i5 = l4 & 255;
                    dy -= l4 as f64;
                    let f21 = fade(dy);

                    if k4 == 0 || i5 != cache {
                        cache = i5;
                        let j5 = self.p(l1) + i5;
                        let k5 = self.p(j5 as usize) + j4 as i32;
                        let l5 = self.p(j5 as usize + 1) + j4 as i32;
                        let i6 = self.p(l1 + 1) + i5;
                        let j2 = self.p(i6 as usize) + j4 as i32;
                        let j6 = self.p(i6 as usize + 1) + j4 as i32;

                        d16 = lerp(
                            f10,
                            self.grad3(self.p(k5 as usize), dx, dy, dz),
                            self.grad3(self.p(j2 as usize), dx - 1.0, dy, dz),
                        );
                        d7 = lerp(
                            f10,
                            self.grad3(self.p(l5 as usize), dx, dy - 1.0, dz),
                            self.grad3(self.p(j6 as usize), dx - 1.0, dy - 1.0, dz),
                        );
                        d17 = lerp(
                            f10,
                            self.grad3(self.p(k5 as usize + 1), dx, dy, dz - 1.0),
                            self.grad3(self.p(j2 as usize + 1), dx - 1.0, dy, dz - 1.0),
                        );
                        d8 = lerp(
                            f10,
                            self.grad3(self.p(l5 as usize + 1), dx, dy - 1.0, dz - 1.0),
                            self.grad3(self.p(j6 as usize + 1), dx - 1.0, dy - 1.0, dz - 1.0),
                        );
                    }

                    let d22 = lerp(f21, d16, d7);
                    let d23 = lerp(f21, d17, d8);
                    let d24 = lerp(f19, d22, d23);
                    out[cursor] += d24 * inverse;
                    cursor += 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_noise_2d(
        &self,
        out: &mut [f64],
        x: f64,
        z: f64,
        count_x: usize,
        count_z: usize,
        scale_x: f64,
        scale_z: f64,
        amplitude: f64,
    ) {
        let inverse = 1.0 / amplitude;
        let mut cursor = 0usize;

        for k2 in 0..count_x {
            let mut dx = (x + k2 as f64) * scale_x + self.x_offset;
            let l2 = floor_double(dx);
            let i3 = (l2 & 255) as usize;
            dx -= l2 as f64;
            let f8 = fade(dx);

            for j1 in 0..count_z {
                let mut dz = (z + j1 as f64) * scale_z + self.z_offset;
                let k1 = floor_double(dz);
                let l1 = (k1 & 255) as usize;
                dz -= k1 as f64;
                let f10 = fade(dz);

                let l = self.p(i3);
                let j3 = self.p(l as usize) + l1 as i32;
                let k3 = self.p(i3 + 1);
                let i1 = self.p(k3 as usize) + l1 as i32;

                let d11 = lerp(
                    f8,
                    self.grad2(self.p(j3 as usize), dx, dz),
                    self.grad3(self.p(i1 as usize), dx - 1.0, 0.0, dz),
                );
                let d12 = lerp(
                    f8,
                    self.grad3(self.p(j3 as usize + 1), dx, 0.0, dz - 1.0),
                    self.grad3(self.p(i1 as usize + 1), dx - 1.0, 0.0, dz - 1.0),
                );
                let d14 = lerp(f10, d11, d12);

                out[cursor] += d14 * inverse;
                cursor += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(value: f64) -> i64 {
        value.to_bits() as i64
    }

    #[test]
    fn matches_java_perlin() {
        let mut random = JavaRandom::new(42);
        let perlin = PerlinNoise::new(&mut random);
        assert_eq!(bits(perlin.noise3(0.5, 0.25, 0.75)), -4622441552545786778);
        assert_eq!(bits(perlin.noise3(12.34, -5.6, 100.0)), 4599745837075539668);
        assert_eq!(bits(perlin.noise2(3.0, 7.0)), 4598280523984164472);
    }
}
