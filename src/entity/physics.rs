//! The collision model of the reference implementation (Project-Poseidon,
//! Beta 1.7.3): the axis aligned bounding box (`AxisAlignedBB`), the movement
//! of an entity (`Entity.move`) and the collision boxes of the blocks
//! (`Block.e`).
//!
//! The mobs of the reference are moved by this model, the flintcore port has
//! to reproduce it tick by tick (see `tests/golden/poseidon_mob_traces.txt`).

use crate::gameplay::blocks;
use crate::world::world::World;

/// The axis aligned bounding box of an entity (`AxisAlignedBB`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

impl Aabb {
    pub fn new(min_x: f64, min_y: f64, min_z: f64, max_x: f64, max_y: f64, max_z: f64) -> Self {
        Self {
            min_x,
            min_y,
            min_z,
            max_x,
            max_y,
            max_z,
        }
    }

    /// The aabb of an entity (`Entity.setPosition`).
    ///
    /// `length` is the horizontal size, `width` the vertical one and
    /// `height` the offset of the eyes (`Entity.height`).
    pub fn for_entity(
        x: f64,
        y: f64,
        z: f64,
        length: f32,
        width: f32,
        height: f32,
        step_offset: f32,
    ) -> Self {
        let half = (length / 2.0) as f64;
        let min_y = y - height as f64 + step_offset as f64;
        Self::new(
            x - half,
            min_y,
            z - half,
            x + half,
            min_y + width as f64,
            z + half,
        )
    }

    /// Java `AxisAlignedBB.a(dx, dy, dz)`: the expanded aabb.
    pub fn expanded(&self, dx: f64, dy: f64, dz: f64) -> Aabb {
        let (mut min_x, mut min_y, mut min_z) = (self.min_x, self.min_y, self.min_z);
        let (mut max_x, mut max_y, mut max_z) = (self.max_x, self.max_y, self.max_z);
        if dx < 0.0 {
            min_x += dx;
        }
        if dx > 0.0 {
            max_x += dx;
        }
        if dy < 0.0 {
            min_y += dy;
        }
        if dy > 0.0 {
            max_y += dy;
        }
        if dz < 0.0 {
            min_z += dz;
        }
        if dz > 0.0 {
            max_z += dz;
        }
        Aabb::new(min_x, min_y, min_z, max_x, max_y, max_z)
    }

    /// Java `AxisAlignedBB.b(dx, dy, dz)`: the grown aabb.
    ///
    /// The reference grows the box on both sides, the negative values make
    /// the box smaller.
    pub fn grown(&self, dx: f64, dy: f64, dz: f64) -> Aabb {
        Aabb::new(
            self.min_x - dx,
            self.min_y - dy,
            self.min_z - dz,
            self.max_x + dx,
            self.max_y + dy,
            self.max_z + dz,
        )
    }

    /// Java `AxisAlignedBB.c(dx, dy, dz)`: the moved aabb.
    pub fn moved(&self, dx: f64, dy: f64, dz: f64) -> Aabb {
        Aabb::new(
            self.min_x + dx,
            self.min_y + dy,
            self.min_z + dz,
            self.max_x + dx,
            self.max_y + dy,
            self.max_z + dz,
        )
    }

    /// Java `AxisAlignedBB.d(dx, dy, dz)`: the move in place.
    pub fn offset(&mut self, dx: f64, dy: f64, dz: f64) {
        self.min_x += dx;
        self.min_y += dy;
        self.min_z += dz;
        self.max_x += dx;
        self.max_y += dy;
        self.max_z += dz;
    }

    /// Java `AxisAlignedBB.shrink(dx, dy, dz)`.
    pub fn shrunk(&self, dx: f64, dy: f64, dz: f64) -> Aabb {
        Aabb::new(
            self.min_x + dx,
            self.min_y + dy,
            self.min_z + dz,
            self.max_x - dx,
            self.max_y - dy,
            self.max_z - dz,
        )
    }

    /// Java `AxisAlignedBB.a(other)`: do the two boxes overlap?
    pub fn intersects(&self, other: &Aabb) -> bool {
        other.max_x > self.min_x
            && other.min_x < self.max_x
            && other.max_y > self.min_y
            && other.min_y < self.max_y
            && other.max_z > self.min_z
            && other.min_z < self.max_z
    }

