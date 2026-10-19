use crate::gameplay::blocks;
use crate::util::math::{cos, floor_double, sin};
use crate::util::rng::JavaRandom;

fn block_index(x: usize, y: usize, z: usize) -> usize {
    (x << 11) | (z << 7) | y
}

pub struct MapGenCaves {
    range: i32,
    random: JavaRandom,
}

impl Default for MapGenCaves {
    fn default() -> Self {
        Self::new()
    }
}

impl MapGenCaves {
    pub fn new() -> Self {
        Self {
            range: 8,
            random: JavaRandom::new(0),
        }
    }

    pub fn generate(&mut self, seed: i64, target_x: i32, target_z: i32, blocks: &mut [u8]) {
        self.random.set_seed(seed);
        let x_multiplier = self.random.next_i64() / 2 * 2 + 1;
        let z_multiplier = self.random.next_i64() / 2 * 2 + 1;

        for cave_x in (target_x - self.range)..=(target_x + self.range) {
            for cave_z in (target_z - self.range)..=(target_z + self.range) {
                self.random.set_seed(
                    (cave_x as i64)
                        .wrapping_mul(x_multiplier)
                        .wrapping_add((cave_z as i64).wrapping_mul(z_multiplier))
                        ^ seed,
                );
                self.generate_chunk(cave_x, cave_z, target_x, target_z, blocks);
            }
        }
    }

    fn generate_chunk(
        &mut self,
        cave_x: i32,
        cave_z: i32,
        target_x: i32,
        target_z: i32,
        blocks: &mut [u8],
    ) {
        let bound = self.random.next_i32_bound(40) + 1;
        let bound = self.random.next_i32_bound(bound) + 1;
        let mut count = self.random.next_i32_bound(bound);

        if self.random.next_i32_bound(15) != 0 {
            count = 0;
        }

        for _ in 0..count {
            let x = (cave_x * 16 + self.random.next_i32_bound(16)) as f64;
            let y_bound = self.random.next_i32_bound(120) + 8;
            let y = self.random.next_i32_bound(y_bound) as f64;
            let z = (cave_z * 16 + self.random.next_i32_bound(16)) as f64;
            let mut branches = 1;

            if self.random.next_i32_bound(4) == 0 {
                self.carve(target_x, target_z, blocks, x, y, z);
                branches += self.random.next_i32_bound(4);
            }

            for _ in 0..branches {
                let yaw = self.random.next_f32() * std::f32::consts::PI * 2.0;
                let pitch = (self.random.next_f32() - 0.5) * 2.0 / 8.0;
                let thickness = self.random.next_f32() * 2.0 + self.random.next_f32();
                self.carve_region(
                    target_x, target_z, blocks, x, y, z, thickness, yaw, pitch, 0, 0, 1.0,
                );
            }
        }
    }

