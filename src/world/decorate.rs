use crate::gameplay::blocks;
use crate::gameplay::items::ItemStack;
use crate::gameplay::tile::TileEntity;
use crate::util::math;
use crate::util::rng::JavaRandom;
use crate::world::biome::{Biome, TreeKind};

pub trait GenWorld {
    fn get_block(&self, x: i32, y: i32, z: i32) -> u8;
    fn get_data(&self, x: i32, y: i32, z: i32) -> u8;
    fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8);
    fn set_block_and_data(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8);
    /// Java `World.setTypeId` — raw write plus physics propagation.
    fn set_block_physics(&mut self, x: i32, y: i32, z: i32, id: u8) {
        self.set_block(x, y, z, id);
    }
    /// Java `World.setTypeIdAndData` — raw write plus physics propagation.
    fn set_block_and_data_physics(&mut self, x: i32, y: i32, z: i32, id: u8, data: u8) {
        self.set_block_and_data(x, y, z, id, data);
    }
    /// Java `Block.byId[id].a(world, x, y, z, random)` for flowing fluids,
    /// invoked directly by `WorldGenLiquids`.
    fn fluid_tick(&mut self, x: i32, y: i32, z: i32, lava: bool, random: &mut JavaRandom) {
        let _ = (x, y, z, lava, random);
    }
    fn set_tile_entity(&mut self, _te: TileEntity) {}
    fn highest_block_y(&self, x: i32, z: i32) -> i32;
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32;
    fn light_value(&self, x: i32, y: i32, z: i32) -> i32;
    fn see_sky(&self, x: i32, y: i32, z: i32) -> bool;
    fn highest_solid_y(&self, x: i32, z: i32) -> i32;
    fn is_normal_solid(&self, x: i32, y: i32, z: i32) -> bool;
    fn biome_at(&self, x: i32, z: i32) -> Biome;

    fn is_empty(&self, x: i32, y: i32, z: i32) -> bool {
        self.get_block(x, y, z) == blocks::AIR
    }
}

fn flower_soil(id: u8) -> bool {
    matches!(id, blocks::GRASS | blocks::DIRT | blocks::FARMLAND)
}

fn flower_can_place(world: &dyn GenWorld, id: u8, x: i32, y: i32, z: i32) -> bool {
    if id == blocks::BROWN_MUSHROOM || id == blocks::RED_MUSHROOM {
        (0..128).contains(&y)
            && world.light_value(x, y, z) < 13
            && blocks::is_opaque_cube(world.get_block(x, y - 1, z))
    } else if id == blocks::DEAD_BUSH {
        (world.light_value(x, y, z) >= 8 || world.see_sky(x, y, z))
            && world.get_block(x, y - 1, z) == blocks::SAND
    } else {
        (world.light_value(x, y, z) >= 8 || world.see_sky(x, y, z))
            && flower_soil(world.get_block(x, y - 1, z))
    }
}

pub fn generate_lakes(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    block: u8,
) -> bool {
    let i = i - 8;
    let k = k - 8;
    let mut j = j;
    while j > 0 && world.is_empty(i, j, k) {
        j -= 1;
    }
    j -= 4;

    let mut ids = [false; 2048];
    let count = random.next_i32_bound(4) + 4;
    for _ in 0..count {
        let d0 = random.next_f64() * 6.0 + 3.0;
        let d1 = random.next_f64() * 4.0 + 2.0;
        let d2 = random.next_f64() * 6.0 + 3.0;
        let d3 = random.next_f64() * (16.0 - d0 - 2.0) + 1.0 + d0 / 2.0;
        let d4 = random.next_f64() * (8.0 - d1 - 4.0) + 2.0 + d1 / 2.0;
        let d5 = random.next_f64() * (16.0 - d2 - 2.0) + 1.0 + d2 / 2.0;

        for j1 in 1..15i32 {
            for k1 in 1..15i32 {
                for l1 in 1..7i32 {
                    let d6 = (j1 as f64 - d3) / (d0 / 2.0);
                    let d7 = (l1 as f64 - d4) / (d1 / 2.0);
                    let d8 = (k1 as f64 - d5) / (d2 / 2.0);
                    if d6 * d6 + d7 * d7 + d8 * d8 < 1.0 {
                        ids[((j1 * 16 + k1) * 8 + l1) as usize] = true;
                    }
                }
            }
        }
    }

    let edge = |i1: i32, i2: i32, j2: i32, ids: &[bool; 2048]| {
        !ids[((i1 * 16 + i2) * 8 + j2) as usize]
            && ((i1 < 15 && ids[(((i1 + 1) * 16 + i2) * 8 + j2) as usize])
                || (i1 > 0 && ids[(((i1 - 1) * 16 + i2) * 8 + j2) as usize])
                || (i2 < 15 && ids[((i1 * 16 + i2 + 1) * 8 + j2) as usize])
                || (i2 > 0 && ids[((i1 * 16 + (i2 - 1)) * 8 + j2) as usize])
                || (j2 < 7 && ids[((i1 * 16 + i2) * 8 + j2 + 1) as usize])
                || (j2 > 0 && ids[((i1 * 16 + i2) * 8 + (j2 - 1)) as usize]))
    };

    for i1 in 0..16i32 {
        for i2 in 0..16i32 {
            for j2 in 0..8i32 {
                if edge(i1, i2, j2, &ids) {
                    let id = world.get_block(i + i1, j + j2, k + i2);
                    if j2 >= 4 && blocks::is_liquid(id) {
                        return false;
                    }
                    if j2 < 4 && !blocks::is_solid(id) && id != block {
                        return false;
                    }
                }
            }
        }
    }

    for i1 in 0..16i32 {
        for i2 in 0..16i32 {
            for j2 in 0..8i32 {
                if ids[((i1 * 16 + i2) * 8 + j2) as usize] {
                    let id = if j2 >= 4 { blocks::AIR } else { block };
                    world.set_block(i + i1, j + j2, k + i2, id);
                }
            }
        }
    }

    for i1 in 0..16i32 {
        for i2 in 0..16i32 {
            for j2 in 4..8i32 {
                if ids[((i1 * 16 + i2) * 8 + j2) as usize]
                    && world.get_block(i + i1, j + j2 - 1, k + i2) == blocks::DIRT
                    && world.sky_light(i + i1, j + j2, k + i2) > 0
                {
                    world.set_block(i + i1, j + j2 - 1, k + i2, blocks::GRASS);
                }
            }
        }
    }

    if matches!(block, blocks::LAVA | blocks::STATIONARY_LAVA) {
        for i1 in 0..16i32 {
            for i2 in 0..16i32 {
                for j2 in 0..8i32 {
                    if edge(i1, i2, j2, &ids)
                        && (j2 < 4 || random.next_i32_bound(2) != 0)
                        && blocks::is_solid(world.get_block(i + i1, j + j2, k + i2))
                    {
                        world.set_block(i + i1, j + j2, k + i2, blocks::STONE);
                    }
                }
            }
        }
    }

    true
}

