use crate::util::noise::{PerlinNoise, SimplexNoise};
use crate::util::rng::JavaRandom;

pub struct OctavesNoise {
    generators: Vec<PerlinNoise>,
}

impl OctavesNoise {
    pub fn new(random: &mut JavaRandom, octaves: usize) -> Self {
        Self {
            generators: (0..octaves).map(|_| PerlinNoise::new(random)).collect(),
        }
    }

    pub fn noise_2d(&self, x: f64, y: f64) -> f64 {
        let mut total = 0.0;
        let mut scale = 1.0;
        for generator in &self.generators {
            total += generator.noise2(x * scale, y * scale) / scale;
            scale /= 2.0;
        }
        total
    }

    #[allow(clippy::too_many_arguments)]
    pub fn noise_array(
        &self,
        x: f64,
        y: f64,
        z: f64,
        count_x: usize,
        count_y: usize,
        count_z: usize,
        scale_x: f64,
        scale_y: f64,
        scale_z: f64,
    ) -> Vec<f64> {
        let mut out = vec![0.0; count_x * count_y * count_z];
        let mut amplitude = 1.0;
        for generator in &self.generators {
            generator.add_noise(
                &mut out,
                x,
                y,
                z,
                count_x,
                count_y,
                count_z,
                scale_x * amplitude,
                scale_y * amplitude,
                scale_z * amplitude,
                amplitude,
            );
            amplitude /= 2.0;
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub fn noise_array_2d(
        &self,
        x: i32,
        z: i32,
        count_x: usize,
        count_z: usize,
        scale_x: f64,
        scale_z: f64,
    ) -> Vec<f64> {
        self.noise_array(
            x as f64, 10.0, z as f64, count_x, 1, count_z, scale_x, 1.0, scale_z,
        )
    }
}

pub struct SimplexOctavesNoise {
    generators: Vec<SimplexNoise>,
}

impl SimplexOctavesNoise {
    pub fn new(random: &mut JavaRandom, octaves: usize) -> Self {
        Self {
            generators: (0..octaves).map(|_| SimplexNoise::new(random)).collect(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn noise_array(
        &self,
        x: f64,
        y: f64,
        count_x: usize,
        count_y: usize,
        scale_x: f64,
        scale_y: f64,
        amplitude: f64,
    ) -> Vec<f64> {
        self.noise_array_persistence(x, y, count_x, count_y, scale_x, scale_y, amplitude, 0.5)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn noise_array_persistence(
        &self,
        x: f64,
        y: f64,
        count_x: usize,
        count_y: usize,
        scale_x: f64,
        scale_y: f64,
        amplitude: f64,
        persistence: f64,
    ) -> Vec<f64> {
        let scale_x = scale_x / 1.5;
        let scale_y = scale_y / 1.5;
        let mut out = vec![0.0; count_x * count_y];
        let mut frequency = 1.0;
        let mut amplitude_acc = 1.0;
        for generator in &self.generators {
            generator.add_noise(
                &mut out,
                x,
                y,
                count_x,
                count_y,
                scale_x * frequency,
                scale_y * frequency,
                0.55 / amplitude_acc,
            );
            frequency *= amplitude;
            amplitude_acc *= persistence;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(value: f64) -> i64 {
        value.to_bits() as i64
    }

    #[test]
    fn octaves_match_java() {
        let mut random = JavaRandom::new(1234);
        let octaves = OctavesNoise::new(&mut random, 8);
        assert_eq!(bits(octaves.noise_2d(1.5, 2.5)), 4614419848157562720);

        let mut random = JavaRandom::new(1234);
        let octaves = OctavesNoise::new(&mut random, 8);
        let values = octaves.noise_array(0.0, 0.0, 0.0, 5, 1, 5, 0.1, 1.0, 0.1);
        let expected: [i64; 25] = [
            4616392211902277264,
            4616377089730997364,
            4616345540020687548,
            4616306736590234276,
            4616264351859248668,
            4616489006108166036,
            4616471102488474096,
            4616430006210153860,
            4616377338225667764,
            4616320014085691028,
            4616554997885997120,
            4616550371452217256,
            4616513641205581012,
            4616456983193350884,
            4616390173047889044,
            4616612615742778196,
            4616635610383496868,
            4616617012073321344,
            4616567680567310032,
            4616499437087805472,
            4616678767628032724,
            4616737894948184132,
            4616746885981527544,
            4616713936323806216,
            4616651973170095516,
        ];
        for (index, value) in values.iter().enumerate() {
            assert_eq!(bits(*value), expected[index], "index {index}");
        }
    }

    #[test]
    fn octaves_3d_match_java() {
        let mut random = JavaRandom::new(99);
        let octaves = OctavesNoise::new(&mut random, 4);
        let values = octaves.noise_array(0.0, 0.0, 0.0, 5, 17, 5, 0.5, 0.5, 0.5);
        assert_eq!(bits(values[0]), 4610352542750441081);
        assert_eq!(bits(values[42]), 4620463199799400111);
        assert_eq!(bits(values[206]), 4583135749301152512);
        assert_eq!(bits(values[424]), 4595490354718767136);
    }

    #[test]
    fn simplex_octaves_match_java() {
        let mut random = JavaRandom::new(7);
        let octaves = SimplexOctavesNoise::new(&mut random, 4);
        let values = octaves.noise_array(0.0, 0.0, 3, 3, 0.1, 0.1, 0.5);
        let expected: [i64; 9] = [
            4617909559618721446,
            4618021156091894876,
            4618087563898256892,
            4618073450416929092,
            4618164494823322449,
            4618207667414122379,
            4618182873516798272,
            4618256205418849336,
            4618281233026901133,
        ];
        for (index, value) in values.iter().enumerate() {
            assert_eq!(bits(*value), expected[index], "index {index}");
        }
    }
}