    fn carve(&mut self, target_x: i32, target_z: i32, blocks: &mut [u8], x: f64, y: f64, z: f64) {
        let thickness = 1.0 + self.random.next_f32() * 6.0;
        self.carve_region(
            target_x, target_z, blocks, x, y, z, thickness, 0.0, 0.0, -1, -1, 0.5,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn carve_region(
        &mut self,
        target_x: i32,
        target_z: i32,
        blocks: &mut [u8],
        mut x: f64,
        mut y: f64,
        mut z: f64,
        thickness: f32,
        mut yaw: f32,
        mut pitch: f32,
        mut step: i32,
        mut max_step: i32,
        vertical_scale: f64,
    ) {
        let center_x = (target_x * 16 + 8) as f64;
        let center_z = (target_z * 16 + 8) as f64;
        let mut yaw_velocity = 0.0f32;
        let mut pitch_velocity = 0.0f32;
        let mut random = JavaRandom::new(self.random.next_i64());

        if max_step <= 0 {
            let bound = self.range * 16 - 16;
            max_step = bound - random.next_i32_bound(bound / 4);
        }

        let mut is_room = false;
        if step == -1 {
            step = max_step / 2;
            is_room = true;
        }

        let split = random.next_i32_bound(max_step / 2) + max_step / 4;
        let steep = random.next_i32_bound(6) == 0;

        while step < max_step {
            let radius = 1.5
                + (sin(step as f32 * std::f32::consts::PI / max_step as f32) * thickness * 1.0f32)
                    as f64;
            let vertical_radius = radius * vertical_scale;
            let cos_pitch = cos(pitch);
            let sin_pitch = sin(pitch);

            x += (cos(yaw) * cos_pitch) as f64;
            y += sin_pitch as f64;
            z += (sin(yaw) * cos_pitch) as f64;

            if steep {
                pitch *= 0.92;
            } else {
                pitch *= 0.7;
            }

            pitch += pitch_velocity * 0.1;
            yaw += yaw_velocity * 0.1;
            pitch_velocity *= 0.9;
            yaw_velocity *= 0.75;
            pitch_velocity += (random.next_f32() - random.next_f32()) * random.next_f32() * 2.0;
            yaw_velocity += (random.next_f32() - random.next_f32()) * random.next_f32() * 4.0;

            if !is_room && step == split && thickness > 1.0 {
                self.carve_region(
                    target_x,
                    target_z,
                    blocks,
                    x,
                    y,
                    z,
                    random.next_f32() * 0.5 + 0.5,
                    yaw - std::f32::consts::FRAC_PI_2,
                    pitch / 3.0,
                    step,
                    max_step,
                    1.0,
                );
                self.carve_region(
                    target_x,
                    target_z,
                    blocks,
                    x,
                    y,
                    z,
                    random.next_f32() * 0.5 + 0.5,
                    yaw + std::f32::consts::FRAC_PI_2,
                    pitch / 3.0,
                    step,
                    max_step,
                    1.0,
                );
                return;
            }

            if is_room || random.next_i32_bound(4) != 0 {
                let dx = x - center_x;
                let dz = z - center_z;
                let remaining = (max_step - step) as f64;
                let reach = (thickness + 2.0f32 + 16.0f32) as f64;

                if dx * dx + dz * dz - remaining * remaining > reach * reach {
                    return;
                }

                if x >= center_x - 16.0 - radius * 2.0
                    && z >= center_z - 16.0 - radius * 2.0
                    && x <= center_x + 16.0 + radius * 2.0
                    && z <= center_z + 16.0 + radius * 2.0
                {
                    let mut min_x = floor_double(x - radius) - target_x * 16 - 1;
                    let mut max_x = floor_double(x + radius) - target_x * 16 + 1;
                    let mut min_y = floor_double(y - vertical_radius) - 1;
                    let mut max_y = floor_double(y + vertical_radius) + 1;
                    let mut min_z = floor_double(z - radius) - target_z * 16 - 1;
                    let mut max_z = floor_double(z + radius) - target_z * 16 + 1;

                    min_x = min_x.max(0);
                    max_x = max_x.min(16);
                    min_y = min_y.max(1);
                    max_y = max_y.min(120);
                    min_z = min_z.max(0);
                    max_z = max_z.min(16);

                    let mut water = false;

                    'outer: for bx in min_x..max_x {
                        for bz in min_z..max_z {
                            let mut by = max_y + 1;
                            while by >= min_y - 1 {
                                let index = block_index(bx as usize, by as usize, bz as usize);
                                if (0..128).contains(&by) {
                                    let block = blocks[index];
                                    if block == blocks::WATER || block == blocks::STATIONARY_WATER {
                                        water = true;
                                        break 'outer;
                                    }
                                    if by != min_y - 1
                                        && bx != min_x
                                        && bx != max_x - 1
                                        && bz != min_z
                                        && bz != max_z - 1
                                    {
                                        by = min_y;
                                    }
                                }
                                by -= 1;
                            }
                        }
                    }

                    if !water {
                        for bx in min_x..max_x {
                            let rel_x = ((bx + target_x * 16) as f64 + 0.5 - x) / radius;

                            for bz in min_z..max_z {
                                let rel_z = ((bz + target_z * 16) as f64 + 0.5 - z) / radius;
                                let mut index =
                                    block_index(bx as usize, max_y as usize, bz as usize);
                                let mut under_grass = false;

                                if rel_x * rel_x + rel_z * rel_z < 1.0 {
                                    let mut by = max_y - 1;
                                    while by >= min_y {
                                        let rel_y = (by as f64 + 0.5 - y) / vertical_radius;

                                        if rel_y > -0.7
                                            && rel_x * rel_x + rel_y * rel_y + rel_z * rel_z < 1.0
                                        {
                                            let block = blocks[index];

                                            if block == blocks::GRASS {
                                                under_grass = true;
                                            }

                                            if block == blocks::STONE
                                                || block == blocks::DIRT
                                                || block == blocks::GRASS
                                            {
                                                if by < 10 {
                                                    blocks[index] = blocks::LAVA;
                                                } else {
                                                    blocks[index] = blocks::AIR;
                                                    if under_grass
                                                        && blocks[index - 1] == blocks::DIRT
                                                    {
                                                        blocks[index - 1] = blocks::GRASS;
                                                    }
                                                }
                                            }
                                        }

                                        by -= 1;
                                        index -= 1;
                                    }
                                }
                            }
                        }

                        if is_room {
                            break;
                        }
                    }
                }
            }

            step += 1;
        }
    }
}

pub struct MapGenCavesHell {
    range: i32,
    random: JavaRandom,
}

impl Default for MapGenCavesHell {
    fn default() -> Self {
        Self::new()
    }
}

impl MapGenCavesHell {
    pub fn new() -> Self {
        Self {
            range: 8,
            random: JavaRandom::new(0),
        }
    }