fn dungeon_loot(random: &mut JavaRandom) -> Option<(usize, ItemStack)> {
    let i = random.next_i32_bound(11);
    let item = match i {
        0 => Some(ItemStack::new(329, 1, 0)),
        1 => {
            let count = (random.next_i32_bound(4) + 1) as u8;
            Some(ItemStack::new(265, count, 0))
        }
        2 => Some(ItemStack::new(297, 1, 0)),
        3 => {
            let count = (random.next_i32_bound(4) + 1) as u8;
            Some(ItemStack::new(296, count, 0))
        }
        4 => {
            let count = (random.next_i32_bound(4) + 1) as u8;
            Some(ItemStack::new(289, count, 0))
        }
        5 => {
            let count = (random.next_i32_bound(4) + 1) as u8;
            Some(ItemStack::new(287, count, 0))
        }
        6 => Some(ItemStack::new(325, 1, 0)),
        7 => {
            if random.next_i32_bound(100) == 0 {
                Some(ItemStack::new(322, 1, 0))
            } else {
                None
            }
        }
        8 => {
            if random.next_i32_bound(2) == 0 {
                let count = (random.next_i32_bound(4) + 1) as u8;
                Some(ItemStack::new(331, count, 0))
            } else {
                None
            }
        }
        9 => {
            if random.next_i32_bound(10) == 0 {
                let rec = (2256 + random.next_i32_bound(2)) as i16;
                Some(ItemStack::new(rec, 1, 0))
            } else {
                None
            }
        }
        10 => Some(ItemStack::new(351, 1, 3)),
        _ => None,
    };
    if let Some(stack) = item {
        let slot = random.next_i32_bound(27) as usize;
        Some((slot, stack))
    } else {
        None
    }
}

pub fn generate_dungeons(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    let b0 = 3i32;
    let l = random.next_i32_bound(2) + 2;
    let i1 = random.next_i32_bound(2) + 2;
    let mut j1 = 0i32;

    for k1 in i - l - 1..=i + l + 1 {
        for l1 in j - 1..=j + b0 + 1 {
            for i2 in k - i1 - 1..=k + i1 + 1 {
                if l1 == j - 1 && !blocks::is_solid(world.get_block(k1, l1, i2)) {
                    return false;
                }
                if l1 == j + b0 + 1 && !blocks::is_solid(world.get_block(k1, l1, i2)) {
                    return false;
                }
                if (k1 == i - l - 1 || k1 == i + l + 1 || i2 == k - i1 - 1 || i2 == k + i1 + 1)
                    && l1 == j
                    && world.is_empty(k1, l1, i2)
                    && world.is_empty(k1, l1 + 1, i2)
                {
                    j1 += 1;
                }
            }
        }
    }

    if !(1..=5).contains(&j1) {
        return false;
    }

    for k1 in i - l - 1..=i + l + 1 {
        for l1 in (j - 1..=j + b0).rev() {
            for i2 in k - i1 - 1..=k + i1 + 1 {
                if (k1 != i - l - 1
                    && l1 != j - 1
                    && i2 != k - i1 - 1
                    && k1 != i + l + 1
                    && l1 != j + b0 + 1
                    && i2 != k + i1 + 1)
                    || (l1 >= 0 && !blocks::is_solid(world.get_block(k1, l1 - 1, i2)))
                {
                    world.set_block_physics(k1, l1, i2, blocks::AIR);
                } else if blocks::is_solid(world.get_block(k1, l1, i2)) {
                    if l1 == j - 1 && random.next_i32_bound(4) != 0 {
                        world.set_block_physics(k1, l1, i2, blocks::MOSSY_COBBLESTONE);
                    } else {
                        world.set_block_physics(k1, l1, i2, blocks::COBBLESTONE);
                    }
                }
            }
        }
    }

    for chest in 0..2 {
        let _ = chest;
        let mut placed = false;
        for _ in 0..3 {
            let i2 = i + random.next_i32_bound(l * 2 + 1) - l;
            let j2 = k + random.next_i32_bound(i1 * 2 + 1) - i1;
            if world.is_empty(i2, j, j2) {
                let mut k2 = 0;
                if blocks::is_solid(world.get_block(i2 - 1, j, j2)) {
                    k2 += 1;
                }
                if blocks::is_solid(world.get_block(i2 + 1, j, j2)) {
                    k2 += 1;
                }
                if blocks::is_solid(world.get_block(i2, j, j2 - 1)) {
                    k2 += 1;
                }
                if blocks::is_solid(world.get_block(i2, j, j2 + 1)) {
                    k2 += 1;
                }
                if k2 == 1 {
                    world.set_block_physics(i2, j, j2, blocks::CHEST);
                    let mut chest_te = TileEntity::new_chest(i2, j, j2);
                    for _ in 0..8 {
                        if let Some((slot, stack)) = dungeon_loot(random) {
                            if let crate::gameplay::tile::TileEntityKind::Chest { items } =
                                &mut chest_te.kind
                            {
                                if slot < 27 {
                                    items[slot] = Some(stack);
                                }
                            }
                        }
                    }
                    world.set_tile_entity(chest_te);
                    placed = true;
                    break;
                }
            }
        }
        if placed {
            continue;
        }
    }

    world.set_block_physics(i, j, k, blocks::MOB_SPAWNER);
    let spawner_mob = match random.next_i32_bound(4) {
        0 => "Skeleton",
        1 | 2 => "Zombie",
        _ => "Spider",
    };
    world.set_tile_entity(TileEntity::new_spawner(i, j, k, spawner_mob.into()));
    true
}

