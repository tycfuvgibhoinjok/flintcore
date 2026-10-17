pub mod octaves;
pub mod perlin;
pub mod simplex;

pub use octaves::{OctavesNoise, SimplexOctavesNoise};
pub use perlin::PerlinNoise;
pub use simplex::SimplexNoise;

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