    pub fn generate(&mut self, seed: i64, target_x: i32, target_z: i32, blocks: &mut [u8]) {
        self.random.set_seed(seed);
        let x_multiplier = self.random.next_i64() / 2 * 2 + 1;
        let z_multiplier = self.random.next_i64() / 2 * 2 + 1;

        for cave_x in (target_x - self.range)..=(target_x + self.range) {
            for cave_z in (target_z - self.range)..=(target_z + self.range) {
                self.random.set_seed(
                    (cave_x as i64)
                        .wrapping_mul(x_multiplier)
                        .wrapping_add((cave_z as i64).wrapping_mul(z_multiplier))
                        ^ seed,
                );
                self.generate_chunk(cave_x, cave_z, target_x, target_z, blocks);
            }
        }
    }

    fn generate_chunk(
        &mut self,
        cave_x: i32,
        cave_z: i32,
        target_x: i32,
        target_z: i32,
        blocks: &mut [u8],
    ) {
        let bound = self.random.next_i32_bound(10) + 1;
        let bound = self.random.next_i32_bound(bound) + 1;
        let mut count = self.random.next_i32_bound(bound);

        if self.random.next_i32_bound(5) != 0 {
            count = 0;
        }

        for _ in 0..count {
            let x = (cave_x * 16 + self.random.next_i32_bound(16)) as f64;
            let y = self.random.next_i32_bound(128) as f64;
            let z = (cave_z * 16 + self.random.next_i32_bound(16)) as f64;
            let mut branches = 1;

            if self.random.next_i32_bound(4) == 0 {
                self.carve(target_x, target_z, blocks, x, y, z);
                branches += self.random.next_i32_bound(4);
            }

            for _ in 0..branches {
                let yaw = self.random.next_f32() * std::f32::consts::PI * 2.0;
                let pitch = (self.random.next_f32() - 0.5) * 2.0 / 8.0;
                let thickness = self.random.next_f32() * 2.0 + self.random.next_f32();
                self.carve_region(
                    target_x,
                    target_z,
                    blocks,
                    x,
                    y,
                    z,
                    thickness * 2.0,
                    yaw,
                    pitch,
                    0,
                    0,
                    0.5,
                );
            }
        }
    }