    /// Java `AxisAlignedBB.a(other, d0)`: the clipping on the X axis.
    ///
    /// `self` is the obstacle, `other` the moving box (the arguments of the
    /// reference are the other way round).
    pub fn clip_x(&self, other: &Aabb, d0: f64) -> f64 {
        if other.max_y <= self.min_y || other.min_y >= self.max_y {
            return d0;
        }
        if other.max_z <= self.min_z || other.min_z >= self.max_z {
            return d0;
        }
        let mut d0 = d0;
        if d0 > 0.0 && other.max_x <= self.min_x {
            let d1 = self.min_x - other.max_x;
            if d1 < d0 {
                d0 = d1;
            }
        }
        if d0 < 0.0 && other.min_x >= self.max_x {
            let d1 = self.max_x - other.min_x;
            if d1 > d0 {
                d0 = d1;
            }
        }
        d0
    }

    /// Java `AxisAlignedBB.b(other, d0)`: the clipping on the Y axis.
    pub fn clip_y(&self, other: &Aabb, d0: f64) -> f64 {
        if other.max_x <= self.min_x || other.min_x >= self.max_x {
            return d0;
        }
        if other.max_z <= self.min_z || other.min_z >= self.max_z {
            return d0;
        }
        let mut d0 = d0;
        if d0 > 0.0 && other.max_y <= self.min_y {
            let d1 = self.min_y - other.max_y;
            if d1 < d0 {
                d0 = d1;
            }
        }
        if d0 < 0.0 && other.min_y >= self.max_y {
            let d1 = self.max_y - other.min_y;
            if d1 > d0 {
                d0 = d1;
            }
        }
        d0
    }

    /// Java `AxisAlignedBB.c(other, d0)`: the clipping on the Z axis.
    pub fn clip_z(&self, other: &Aabb, d0: f64) -> f64 {
        if other.max_x <= self.min_x || other.min_x >= self.max_x {
            return d0;
        }
        if other.max_y <= self.min_y || other.min_y >= self.max_y {
            return d0;
        }
        let mut d0 = d0;
        if d0 > 0.0 && other.max_z <= self.min_z {
            let d1 = self.min_z - other.max_z;
            if d1 < d0 {
                d0 = d1;
            }
        }
        if d0 < 0.0 && other.min_z >= self.max_z {
            let d1 = self.max_z - other.min_z;
            if d1 > d0 {
                d0 = d1;
            }
        }
        d0
    }
}

/// The height of a fluid (`BlockFluids.c`).
fn fluid_height(data: u8) -> f64 {
    let level = if data >= 8 { 0 } else { data };
    (level as f64 + 1.0) / 9.0
}

/// Java `World.a(AxisAlignedBB, Material)`: does the box touch the fluid?
/// Java `World.a(AxisAlignedBB, Material)`: is any block of the material in the
/// box? The reference does not look at the height of the fluid here.
fn contains_fluid_block(world: &World, aabb: &Aabb, water: bool) -> bool {
    let min_x = aabb.min_x.floor() as i32;
    let max_x = (aabb.max_x + 1.0).floor() as i32;
    let min_y = aabb.min_y.floor() as i32;
    let max_y = (aabb.max_y + 1.0).floor() as i32;
    let min_z = aabb.min_z.floor() as i32;
    let max_z = (aabb.max_z + 1.0).floor() as i32;
    for x in min_x..max_x {
        for y in min_y..max_y {
            for z in min_z..max_z {
                let id = world.get_block(x, y, z);
                let fluid = if water {
                    blocks::is_water(id)
                } else {
                    blocks::is_lava(id)
                };
                if fluid {
                    return true;
                }
            }
        }
    }
    false
}

/// Java `World.c(AxisAlignedBB)`: is there a liquid in the box?
///
/// The reference checks the water and the lava.
pub fn box_has_fluid(world: &World, aabb: &Aabb) -> bool {
    intersects_fluid(world, aabb, true) || intersects_fluid(world, aabb, false)
}