pub fn generate_minable(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    block: u8,
    size: i32,
) -> bool {
    let f = random.next_f32() * std::f32::consts::PI;
    let d0 = ((i + 8) as f32 + math::sin(f) * size as f32 / 8.0) as f64;
    let d1 = ((i + 8) as f32 - math::sin(f) * size as f32 / 8.0) as f64;
    let d2 = ((k + 8) as f32 + math::cos(f) * size as f32 / 8.0) as f64;
    let d3 = ((k + 8) as f32 - math::cos(f) * size as f32 / 8.0) as f64;
    let d4 = (j + random.next_i32_bound(3) + 2) as f64;
    let d5 = (j + random.next_i32_bound(3) + 2) as f64;

    for l in 0..=size {
        let d6 = d0 + (d1 - d0) * l as f64 / size as f64;
        let d7 = d4 + (d5 - d4) * l as f64 / size as f64;
        let d8 = d2 + (d3 - d2) * l as f64 / size as f64;
        let d9 = random.next_f64() * size as f64 / 16.0;
        let d10 =
            (math::sin(l as f32 * std::f32::consts::PI / size as f32) + 1.0) as f64 * d9 + 1.0;
        let d11 =
            (math::sin(l as f32 * std::f32::consts::PI / size as f32) + 1.0) as f64 * d9 + 1.0;
        let i1 = math::floor_double(d6 - d10 / 2.0);
        let j1 = math::floor_double(d7 - d11 / 2.0);
        let k1 = math::floor_double(d8 - d10 / 2.0);
        let l1 = math::floor_double(d6 + d10 / 2.0);
        let i2 = math::floor_double(d7 + d11 / 2.0);
        let j2 = math::floor_double(d8 + d10 / 2.0);

        for k2 in i1..=l1 {
            let d12 = (k2 as f64 + 0.5 - d6) / (d10 / 2.0);
            if d12 * d12 < 1.0 {
                for l2 in j1..=i2 {
                    let d13 = (l2 as f64 + 0.5 - d7) / (d11 / 2.0);
                    if d12 * d12 + d13 * d13 < 1.0 {
                        for i3 in k1..=j2 {
                            let d14 = (i3 as f64 + 0.5 - d8) / (d10 / 2.0);
                            if d12 * d12 + d13 * d13 + d14 * d14 < 1.0
                                && world.get_block(k2, l2, i3) == blocks::STONE
                            {
                                world.set_block(k2, l2, i3, block);
                            }
                        }
                    }
                }
            }
        }
    }

    true
}

pub fn generate_clay(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    size: i32,
) -> bool {
    if !blocks::is_water(world.get_block(i, j, k)) {
        return false;
    }
    let f = random.next_f32() * std::f32::consts::PI;
    let d0 = ((i + 8) as f32 + math::sin(f) * size as f32 / 8.0) as f64;
    let d1 = ((i + 8) as f32 - math::sin(f) * size as f32 / 8.0) as f64;
    let d2 = ((k + 8) as f32 + math::cos(f) * size as f32 / 8.0) as f64;
    let d3 = ((k + 8) as f32 - math::cos(f) * size as f32 / 8.0) as f64;
    let d4 = (j + random.next_i32_bound(3) + 2) as f64;
    let d5 = (j + random.next_i32_bound(3) + 2) as f64;

    for l in 0..=size {
        let d6 = d0 + (d1 - d0) * l as f64 / size as f64;
        let d7 = d4 + (d5 - d4) * l as f64 / size as f64;
        let d8 = d2 + (d3 - d2) * l as f64 / size as f64;
        let d9 = random.next_f64() * size as f64 / 16.0;
        let d10 =
            (math::sin(l as f32 * std::f32::consts::PI / size as f32) + 1.0) as f64 * d9 + 1.0;
        let d11 =
            (math::sin(l as f32 * std::f32::consts::PI / size as f32) + 1.0) as f64 * d9 + 1.0;
        let i1 = math::floor_double(d6 - d10 / 2.0);
        let j1 = math::floor_double(d6 + d10 / 2.0);
        let k1 = math::floor_double(d7 - d11 / 2.0);
        let l1 = math::floor_double(d7 + d11 / 2.0);
        let i2 = math::floor_double(d8 - d10 / 2.0);
        let j2 = math::floor_double(d8 + d10 / 2.0);

        for k2 in i1..=j1 {
            for l2 in k1..=l1 {
                for i3 in i2..=j2 {
                    let d12 = (k2 as f64 + 0.5 - d6) / (d10 / 2.0);
                    let d13 = (l2 as f64 + 0.5 - d7) / (d11 / 2.0);
                    let d14 = (i3 as f64 + 0.5 - d8) / (d10 / 2.0);
                    if d12 * d12 + d13 * d13 + d14 * d14 < 1.0
                        && world.get_block(k2, l2, i3) == blocks::SAND
                    {
                        world.set_block(k2, l2, i3, blocks::CLAY);
                    }
                }
            }
        }
    }

    true
}