    fn carve(&mut self, target_x: i32, target_z: i32, blocks: &mut [u8], x: f64, y: f64, z: f64) {
        let thickness = 1.0 + self.random.next_f32() * 6.0;
        self.carve_region(
            target_x, target_z, blocks, x, y, z, thickness, 0.0, 0.0, -1, -1, 0.5,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn carve_region(
        &mut self,
        target_x: i32,
        target_z: i32,
        blocks: &mut [u8],
        mut x: f64,
        mut y: f64,
        mut z: f64,
        thickness: f32,
        mut yaw: f32,
        mut pitch: f32,
        mut step: i32,
        mut max_step: i32,
        vertical_scale: f64,
    ) {
        let center_x = (target_x * 16 + 8) as f64;
        let center_z = (target_z * 16 + 8) as f64;
        let mut yaw_velocity = 0.0f32;
        let mut pitch_velocity = 0.0f32;
        let mut random = JavaRandom::new(self.random.next_i64());

        if max_step <= 0 {
            let bound = self.range * 16 - 16;
            max_step = bound - random.next_i32_bound(bound / 4);
        }

        let mut is_room = false;
        if step == -1 {
            step = max_step / 2;
            is_room = true;
        }

        let split = random.next_i32_bound(max_step / 2) + max_step / 4;
        let steep = random.next_i32_bound(6) == 0;

        while step < max_step {
            let radius = 1.5
                + (sin(step as f32 * std::f32::consts::PI / max_step as f32) * thickness * 1.0f32)
                    as f64;
            let vertical_radius = radius * vertical_scale;
            let cos_pitch = cos(pitch);
            let sin_pitch = sin(pitch);

            x += (cos(yaw) * cos_pitch) as f64;
            y += sin_pitch as f64;
            z += (sin(yaw) * cos_pitch) as f64;

            if steep {
                pitch *= 0.92;
            } else {
                pitch *= 0.7;
            }

            pitch += pitch_velocity * 0.1;
            yaw += yaw_velocity * 0.1;
            pitch_velocity *= 0.9;
            yaw_velocity *= 0.75;
            pitch_velocity += (random.next_f32() - random.next_f32()) * random.next_f32() * 2.0;
            yaw_velocity += (random.next_f32() - random.next_f32()) * random.next_f32() * 4.0;

            if !is_room && step == split && thickness > 1.0 {
                self.carve_region(
                    target_x,
                    target_z,
                    blocks,
                    x,
                    y,
                    z,
                    random.next_f32() * 0.5 + 0.5,
                    yaw - std::f32::consts::FRAC_PI_2,
                    pitch / 3.0,
                    step,
                    max_step,
                    1.0,
                );
                self.carve_region(
                    target_x,
                    target_z,
                    blocks,
                    x,
                    y,
                    z,
                    random.next_f32() * 0.5 + 0.5,
                    yaw + std::f32::consts::FRAC_PI_2,
                    pitch / 3.0,
                    step,
                    max_step,
                    1.0,
                );
                return;
            }

            if is_room || random.next_i32_bound(4) != 0 {
                let dx = x - center_x;
                let dz = z - center_z;
                let remaining = (max_step - step) as f64;
                let reach = (thickness + 2.0f32 + 16.0f32) as f64;

                if dx * dx + dz * dz - remaining * remaining > reach * reach {
                    return;
                }

                if x >= center_x - 16.0 - radius * 2.0
                    && z >= center_z - 16.0 - radius * 2.0
                    && x <= center_x + 16.0 + radius * 2.0
                    && z <= center_z + 16.0 + radius * 2.0
                {
                    let mut min_x = floor_double(x - radius) - target_x * 16 - 1;
                    let mut max_x = floor_double(x + radius) - target_x * 16 + 1;
                    let mut min_y = floor_double(y - vertical_radius) - 1;
                    let mut max_y = floor_double(y + vertical_radius) + 1;
                    let mut min_z = floor_double(z - radius) - target_z * 16 - 1;
                    let mut max_z = floor_double(z + radius) - target_z * 16 + 1;

                    min_x = min_x.max(0);
                    max_x = max_x.min(16);
                    min_y = min_y.max(1);
                    max_y = max_y.min(120);
                    min_z = min_z.max(0);
                    max_z = max_z.min(16);

                    let mut lava = false;

                    'outer: for bx in min_x..max_x {
                        for bz in min_z..max_z {
                            let mut by = max_y + 1;
                            while by >= min_y - 1 {
                                let index = block_index(bx as usize, by as usize, bz as usize);
                                if (0..128).contains(&by) {
                                    let block = blocks[index];
                                    if block == blocks::LAVA || block == blocks::STATIONARY_LAVA {
                                        lava = true;
                                        break 'outer;
                                    }
                                    if by != min_y - 1
                                        && bx != min_x
                                        && bx != max_x - 1
                                        && bz != min_z
                                        && bz != max_z - 1
                                    {
                                        by = min_y;
                                    }
                                }
                                by -= 1;
                            }
                        }
                    }

                    if !lava {
                        for bx in min_x..max_x {
                            let rel_x = ((bx + target_x * 16) as f64 + 0.5 - x) / radius;

                            for bz in min_z..max_z {
                                let rel_z = ((bz + target_z * 16) as f64 + 0.5 - z) / radius;
                                let mut index =
                                    block_index(bx as usize, max_y as usize, bz as usize);

                                let mut by = max_y - 1;
                                while by >= min_y {
                                    let rel_y = (by as f64 + 0.5 - y) / vertical_radius;

                                    if rel_y > -0.7
                                        && rel_x * rel_x + rel_y * rel_y + rel_z * rel_z < 1.0
                                    {
                                        let block = blocks[index];
                                        if block == blocks::NETHERRACK
                                            || block == blocks::DIRT
                                            || block == blocks::GRASS
                                        {
                                            blocks[index] = blocks::AIR;
                                        }
                                    }

                                    by -= 1;
                                    index -= 1;
                                }
                            }
                        }

                        if is_room {
                            break;
                        }
                    }
                }
            }

            step += 1;
        }
    }
}