fn intersects_fluid(world: &World, aabb: &Aabb, water: bool) -> bool {
    let min_x = aabb.min_x.floor() as i32;
    let max_x = (aabb.max_x + 1.0).floor() as i32;
    let min_y = aabb.min_y.floor() as i32;
    let max_y = (aabb.max_y + 1.0).floor() as i32;
    let min_z = aabb.min_z.floor() as i32;
    let max_z = (aabb.max_z + 1.0).floor() as i32;
    for x in min_x..max_x {
        for y in min_y..max_y {
            for z in min_z..max_z {
                let id = world.get_block(x, y, z);
                let fluid = if water {
                    blocks::is_water(id)
                } else {
                    blocks::is_lava(id)
                };
                if !fluid {
                    continue;
                }
                let top = (y + 1) as f64 - fluid_height(world.get_data(x, y, z));
                if (aabb.max_y + 1.0).floor() >= top {
                    return true;
                }
            }
        }
    }
    false
}

/// Java `Entity.f_()`: is the body of the entity in the water?
///
/// The reference expands the box of the entity (`AxisAlignedBB.b`) and
/// shrinks it afterwards, so the box is smaller in the vertical direction.
pub fn in_water(world: &World, aabb: &Aabb) -> bool {
    let shrunk = aabb.grown(0.0, -0.4, 0.0).shrunk(0.001, 0.001, 0.001);
    intersects_fluid(world, &shrunk, true)
}

/// Java `EntitySquid.ad()`: is the squid in the water?
///
/// The squid uses a smaller box than the other entities.
pub fn squid_in_water(world: &World, aabb: &Aabb) -> bool {
    let shrunk = aabb.grown(0.0, -0.6, 0.0);
    intersects_fluid(world, &shrunk, true)
}

/// Java `Entity.ae()`: is the body of the entity in the lava?
///
/// The reference uses the two-argument variant of the check of the world: any
/// block of lava in the box counts, the height of the fluid does not matter
/// and the box is not shrunk.
pub fn in_lava(world: &World, aabb: &Aabb) -> bool {
    let box_ = aabb.grown(-0.1, -0.4, -0.1);
    contains_fluid_block(world, &box_, false)
}

/// The result of a movement (`Entity.move`).
#[derive(Debug, Clone, Copy)]
pub struct MoveResult {
    /// The new position of the entity.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The movement that was actually possible on the three axes.
    pub moved_x: f64,
    pub moved_y: f64,
    pub moved_z: f64,
    /// Java `Entity.positionChanged`.
    pub position_changed: bool,
    /// Java `Entity.onGround`.
    pub on_ground: bool,
    /// Java `Entity.bc`: the entity moved on the Y axis.
    pub moved_vertically: bool,
    /// Java `Entity.br`: the offset between the box and the position.
    pub step_offset: f32,
}