pub fn generate_flowers(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    block: u8,
) -> bool {
    for _ in 0..64 {
        let i1 = i + random.next_i32_bound(8) - random.next_i32_bound(8);
        let j1 = j + random.next_i32_bound(4) - random.next_i32_bound(4);
        let k1 = k + random.next_i32_bound(8) - random.next_i32_bound(8);
        if world.is_empty(i1, j1, k1) && flower_can_place(world, block, i1, j1, k1) {
            world.set_block(i1, j1, k1, block);
        }
    }
    true
}

pub fn generate_grass(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    block: u8,
    data: u8,
) -> bool {
    let mut j = j;
    loop {
        let id = world.get_block(i, j, k);
        if !((id == blocks::AIR || id == blocks::LEAVES) && j > 0) {
            break;
        }
        j -= 1;
    }

    for _ in 0..128 {
        let j1 = i + random.next_i32_bound(8) - random.next_i32_bound(8);
        let k1 = j + random.next_i32_bound(4) - random.next_i32_bound(4);
        let l1 = k + random.next_i32_bound(8) - random.next_i32_bound(8);
        if world.is_empty(j1, k1, l1) && flower_can_place(world, block, j1, k1, l1) {
            world.set_block_and_data(j1, k1, l1, block, data);
        }
    }
    true
}

pub fn generate_dead_bush(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    block: u8,
) -> bool {
    let mut j = j;
    loop {
        let id = world.get_block(i, j, k);
        if !((id == blocks::AIR || id == blocks::LEAVES) && j > 0) {
            break;
        }
        j -= 1;
    }

    for _ in 0..4 {
        let j1 = i + random.next_i32_bound(8) - random.next_i32_bound(8);
        let k1 = j + random.next_i32_bound(4) - random.next_i32_bound(4);
        let l1 = k + random.next_i32_bound(8) - random.next_i32_bound(8);
        if world.is_empty(j1, k1, l1) && flower_can_place(world, block, j1, k1, l1) {
            world.set_block(j1, k1, l1, block);
        }
    }
    true
}

fn reed_can_place(world: &dyn GenWorld, x: i32, y: i32, z: i32) -> bool {
    let below = world.get_block(x, y - 1, z);
    if below == blocks::REED {
        return true;
    }
    if below != blocks::GRASS && below != blocks::DIRT {
        return false;
    }
    blocks::is_water(world.get_block(x - 1, y - 1, z))
        || blocks::is_water(world.get_block(x + 1, y - 1, z))
        || blocks::is_water(world.get_block(x, y - 1, z - 1))
        || blocks::is_water(world.get_block(x, y - 1, z + 1))
}

pub fn generate_reed(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    for _ in 0..20 {
        let i1 = i + random.next_i32_bound(4) - random.next_i32_bound(4);
        let j1 = j;
        let k1 = k + random.next_i32_bound(4) - random.next_i32_bound(4);
        if world.is_empty(i1, j, k1)
            && (blocks::is_water(world.get_block(i1 - 1, j - 1, k1))
                || blocks::is_water(world.get_block(i1 + 1, j - 1, k1))
                || blocks::is_water(world.get_block(i1, j - 1, k1 - 1))
                || blocks::is_water(world.get_block(i1, j - 1, k1 + 1)))
        {
            let inner = random.next_i32_bound(3) + 1;
            let l1 = 2 + random.next_i32_bound(inner);
            for i2 in 0..l1 {
                if reed_can_place(world, i1, j1 + i2, k1) {
                    world.set_block(i1, j1 + i2, k1, blocks::REED);
                }
            }
        }
    }
    true
}

fn cactus_can_place(world: &dyn GenWorld, x: i32, y: i32, z: i32) -> bool {
    if blocks::is_solid(world.get_block(x - 1, y, z))
        || blocks::is_solid(world.get_block(x + 1, y, z))
        || blocks::is_solid(world.get_block(x, y, z - 1))
        || blocks::is_solid(world.get_block(x, y, z + 1))
    {
        return false;
    }
    let below = world.get_block(x, y - 1, z);
    below == blocks::CACTUS || below == blocks::SAND
}

pub fn generate_cactus(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    for _ in 0..10 {
        let i1 = i + random.next_i32_bound(8) - random.next_i32_bound(8);
        let j1 = j + random.next_i32_bound(4) - random.next_i32_bound(4);
        let k1 = k + random.next_i32_bound(8) - random.next_i32_bound(8);
        if world.is_empty(i1, j1, k1) {
            let inner = random.next_i32_bound(3) + 1;
            let l1 = 1 + random.next_i32_bound(inner);
            for i2 in 0..l1 {
                if cactus_can_place(world, i1, j1 + i2, k1) {
                    world.set_block(i1, j1 + i2, k1, blocks::CACTUS);
                }
            }
        }
    }
    true
}

pub fn generate_pumpkin(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    for _ in 0..64 {
        let i1 = i + random.next_i32_bound(8) - random.next_i32_bound(8);
        let j1 = j + random.next_i32_bound(4) - random.next_i32_bound(4);
        let k1 = k + random.next_i32_bound(8) - random.next_i32_bound(8);
        if world.is_empty(i1, j1, k1)
            && world.get_block(i1, j1 - 1, k1) == blocks::GRASS
            && (world.get_block(i1, j1, k1) == blocks::AIR
                || blocks::is_replacable(world.get_block(i1, j1, k1)))
            && world.is_normal_solid(i1, j1 - 1, k1)
        {
            world.set_block_and_data(i1, j1, k1, blocks::PUMPKIN, random.next_i32_bound(4) as u8);
        }
    }
    true
}

