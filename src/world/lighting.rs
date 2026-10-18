use std::collections::VecDeque;

use crate::gameplay::blocks;
use crate::world::chunk::{Chunk, CHUNK_HEIGHT, CHUNK_WIDTH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightType {
    Sky,
    Block,
}

pub trait LightingWorld {
    fn get_block(&self, x: i32, y: i32, z: i32) -> u8;
    fn get_light(&self, light_type: LightType, x: i32, y: i32, z: i32) -> u8;
    fn set_light(&mut self, light_type: LightType, x: i32, y: i32, z: i32, val: u8);
    fn can_see_sky(&self, x: i32, y: i32, z: i32) -> bool;
    fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool;
}

const NEIGHBORS: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

pub fn init_chunk_lighting(chunk: &mut Chunk) {
    let mut max_height = 0usize;
    for x in 0..CHUNK_WIDTH {
        for z in 0..CHUNK_WIDTH {
            let mut height = CHUNK_HEIGHT as i32 - 1;
            while height > 0
                && blocks::light_opacity(chunk.get_block(x, height as usize - 1, z)) == 0
            {
                height -= 1;
            }
            chunk.height_map[(z << 4) | x] = height as u8;
            if (height as usize) > max_height {
                max_height = height as usize;
            }
        }
    }

    // Any sub-chunk completely at or above `max_height` has uniform sky_light = 15.
    // Setting its fill directly avoids allocating `Box<[u8; 2048]>` for air subchunks.
    let first_full_air_sub = max_height.div_ceil(16);
    for sub in first_full_air_sub..(CHUNK_HEIGHT / 16) {
        chunk.set_subchunk_sky_light_fill(sub, 15);
    }

    let top_y = (first_full_air_sub * 16).min(CHUNK_HEIGHT) as i32 - 1;
    if top_y <= 0 {
        return;
    }

    for x in 0..CHUNK_WIDTH {
        for z in 0..CHUNK_WIDTH {
            let mut level = 15i32;
            let mut y = top_y;
            loop {
                level -= blocks::light_opacity(chunk.get_block(x, y as usize, z)) as i32;
                if level > 0 {
                    chunk.set_sky_light(x, y as usize, z, level as u8);
                }
                y -= 1;
                if y <= 0 || level <= 0 {
                    break;
                }
            }
        }
    }
}

pub fn propagate_increase<W: LightingWorld>(
    world: &mut W,
    light_type: LightType,
    mut queue: VecDeque<(i32, i32, i32)>,
) {
    while let Some((x, y, z)) = queue.pop_front() {
        let current = world.get_light(light_type, x, y, z);
        if current <= 1 {
            continue;
        }

        for (dx, dy, dz) in NEIGHBORS {
            let nx = x + dx;
            let ny = y + dy;
            let nz = z + dz;

            if !(0..128).contains(&ny) {
                continue;
            }

            let ncx = nx >> 4;
            let ncz = nz >> 4;
            if !world.is_chunk_loaded(ncx, ncz) {
                continue;
            }

            let op = blocks::light_opacity(world.get_block(nx, ny, nz));
            if op >= 15 {
                continue;
            }

            let target = if light_type == LightType::Sky && world.can_see_sky(nx, ny, nz) {
                15
            } else {
                current.saturating_sub(op.max(1))
            };

            let neighbor_light = world.get_light(light_type, nx, ny, nz);
            if target > neighbor_light {
                world.set_light(light_type, nx, ny, nz, target);
                queue.push_back((nx, ny, nz));
            }
        }
    }
}

pub fn propagate_decrease<W: LightingWorld>(
    world: &mut W,
    light_type: LightType,
    mut dark_queue: VecDeque<(i32, i32, i32, u8)>,
) {
    let mut bright_queue = VecDeque::new();

    while let Some((x, y, z, old_level)) = dark_queue.pop_front() {
        for (dx, dy, dz) in NEIGHBORS {
            let nx = x + dx;
            let ny = y + dy;
            let nz = z + dz;

            if !(0..128).contains(&ny) {
                continue;
            }

            let ncx = nx >> 4;
            let ncz = nz >> 4;
            if !world.is_chunk_loaded(ncx, ncz) {
                continue;
            }

            let neighbor_light = world.get_light(light_type, nx, ny, nz);
            if neighbor_light > 0 && neighbor_light < old_level {
                world.set_light(light_type, nx, ny, nz, 0);
                dark_queue.push_back((nx, ny, nz, neighbor_light));
            } else if neighbor_light >= old_level {
                bright_queue.push_back((nx, ny, nz));
            }
        }
    }

    propagate_increase(world, light_type, bright_queue);
}

pub fn update_block_light<W: LightingWorld>(
    world: &mut W,
    x: i32,
    y: i32,
    z: i32,
    old_id: u8,
    new_id: u8,
) {
    let old_opacity = blocks::light_opacity(old_id);
    let new_opacity = blocks::light_opacity(new_id);
    let old_emission = blocks::light_emission(old_id);
    let new_emission = blocks::light_emission(new_id);

    // 1. Block light updates
    if old_emission != new_emission {
        if new_emission > old_emission {
            world.set_light(LightType::Block, x, y, z, new_emission);
            let mut q = VecDeque::new();
            q.push_back((x, y, z));
            propagate_increase(world, LightType::Block, q);
        } else {
            let mut dark_q = VecDeque::new();
            dark_q.push_back((x, y, z, old_emission));
            world.set_light(LightType::Block, x, y, z, new_emission);
            propagate_decrease(world, LightType::Block, dark_q);
            if new_emission > 0 {
                let mut q = VecDeque::new();
                q.push_back((x, y, z));
                propagate_increase(world, LightType::Block, q);
            }
        }
    } else if old_opacity != new_opacity {
        if new_opacity > old_opacity {
            let current = world.get_light(LightType::Block, x, y, z);
            if current > 0 {
                let mut dark_q = VecDeque::new();
                dark_q.push_back((x, y, z, current));
                world.set_light(LightType::Block, x, y, z, 0);
                propagate_decrease(world, LightType::Block, dark_q);
            }
        } else {
            let mut q = VecDeque::new();
            for (dx, dy, dz) in NEIGHBORS {
                let nx = x + dx;
                let ny = y + dy;
                let nz = z + dz;
                if (0..128).contains(&ny) && world.get_light(LightType::Block, nx, ny, nz) > 0 {
                    q.push_back((nx, ny, nz));
                }
            }
            propagate_increase(world, LightType::Block, q);
        }
    }

    // 2. Sky light updates
    if old_opacity != new_opacity {
        if new_opacity > old_opacity {
            let current = world.get_light(LightType::Sky, x, y, z);
            if current > 0 {
                let mut dark_q = VecDeque::new();
                dark_q.push_back((x, y, z, current));
                world.set_light(LightType::Sky, x, y, z, 0);
                propagate_decrease(world, LightType::Sky, dark_q);
            }
        } else {
            let mut q = VecDeque::new();
            if world.can_see_sky(x, y, z) {
                world.set_light(LightType::Sky, x, y, z, 15);
                q.push_back((x, y, z));
            } else {
                for (dx, dy, dz) in NEIGHBORS {
                    let nx = x + dx;
                    let ny = y + dy;
                    let nz = z + dz;
                    if (0..128).contains(&ny) && world.get_light(LightType::Sky, nx, ny, nz) > 0 {
                        q.push_back((nx, ny, nz));
                    }
                }
            }
            propagate_increase(world, LightType::Sky, q);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::chunk::ChunkPos;

    #[test]
    fn chunk_init_lighting_sunlight() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        // Flat stone at Y = 60
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, 60, z, blocks::STONE);
            }
        }

        init_chunk_lighting(&mut chunk);

        assert_eq!(chunk.get_height(0, 0), 61);
        assert_eq!(chunk.get_sky_light(0, 62, 0), 15);
        assert_eq!(chunk.get_sky_light(0, 61, 0), 15);
        assert_eq!(chunk.get_sky_light(0, 59, 0), 0);
    }
}