/// The collision box of a block (`Block.e(world, x, y, z)`).
///
/// `None` means the block has no collision box at all (the air, the plants,
/// the torches, the fluids, ...).
pub fn block_collision_box(world: &World, x: i32, y: i32, z: i32) -> Option<Aabb> {
    let id = world.get_block(x, y, z);
    let data = world.get_data(x, y, z);
    let (fx, fy, fz) = (x as f64, y as f64, z as f64);
    match id {
        // The blocks without a collision box.
        blocks::AIR
        | blocks::WATER
        | blocks::STATIONARY_WATER
        | blocks::LAVA
        | blocks::STATIONARY_LAVA
        | blocks::SAPLING
        | blocks::TALL_GRASS
        | blocks::DEAD_BUSH
        | blocks::YELLOW_FLOWER
        | blocks::RED_ROSE
        | blocks::BROWN_MUSHROOM
        | blocks::RED_MUSHROOM
        | blocks::CROPS
        | blocks::TORCH
        | blocks::FIRE
        | blocks::REDSTONE_WIRE => None,
        // `BlockButton`, `BlockLever`, `BlockPressurePlate`, `BlockMinecartTrack`,
        // `BlockWeb`, `BlockFlower`, `BlockFire`, `BlockFluids`, `BlockPortal`
        // and the signs have no aabb at all.
        blocks::LEVER
        | blocks::STONE_PLATE
        | blocks::WOOD_PLATE
        | blocks::REDSTONE_TORCH_OFF
        | blocks::REDSTONE_TORCH_ON
        | blocks::STONE_BUTTON
        | blocks::RAIL
        | blocks::POWERED_RAIL
        | blocks::DETECTOR_RAIL
        | blocks::WEB
        | blocks::REED
        | blocks::PORTAL
        | blocks::SIGN
        | blocks::WALL_SIGN => None,
        // The ladder is a thin plate on the side it is attached to
        // (`BlockLadder.e`).
        blocks::LADDER => {
            let f = 0.125;
            let (min_x, max_x, min_z, max_z) = match data & 3 {
                2 => (0.0, 1.0, 1.0 - f, 1.0),
                3 => (0.0, 1.0, 0.0, f),
                4 => (1.0 - f, 1.0, 0.0, 1.0),
                _ => (0.0, f, 0.0, 1.0),
            };
            Some(Aabb::new(
                fx + min_x,
                fy,
                fz + min_z,
                fx + max_x,
                fy + 1.0,
                fz + max_z,
            ))
        }
        // The cactus is thinner than a block.
        blocks::CACTUS => {
            let f = 0.0625;
            Some(Aabb::new(
                fx + f,
                fy,
                fz + f,
                fx + 1.0 - f,
                fy + 1.0 - f,
                fz + 1.0 - f,
            ))
        }
        // The slow sand is a bit lower than a block.
        blocks::SOUL_SAND => Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 0.875, fz + 1.0)),
        // The snow layer is half a block high (only from the third layer on).
        blocks::SNOW => {
            if data & 7 >= 3 {
                Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 0.5, fz + 1.0))
            } else {
                None
            }
        }
        // The stone slab is half a block high.
        blocks::STEP => Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 0.5, fz + 1.0)),
        // The bed is a bit lower than a block.
        blocks::BED => Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 0.5625, fz + 1.0)),
        // The repeater is flat.
        blocks::REPEATER_OFF | blocks::REPEATER_ON => {
            Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 0.125, fz + 1.0))
        }
        // The cake is a thin block in the middle of the tile.
        blocks::CAKE => {
            let f = 0.0625;
            let f1 = (1 + data * 2) as f32 / 16.0;
            Some(Aabb::new(
                fx + f1 as f64,
                fy,
                fz + f,
                fx + 1.0 - f,
                fy + 0.5 - f,
                fz + 1.0 - f,
            ))
        }
        // The fence is 1.5 blocks high (the modern aabb depends on the
        // neighbours, the legacy one is the default of the configuration).
        blocks::FENCE => Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 1.5, fz + 1.0)),
        // The stairs are a full block with a step, the collision is handled
        // by `stairs_collision_boxes`.
        blocks::COBBLESTONE_STAIRS | blocks::WOOD_STAIRS => {
            Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 1.0, fz + 1.0))
        }
        // The doors are one block wide, they are open or closed.
        blocks::WOODEN_DOOR | blocks::IRON_DOOR => {
            let f = 0.1875;
            let box_min = match data & 3 {
                0 => (0.0, 0.0, 0.0, 1.0, 1.0, f),
                1 => (1.0 - f, 0.0, 0.0, 1.0, 1.0, 1.0),
                2 => (0.0, 0.0, 1.0 - f, 1.0, 1.0, 1.0),
                _ => (0.0, 0.0, 0.0, f, 1.0, 1.0),
            };
            Some(Aabb::new(
                fx + box_min.0,
                fy + box_min.1,
                fz + box_min.2,
                fx + box_min.3,
                fy + box_min.4,
                fz + box_min.5,
            ))
        }
        // The trapdoor is a thin plate in the lower half of the block.
        blocks::TRAPDOOR => {
            let f = 0.1875;
            let (min_y, max_y) = if data & 4 == 0 {
                (0.0, f)
            } else {
                (1.0 - f, 1.0)
            };
            Some(Aabb::new(
                fx,
                fy + min_y,
                fz,
                fx + 1.0,
                fy + max_y,
                fz + 1.0,
            ))
        }
        // The piston head is thinner than a block.
        blocks::PISTON_HEAD => {
            let f = 0.25;
            let (min_x, max_x, min_y, max_y, min_z, max_z) = match data & 7 {
                4 => (0.0, 1.0, 0.0, 1.0, 0.0, f),
                5 => (0.0, 1.0, 0.0, 1.0, 1.0 - f, 1.0),
                6 => (0.0, 1.0, 0.0, f, 1.0, 1.0),
                7 => (1.0 - f, 1.0, 0.0, 1.0, 1.0, 1.0),
                8 => (0.0, f, 0.0, 1.0, 0.0, 1.0),
                9 => (0.0, 1.0, 0.0, 1.0, 0.0, f),
                10 => (0.0, 1.0, 0.0, 1.0, 1.0 - f, 1.0),
                _ => (0.0, 1.0, 0.0, f, 1.0, 1.0),
            };
            Some(Aabb::new(
                fx + min_x,
                fy + min_y,
                fz + min_z,
                fx + max_x,
                fy + max_y,
                fz + max_z,
            ))
        }
        // The moving piston carries the box of the block it pushes.
        blocks::PISTON_MOVING => {
            let base = world.get_block(x, y, z);
            if base == blocks::AIR {
                None
            } else {
                Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 1.0, fz + 1.0))
            }
        }
        // Every other block is a full cube.
        _ => Some(Aabb::new(fx, fy, fz, fx + 1.0, fy + 1.0, fz + 1.0)),
    }
}