pub fn generate_liquids(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
    block: u8,
) -> bool {
    if world.get_block(i, j + 1, k) != blocks::STONE {
        return false;
    }
    if world.get_block(i, j - 1, k) != blocks::STONE {
        return false;
    }
    let here = world.get_block(i, j, k);
    if here != blocks::AIR && here != blocks::STONE {
        return false;
    }
    let mut stone = 0;
    if world.get_block(i - 1, j, k) == blocks::STONE {
        stone += 1;
    }
    if world.get_block(i + 1, j, k) == blocks::STONE {
        stone += 1;
    }
    if world.get_block(i, j, k - 1) == blocks::STONE {
        stone += 1;
    }
    if world.get_block(i, j, k + 1) == blocks::STONE {
        stone += 1;
    }
    let mut air = 0;
    if world.is_empty(i - 1, j, k) {
        air += 1;
    }
    if world.is_empty(i + 1, j, k) {
        air += 1;
    }
    if world.is_empty(i, j, k - 1) {
        air += 1;
    }
    if world.is_empty(i, j, k + 1) {
        air += 1;
    }
    if stone == 3 && air == 1 {
        // Java WorldGenLiquids: place with full physics, then tick the fluid
        // once directly (`world.a` is a no-op flag in Poseidon).
        world.set_block_physics(i, j, k, block);
        world.fluid_tick(i, j, k, block == blocks::LAVA, random);
    }
    true
}

fn tree_base_ok(id: u8) -> bool {
    id == blocks::GRASS || id == blocks::DIRT
}

pub fn generate_trees(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    let l = random.next_i32_bound(3) + 4;
    let mut flag = true;
    if !(j >= 1 && j + l < 128) {
        return false;
    }

    let mut i1 = j;
    'outer: while i1 <= j + 1 + l {
        let b0 = if i1 == j {
            0
        } else if i1 >= j + 1 + l - 2 {
            2
        } else {
            1
        };
        let mut j1 = i - b0;
        while j1 <= i + b0 && flag {
            let mut k1 = k - b0;
            while k1 <= k + b0 && flag {
                if (0..128).contains(&i1) {
                    let id = world.get_block(j1, i1, k1);
                    if id != blocks::AIR && id != blocks::LEAVES {
                        flag = false;
                    }
                } else {
                    flag = false;
                }
                k1 += 1;
            }
            j1 += 1;
        }
        if !flag {
            break 'outer;
        }
        i1 += 1;
    }

    if !flag {
        return false;
    }

    let below = world.get_block(i, j - 1, k);
    if !(tree_base_ok(below) && j < 128 - l - 1) {
        return false;
    }
    world.set_block(i, j - 1, k, blocks::DIRT);

    let mut i2 = j - 3 + l;
    while i2 <= j + l {
        let j1 = i2 - (j + l);
        let k1 = 1 - j1 / 2;
        for l1 in i - k1..=i + k1 {
            let j2 = l1 - i;
            for k2 in k - k1..=k + k1 {
                let l2 = k2 - k;
                if (j2.abs() != k1 || l2.abs() != k1 || random.next_i32_bound(2) != 0 && j1 != 0)
                    && !blocks::is_opaque_cube(world.get_block(l1, i2, k2))
                    && !blocks::leaf_decay_blacklist(world.get_block(l1, i2, k2))
                {
                    world.set_block(l1, i2, k2, blocks::LEAVES);
                }
            }
        }
        i2 += 1;
    }

    for i2 in 0..l {
        let j1 = world.get_block(i, j + i2, k);
        if j1 == blocks::AIR || j1 == blocks::LEAVES {
            world.set_block(i, j + i2, k, blocks::LOG);
        }
    }
    true
}

pub fn generate_forest(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    let l = random.next_i32_bound(3) + 5;
    let mut flag = true;
    if !(j >= 1 && j + l < 128) {
        return false;
    }

    let mut i1 = j;
    'outer: while i1 <= j + 1 + l {
        let b0 = if i1 == j {
            0
        } else if i1 >= j + 1 + l - 2 {
            2
        } else {
            1
        };
        let mut j1 = i - b0;
        while j1 <= i + b0 && flag {
            let mut k1 = k - b0;
            while k1 <= k + b0 && flag {
                if (0..128).contains(&i1) {
                    let id = world.get_block(j1, i1, k1);
                    if id != blocks::AIR && id != blocks::LEAVES {
                        flag = false;
                    }
                } else {
                    flag = false;
                }
                k1 += 1;
            }
            j1 += 1;
        }
        if !flag {
            break 'outer;
        }
        i1 += 1;
    }

    if !flag {
        return false;
    }

    let below = world.get_block(i, j - 1, k);
    if !(tree_base_ok(below) && j < 128 - l - 1) {
        return false;
    }
    world.set_block(i, j - 1, k, blocks::DIRT);

    let mut i2 = j - 3 + l;
    while i2 <= j + l {
        let j1 = i2 - (j + l);
        let k1 = 1 - j1 / 2;
        for l1 in i - k1..=i + k1 {
            let j2 = l1 - i;
            for k2 in k - k1..=k + k1 {
                let l2 = k2 - k;
                if (j2.abs() != k1 || l2.abs() != k1 || random.next_i32_bound(2) != 0 && j1 != 0)
                    && !blocks::is_opaque_cube(world.get_block(l1, i2, k2))
                {
                    world.set_block_and_data(l1, i2, k2, blocks::LEAVES, 2);
                }
            }
        }
        i2 += 1;
    }

    for i2 in 0..l {
        let j1 = world.get_block(i, j + i2, k);
        if j1 == blocks::AIR || j1 == blocks::LEAVES {
            world.set_block_and_data(i, j + i2, k, blocks::LOG, 2);
        }
    }
    true
}

pub fn generate_taiga1(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    let l = random.next_i32_bound(5) + 7;
    let i1 = l - random.next_i32_bound(2) - 3;
    let j1 = l - i1;
    let k1 = 1 + random.next_i32_bound(j1 + 1);
    let mut flag = true;
    if !(j >= 1 && j + l < 128) {
        return false;
    }

    let mut l1 = j;
    while l1 <= j + 1 + l && flag {
        let l2 = if l1 - j < i1 { 0 } else { k1 };
        let mut i2 = i - l2;
        while i2 <= i + l2 && flag {
            let mut j2 = k - l2;
            while j2 <= k + l2 && flag {
                if (0..128).contains(&l1) {
                    let k2 = world.get_block(i2, l1, j2);
                    if k2 != blocks::AIR && k2 != blocks::LEAVES {
                        flag = false;
                    }
                } else {
                    flag = false;
                }
                j2 += 1;
            }
            i2 += 1;
        }
        l1 += 1;
    }

    if !flag {
        return false;
    }

    let ground = world.get_block(i, j - 1, k);
    if !(tree_base_ok(ground) && j < 128 - l - 1) {
        return false;
    }
    world.set_block(i, j - 1, k, blocks::DIRT);

    let mut l2 = 0;
    let mut i2 = j + l;
    while i2 >= j + i1 {
        let mut j2 = i - l2;
        while j2 <= i + l2 {
            let k2 = j2 - i;
            let mut i3 = k - l2;
            while i3 <= k + l2 {
                let j3 = i3 - k;
                if (k2.abs() != l2 || j3.abs() != l2 || l2 <= 0)
                    && !blocks::is_opaque_cube(world.get_block(j2, i2, i3))
                    && !blocks::leaf_decay_blacklist(world.get_block(ground as i32, i2, k2))
                {
                    world.set_block_and_data(j2, i2, i3, blocks::LEAVES, 1);
                }
                i3 += 1;
            }
            j2 += 1;
        }
        if l2 >= 1 && i2 == j + i1 + 1 {
            l2 -= 1;
        } else if l2 < k1 {
            l2 += 1;
        }
        i2 -= 1;
    }

    for i2 in 0..l - 1 {
        let j2 = world.get_block(i, j + i2, k);
        if j2 == blocks::AIR || j2 == blocks::LEAVES {
            world.set_block_and_data(i, j + i2, k, blocks::LOG, 1);
        }
    }
    true
}

pub fn generate_taiga2(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    i: i32,
    j: i32,
    k: i32,
) -> bool {
    let l = random.next_i32_bound(4) + 6;
    let i1 = 1 + random.next_i32_bound(2);
    let j1 = l - i1;
    let k1 = 2 + random.next_i32_bound(2);
    let mut flag = true;
    if !(j >= 1 && j + l < 128) {
        return false;
    }

    let mut l1 = j;
    while l1 <= j + 1 + l && flag {
        let k2 = if l1 - j < i1 { 0 } else { k1 };
        let mut i2 = i - k2;
        while i2 <= i + k2 && flag {
            let mut l2 = k - k2;
            while l2 <= k + k2 && flag {
                if (0..128).contains(&l1) {
                    let j2 = world.get_block(i2, l1, l2);
                    if j2 != blocks::AIR && j2 != blocks::LEAVES {
                        flag = false;
                    }
                } else {
                    flag = false;
                }
                l2 += 1;
            }
            i2 += 1;
        }
        l1 += 1;
    }

    if !flag {
        return false;
    }

    let ground = world.get_block(i, j - 1, k);
    if !(tree_base_ok(ground) && j < 128 - l - 1) {
        return false;
    }
    world.set_block(i, j - 1, k, blocks::DIRT);

    let mut k2 = random.next_i32_bound(2);
    let mut i2 = 1;
    let mut b0 = 0;
    let mut j2 = 0;
    while j2 <= j1 {
        let j3 = j + l - j2;
        let mut i3 = i - k2;
        while i3 <= i + k2 {
            let k3 = i3 - i;
            let mut l3 = k - k2;
            while l3 <= k + k2 {
                let i4 = l3 - k;
                if (k3.abs() != k2 || i4.abs() != k2 || k2 <= 0)
                    && !blocks::is_opaque_cube(world.get_block(i3, j3, l3))
                    && !blocks::leaf_decay_blacklist(world.get_block(ground as i32, i2, k2))
                {
                    world.set_block_and_data(i3, j3, l3, blocks::LEAVES, 1);
                }
                l3 += 1;
            }
            i3 += 1;
        }
        if k2 >= i2 {
            k2 = b0;
            b0 = 1;
            i2 += 1;
            if i2 > k1 {
                i2 = k1;
            }
        } else {
            k2 += 1;
        }
        j2 += 1;
    }

    j2 = random.next_i32_bound(3);
    for j3 in 0..l - j2 {
        let i3 = world.get_block(i, j + j3, k);
        if i3 == blocks::AIR || i3 == blocks::LEAVES {
            world.set_block_and_data(i, j + j3, k, blocks::LOG, 1);
        }
    }
    true
}

const BIG_TREE_A: [i32; 6] = [2, 0, 0, 1, 2, 1];

pub struct BigTree {
    rng: JavaRandom,
    base: [i32; 3],
    height: i32,
    trunk: i32,
    leaf_depth: i32,
    height_limit: i32,
    scale_width: f64,
    scale_height: f64,
    branch: Vec<[i32; 4]>,
}