/// The collision boxes of the stairs (`BlockStairs.a`).
///
/// The stairs are two half blocks, the collision depends on the orientation.
pub fn stairs_collision_boxes(world: &World, x: i32, y: i32, z: i32, out: &mut Vec<Aabb>) {
    let data = world.get_data(x, y, z);
    let (fx, fy, fz) = (x as f64, y as f64, z as f64);
    let half = match data & 3 {
        0 => Some((0.0, 0.0, 0.0, 0.5, 0.5, 1.0)),
        1 => Some((0.0, 0.0, 0.0, 0.5, 1.0, 1.0)),
        2 => Some((0.0, 0.0, 0.0, 1.0, 0.5, 0.5)),
        _ => Some((0.0, 0.0, 0.0, 1.0, 1.0, 0.5)),
    };
    if let Some((min_x, min_y, min_z, max_x, max_y, max_z)) = half {
        out.push(Aabb::new(
            fx + min_x,
            fy + min_y,
            fz + min_z,
            fx + max_x,
            fy + max_y,
            fz + max_z,
        ));
    }
    let upper = match data & 3 {
        0 => (0.5, 0.0, 0.0, 1.0, 1.0, 1.0),
        1 => (0.5, 0.0, 0.0, 1.0, 0.5, 1.0),
        2 => (0.0, 0.0, 0.5, 1.0, 1.0, 1.0),
        _ => (0.0, 0.0, 0.5, 1.0, 0.5, 1.0),
    };
    out.push(Aabb::new(
        fx + upper.0,
        fy + upper.1,
        fz + upper.2,
        fx + upper.3,
        fy + upper.4,
        fz + upper.5,
    ));
}

/// The collision boxes of the blocks and the entities in a box
/// (`World.getEntities(entity, aabb)`).
pub fn collect_collision_boxes(
    world: &World,
    aabb: &Aabb,
    entity_boxes: &[Aabb],
    out: &mut Vec<Aabb>,
) {
    out.clear();
    let min_x = aabb.min_x.floor() as i32;
    let max_x = aabb.max_x.floor() as i32;
    let min_y = aabb.min_y.floor() as i32;
    let max_y = aabb.max_y.floor() as i32;
    let min_z = aabb.min_z.floor() as i32;
    let max_z = aabb.max_z.floor() as i32;
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                let id = world.get_block(x, y, z);
                if id == blocks::COBBLESTONE_STAIRS || id == blocks::WOOD_STAIRS {
                    stairs_collision_boxes(world, x, y, z, out);
                    continue;
                }
                if let Some(collision) = block_collision_box(world, x, y, z) {
                    if aabb.intersects(&collision) {
                        out.push(collision);
                    }
                }
            }
        }
    }
    for entity_box in entity_boxes {
        if aabb.intersects(entity_box) {
            out.push(*entity_box);
        }
    }
}

/// Moves an entity the way the reference does (`Entity.move`).
///
/// The `boxes` are the collision boxes of the other entities, `scratch` is a
/// reusable buffer for the collision boxes of the blocks.
#[allow(clippy::too_many_arguments)]
pub fn move_entity(
    world: &World,
    aabb: &mut Aabb,
    dx: f64,
    dy: f64,
    dz: f64,
    step_height: f64,
    step_offset: f32,
    on_ground: bool,
    height: f32,
    entity_boxes: &[Aabb],
    scratch: &mut Vec<Aabb>,
) -> MoveResult {
    // `Entity.br *= 0.4F`
    let mut step_offset = step_offset * 0.4;
    let start_box = *aabb;

    let original_x = dx;
    let original_y = dy;
    let original_z = dz;

    // The Y axis.
    let mut moved_y = dy;
    collect_collision_boxes(world, &aabb.expanded(dx, dy, dz), entity_boxes, scratch);
    for obstacle in scratch.iter() {
        moved_y = obstacle.clip_y(aabb, moved_y);
    }
    aabb.offset(0.0, moved_y, 0.0);

    // The X axis.
    let mut moved_x = dx;
    for obstacle in scratch.iter() {
        moved_x = obstacle.clip_x(aabb, moved_x);
    }
    aabb.offset(moved_x, 0.0, 0.0);

    // The Z axis.
    let mut moved_z = dz;
    for obstacle in scratch.iter() {
        moved_z = obstacle.clip_z(aabb, moved_z);
    }
    aabb.offset(0.0, 0.0, moved_z);

    // The step up (`Entity.bs`).
    //
    // The reference climbs a step only when the offset of the step is
    // nearly zero (`Entity.move()`: `br < 0.05F`), the offset of the step is
    // damped at the beginning of the movement (`br *= 0.4F`). A mob that
    // sneaks climbs a step with any offset, the mobs of the port never
    // sneak.
    let grounded = on_ground || (original_y != moved_y && original_y < 0.0);
    if step_height > 0.0
        && grounded
        && step_offset < 0.05
        && (original_x != moved_x || original_z != moved_z)
    {
        let fallback_x = moved_x;
        let fallback_y = moved_y;
        let fallback_z = moved_z;
        let fallback_box = *aabb;

        moved_x = original_x;
        moved_z = original_z;
        *aabb = start_box;

        collect_collision_boxes(
            world,
            &aabb.expanded(original_x, step_height, original_z),
            entity_boxes,
            scratch,
        );
        let mut step_y = step_height;
        for obstacle in scratch.iter() {
            step_y = obstacle.clip_y(aabb, step_y);
        }
        aabb.offset(0.0, step_y, 0.0);

        let mut step_x = original_x;
        for obstacle in scratch.iter() {
            step_x = obstacle.clip_x(aabb, step_x);
        }
        aabb.offset(step_x, 0.0, 0.0);

        let mut step_z = original_z;
        for obstacle in scratch.iter() {
            step_z = obstacle.clip_z(aabb, step_z);
        }
        aabb.offset(0.0, 0.0, step_z);

        // The entity falls back onto the step.
        let mut down = -step_height;
        for obstacle in scratch.iter() {
            down = obstacle.clip_y(aabb, down);
        }
        aabb.offset(0.0, down, 0.0);
        moved_y = down;

        if fallback_x * fallback_x + fallback_z * fallback_z
            >= moved_x * moved_x + moved_z * moved_z
        {
            // The step was not better: keep the normal movement.
            moved_x = fallback_x;
            moved_y = fallback_y;
            moved_z = fallback_z;
            *aabb = fallback_box;
        } else {
            let rest = aabb.min_y - (aabb.min_y as i32) as f64;
            if rest > 0.0 {
                step_offset += (rest + 0.01) as f32;
            }
        }
    }

    let x = (aabb.min_x + aabb.max_x) / 2.0;
    let y = aabb.min_y + height as f64 - step_offset as f64;
    let z = (aabb.min_z + aabb.max_z) / 2.0;
    let position_changed = original_x != moved_x || original_z != moved_z;
    let moved_vertically = original_y != moved_y;
    let ground = moved_vertically && original_y < 0.0;

    MoveResult {
        x,
        y,
        z,
        moved_x,
        moved_y,
        moved_z,
        position_changed,
        on_ground: ground,
        moved_vertically,
        step_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;
    use crate::world::chunk::ChunkPos;

    /// A world with a stone floor at `y = 63`.
    fn test_world() -> World {
        let mut world = World::new(WorldConfig {
            seed: 1,
            save_enabled: false,
            ..Default::default()
        });
        world.ensure_chunk_base(ChunkPos::new(0, 0));
        for x in 0..8 {
            for z in 0..8 {
                world.set_block(x, 63, z, blocks::STONE);
            }
        }
        world
    }

    #[test]
    fn a_mob_does_not_climb_a_block_without_a_jump() {
        let mut world = test_world();
        // A step of one block.
        world.set_block(2, 64, 0, blocks::STONE);

        // The mob (0.6 wide, 1.8 high) walks into the step. The step height
        // of the reference is 0.5, a full block can only be jumped over.
        let mut aabb = Aabb::for_entity(1.5, 64.0, 0.5, 0.6, 1.8, 0.0, 0.0);
        let mut scratch = Vec::new();
        for _ in 0..20 {
            let result = move_entity(
                &world,
                &mut aabb,
                0.2,
                -0.08,
                0.0,
                0.5,
                0.0,
                true,
                0.0,
                &[],
                &mut scratch,
            );
            assert_eq!(result.y, 64.0, "the mob must not climb the block");
            assert!(result.on_ground);
        }
    }

    /// A mob does not climb a step with a big offset.
    ///
    /// Java `Entity.move()`: the mob climbs a step only when the offset of
    /// the step is nearly zero (`br < 0.05F`).
    #[test]
    fn a_mob_does_not_climb_a_step_with_a_big_offset() {
        let mut world = test_world();
        // A slab (0.5 high) in front of the mob.
        world.set_block(2, 63, 0, blocks::STEP);

        // The offset of the step of the mob is 0.5, the reference does not
        // climb the slab with such an offset.
        let mut aabb = Aabb::for_entity(1.9, 64.0, 0.5, 0.6, 1.8, 0.0, 0.0);
        let mut scratch = Vec::new();
        move_entity(
            &world,
            &mut aabb,
            0.2,
            -0.08,
            0.0,
            0.5,
            0.5,
            true,
            0.0,
            &[],
            &mut scratch,
        );
        // The bottom of the box of the mob is still the floor.
        assert_eq!(aabb.min_y, 64.0, "the mob must not climb the slab");
    }

    #[test]
    fn a_mob_lands_on_a_slab() {
        let mut world = test_world();
        // A slab (0.5 high) under the mob.
        world.set_block(2, 63, 0, blocks::STEP);

        let mut aabb = Aabb::for_entity(2.5, 64.0, 0.5, 0.6, 1.8, 0.0, 0.0);
        let mut scratch = Vec::new();
        let mut result = move_entity(
            &world,
            &mut aabb,
            0.0,
            -0.08,
            0.0,
            0.5,
            0.0,
            true,
            0.0,
            &[],
            &mut scratch,
        );
        for _ in 0..20 {
            result = move_entity(
                &world,
                &mut aabb,
                0.0,
                -0.08,
                0.0,
                0.5,
                0.0,
                true,
                0.0,
                &[],
                &mut scratch,
            );
        }
        // The mob rests on the slab.
        assert!(
            (result.y - 63.5).abs() < 0.01,
            "the mob is at {} instead of 63.5",
            result.y
        );
        assert!(result.on_ground);
    }

    #[test]
    fn a_mob_in_the_water_is_in_the_water() {
        let mut world = test_world();
        // A pool of water.
        for x in 0..4 {
            for z in 0..4 {
                world.set_block(x, 63, z, blocks::STATIONARY_WATER);
                world.set_block(x, 64, z, blocks::STATIONARY_WATER);
            }
        }

        // The mob floats in the water.
        let aabb = Aabb::for_entity(0.5, 64.0, 0.5, 0.6, 1.8, 0.0, 0.0);
        assert!(in_water(&world, &aabb));
        assert!(!in_lava(&world, &aabb));

        // A mob on the dry floor is not in the water.
        let aabb = Aabb::for_entity(6.5, 64.0, 6.5, 0.6, 1.8, 0.0, 0.0);
        assert!(!in_water(&world, &aabb));
    }

    #[test]
    fn the_collision_box_of_the_blocks_follows_the_reference() {
        let mut world = test_world();
        // The full cube.
        assert_eq!(
            block_collision_box(&world, 0, 63, 0),
            Some(Aabb::new(0.0, 63.0, 0.0, 1.0, 64.0, 1.0))
        );
        // The air and the plants have no box.
        assert_eq!(block_collision_box(&world, 0, 64, 0), None);
        world.set_block(0, 64, 0, blocks::TALL_GRASS);
        assert_eq!(block_collision_box(&world, 0, 64, 0), None);
        // The slab is half a block high.
        world.set_block(1, 63, 0, blocks::STEP);
        assert_eq!(
            block_collision_box(&world, 1, 63, 0),
            Some(Aabb::new(1.0, 63.0, 0.0, 2.0, 63.5, 1.0))
        );
        // The fence is 1.5 blocks high.
        world.set_block(2, 63, 0, blocks::FENCE);
        assert_eq!(
            block_collision_box(&world, 2, 63, 0),
            Some(Aabb::new(2.0, 63.0, 0.0, 3.0, 64.5, 1.0))
        );
    }
}