impl Default for BigTree {
    fn default() -> Self {
        Self::new()
    }
}

impl BigTree {
    pub fn new() -> Self {
        Self {
            rng: JavaRandom::new(0),
            base: [0, 0, 0],
            height: 0,
            trunk: 0,
            leaf_depth: 4,
            height_limit: 12,
            scale_width: 1.0,
            scale_height: 1.0,
            branch: Vec::new(),
        }
    }

    pub fn configure(&mut self, d0: f64, d1: f64, d2: f64) {
        self.height_limit = (d0 * 12.0) as i32;
        if d0 > 0.5 {
            self.leaf_depth = 5;
        }
        self.scale_width = d1;
        self.scale_height = d2;
    }

    // Beta 1.7.3 WorldGenBigTree.java:61 explicitly uses literal 3.14159D, not Math.PI.
    #[allow(clippy::approx_constant)]
    fn init_branches(&mut self, world: &dyn GenWorld) {
        let g = 0.618;
        let i_const = 0.381;
        let mut trunk = (self.height as f64 * g) as i32;
        if trunk >= self.height {
            trunk = self.height - 1;
        }
        self.trunk = trunk;

        let mut i = (1.382 + (self.scale_height * self.height as f64 / 13.0).powf(2.0)) as i32;
        if i < 1 {
            i = 1;
        }

        let mut aint = vec![[0i32; 4]; (i * self.height) as usize];
        let mut j = self.base[1] + self.height - self.leaf_depth;
        let mut k = 1usize;
        let l = self.base[1] + self.trunk;
        let mut i1 = j - self.base[1];

        aint[0] = [self.base[0], j, self.base[2], l];
        j -= 1;

        while i1 >= 0 {
            let f = self.leaf_radius(i1);
            if f < 0.0 {
                j -= 1;
                i1 -= 1;
                continue;
            }

            let mut j1 = 0i32;
            while j1 < i {
                let d1 = self.scale_width * f as f64 * (self.rng.next_f32() as f64 + 0.328);
                let d2 = self.rng.next_f32() as f64 * 2.0 * 3.14159;
                let k1 = math::floor_double(d1 * d2.sin() + self.base[0] as f64 + 0.5);
                let l1 = math::floor_double(d1 * d2.cos() + self.base[2] as f64 + 0.5);
                let aint1 = [k1, j, l1];
                let aint2 = [k1, j + self.leaf_depth, l1];

                if self.distance(world, &aint1, &aint2) == -1 {
                    let mut aint3 = [self.base[0], self.base[1], self.base[2]];
                    let d3 = (((self.base[0] - aint1[0]).abs() as f64).powf(2.0)
                        + ((self.base[2] - aint1[2]).abs() as f64).powf(2.0))
                    .sqrt();
                    let d4 = d3 * i_const;
                    if aint1[1] as f64 - d4 > l as f64 {
                        aint3[1] = l;
                    } else {
                        aint3[1] = (aint1[1] as f64 - d4) as i32;
                    }

                    if self.distance(world, &aint3, &aint1) == -1 {
                        aint[k] = [k1, j, l1, aint3[1]];
                        k += 1;
                    }
                }
                j1 += 1;
            }

            j -= 1;
            i1 -= 1;
        }

        self.branch = aint[..k].to_vec();
    }

    fn leaf_radius(&self, i: i32) -> f32 {
        if (i as f64) < self.height as f64 * 0.3 {
            -1.618
        } else {
            let f = self.height as f32 / 2.0;
            let f1 = self.height as f32 / 2.0 - i as f32;
            let f2 = if f1 == 0.0 {
                f
            } else if f1.abs() >= f {
                0.0
            } else {
                (((f.abs() as f64).powf(2.0) - (f1.abs() as f64).powf(2.0)).sqrt()) as f32
            };
            f2 * 0.5
        }
    }

    fn leaf_height(&self, i: i32) -> f32 {
        if (0..self.leaf_depth).contains(&i) {
            if i != 0 && i != self.leaf_depth - 1 {
                3.0
            } else {
                2.0
            }
        } else {
            -1.0
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn leaf_blob(
        &self,
        world: &mut dyn GenWorld,
        i: i32,
        j: i32,
        k: i32,
        f: f32,
        b0: usize,
        l: u8,
    ) {
        let i1 = (f as f64 + 0.618) as i32;
        let b1 = BIG_TREE_A[b0] as usize;
        let b2 = BIG_TREE_A[b0 + 3] as usize;
        let aint = [i, j, k];
        let mut aint1 = [0i32; 3];
        aint1[b0] = aint[b0];

        let mut j1 = -i1;
        while j1 <= i1 {
            aint1[b1] = aint[b1] + j1;
            let mut k1 = -i1;
            while k1 <= i1 {
                let d0 = (((j1.abs() as f64) + 0.5).powf(2.0)
                    + ((k1.abs() as f64) + 0.5).powf(2.0))
                .sqrt();
                if d0 > f as f64 {
                    k1 += 1;
                } else {
                    aint1[b2] = aint[b2] + k1;
                    let l1 = world.get_block(aint1[0], aint1[1], aint1[2]);
                    if l1 != blocks::AIR && l1 != blocks::LEAVES {
                        k1 += 1;
                    } else {
                        world.set_block(aint1[0], aint1[1], aint1[2], l);
                        k1 += 1;
                    }
                }
            }
            j1 += 1;
        }
    }

    fn leaf_column(&mut self, world: &mut dyn GenWorld, i: i32, j: i32, k: i32) {
        let mut l = j;
        let i1 = j + self.leaf_depth;
        while l < i1 {
            let f = self.leaf_height(l - j);
            self.leaf_blob(world, i, l, k, f, 1, blocks::LEAVES);
            l += 1;
        }
    }

    fn draw_branches(&mut self, world: &mut dyn GenWorld) {
        for index in 0..self.branch.len() {
            let b = self.branch[index];
            self.leaf_column(world, b[0], b[1], b[2]);
        }
    }

    fn draw_trunk(&self, world: &mut dyn GenWorld) {
        let aint = [self.base[0], self.base[1], self.base[2]];
        let aint1 = [self.base[0], self.base[1] + self.trunk, self.base[2]];
        line(world, aint, aint1, blocks::LOG);
    }

    fn draw_branch_bases(&self, world: &mut dyn GenWorld) {
        let mut aint = [self.base[0], self.base[1], self.base[2]];
        for index in 0..self.branch.len() {
            let b = self.branch[index];
            let aint2 = [b[0], b[1], b[2]];
            aint[1] = b[3];
            let k = aint[1] - self.base[1];
            if (k as f64) >= self.height as f64 * 0.2 {
                line(world, aint, aint2, blocks::LOG);
            }
        }
    }

    fn distance(&self, world: &dyn GenWorld, aint: &[i32; 3], aint1: &[i32; 3]) -> i32 {
        let mut aint2 = [0i32; 3];
        let mut b1 = 0usize;
        for b0 in 0..3usize {
            aint2[b0] = aint1[b0] - aint[b0];
            if aint2[b0].abs() > aint2[b1].abs() {
                b1 = b0;
            }
        }

        if aint2[b1] == 0 {
            return -1;
        }
        let b2 = BIG_TREE_A[b1] as usize;
        let b3 = BIG_TREE_A[b1 + 3] as usize;
        let b4 = if aint2[b1] > 0 { 1 } else { -1 };
        let d0 = aint2[b2] as f64 / aint2[b1] as f64;
        let d1 = aint2[b3] as f64 / aint2[b1] as f64;
        let mut aint3 = [0i32; 3];
        let mut i = 0;
        let j = aint2[b1] + b4;
        while i != j {
            aint3[b1] = aint[b1] + i;
            aint3[b2] = math::floor_double(aint[b2] as f64 + i as f64 * d0);
            aint3[b3] = math::floor_double(aint[b3] as f64 + i as f64 * d1);
            let k = world.get_block(aint3[0], aint3[1], aint3[2]);
            if k != blocks::AIR && k != blocks::LEAVES {
                break;
            }
            i += b4;
        }

        if i == j {
            -1
        } else {
            i.abs()
        }
    }

    fn check_height(&mut self, world: &dyn GenWorld) -> bool {
        let aint = [self.base[0], self.base[1], self.base[2]];
        let aint1 = [self.base[0], self.base[1] + self.height - 1, self.base[2]];
        let i = world.get_block(self.base[0], self.base[1] - 1, self.base[2]);
        if i != blocks::GRASS && i != blocks::DIRT {
            return false;
        }
        let j = self.distance(world, &aint, &aint1);
        if j == -1 {
            true
        } else if j < 6 {
            false
        } else {
            self.height = j;
            true
        }
    }

    pub fn generate(
        &mut self,
        world: &mut dyn GenWorld,
        random: &mut JavaRandom,
        i: i32,
        j: i32,
        k: i32,
    ) -> bool {
        let l = random.next_i64();
        self.rng.set_seed(l);
        self.base = [i, j, k];
        if self.height == 0 {
            self.height = 5 + self.rng.next_i32_bound(self.height_limit);
        }

        if !self.check_height(world) {
            return false;
        }
        self.init_branches(world);
        self.draw_branches(world);
        self.draw_trunk(world);
        self.draw_branch_bases(world);
        true
    }
}

fn line(world: &mut dyn GenWorld, aint: [i32; 3], aint1: [i32; 3], block: u8) {
    let mut aint2 = [0i32; 3];
    let mut b1 = 0usize;
    for b0 in 0..3usize {
        aint2[b0] = aint1[b0] - aint[b0];
        if aint2[b0].abs() > aint2[b1].abs() {
            b1 = b0;
        }
    }

    if aint2[b1] == 0 {
        return;
    }
    let b2 = BIG_TREE_A[b1] as usize;
    let b3 = BIG_TREE_A[b1 + 3] as usize;
    let b4 = if aint2[b1] > 0 { 1 } else { -1 };
    let d0 = aint2[b2] as f64 / aint2[b1] as f64;
    let d1 = aint2[b3] as f64 / aint2[b1] as f64;
    let mut aint3 = [0i32; 3];
    let mut j = 0;
    let k = aint2[b1] + b4;
    while j != k {
        aint3[b1] = math::floor_double(aint[b1] as f64 + j as f64 + 0.5);
        aint3[b2] = math::floor_double(aint[b2] as f64 + j as f64 * d0 + 0.5);
        aint3[b3] = math::floor_double(aint[b3] as f64 + j as f64 * d1 + 0.5);
        world.set_block(aint3[0], aint3[1], aint3[2], block);
        j += b4;
    }
}

pub fn generate_tree(
    world: &mut dyn GenWorld,
    random: &mut JavaRandom,
    kind: TreeKind,
    x: i32,
    y: i32,
    z: i32,
) {
    match kind {
        TreeKind::Trees => {
            generate_trees(world, random, x, y, z);
        }
        TreeKind::Forest => {
            generate_forest(world, random, x, y, z);
        }
        TreeKind::Taiga1 => {
            generate_taiga1(world, random, x, y, z);
        }
        TreeKind::Taiga2 => {
            generate_taiga2(world, random, x, y, z);
        }
        TreeKind::BigTree => {
            let mut tree = BigTree::new();
            tree.configure(1.0, 1.0, 1.0);
            tree.generate(world, random, x, y, z);
        }
    }
}
