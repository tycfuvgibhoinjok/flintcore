use crate::gameplay::items::{self, ItemStack};
use crate::util::rng::JavaRandom;

pub const AIR: u8 = 0;
pub const STONE: u8 = 1;
pub const GRASS: u8 = 2;
pub const DIRT: u8 = 3;
pub const COBBLESTONE: u8 = 4;
pub const WOOD_PLANKS: u8 = 5;
pub const SAPLING: u8 = 6;
pub const BEDROCK: u8 = 7;
pub const WATER: u8 = 8;
pub const STATIONARY_WATER: u8 = 9;
pub const LAVA: u8 = 10;
pub const STATIONARY_LAVA: u8 = 11;
pub const SAND: u8 = 12;
pub const GRAVEL: u8 = 13;
pub const GOLD_ORE: u8 = 14;
pub const IRON_ORE: u8 = 15;
pub const COAL_ORE: u8 = 16;
pub const LOG: u8 = 17;
pub const LEAVES: u8 = 18;
pub const SPONGE: u8 = 19;
pub const GLASS: u8 = 20;
pub const LAPIS_ORE: u8 = 21;
pub const LAPIS_BLOCK: u8 = 22;
pub const DISPENSER: u8 = 23;
pub const SANDSTONE: u8 = 24;
pub const NOTE_BLOCK: u8 = 25;
pub const BED: u8 = 26;
pub const POWERED_RAIL: u8 = 27;
pub const DETECTOR_RAIL: u8 = 28;
pub const STICKY_PISTON: u8 = 29;
pub const WEB: u8 = 30;
pub const TALL_GRASS: u8 = 31;
pub const DEAD_BUSH: u8 = 32;
pub const PISTON: u8 = 33;
pub const PISTON_HEAD: u8 = 34;
pub const WOOL: u8 = 35;
pub const PISTON_MOVING: u8 = 36;
pub const YELLOW_FLOWER: u8 = 37;
pub const RED_ROSE: u8 = 38;
pub const BROWN_MUSHROOM: u8 = 39;
pub const RED_MUSHROOM: u8 = 40;
pub const GOLD_BLOCK: u8 = 41;
pub const IRON_BLOCK: u8 = 42;
pub const DOUBLE_STEP: u8 = 43;
pub const STEP: u8 = 44;
pub const BRICK: u8 = 45;
pub const TNT: u8 = 46;
pub const BOOKSHELF: u8 = 47;
pub const MOSSY_COBBLESTONE: u8 = 48;
pub const OBSIDIAN: u8 = 49;
pub const TORCH: u8 = 50;
pub const FIRE: u8 = 51;
pub const MOB_SPAWNER: u8 = 52;
pub const WOOD_STAIRS: u8 = 53;
pub const CHEST: u8 = 54;
pub const REDSTONE_WIRE: u8 = 55;
pub const DIAMOND_ORE: u8 = 56;
pub const DIAMOND_BLOCK: u8 = 57;
pub const WORKBENCH: u8 = 58;
pub const CROPS: u8 = 59;
pub const FARMLAND: u8 = 60;
pub const FURNACE: u8 = 61;
pub const LIT_FURNACE: u8 = 62;
pub const SIGN: u8 = 63;
pub const WOODEN_DOOR: u8 = 64;
pub const LADDER: u8 = 65;
pub const RAIL: u8 = 66;
pub const COBBLESTONE_STAIRS: u8 = 67;
pub const WALL_SIGN: u8 = 68;
pub const LEVER: u8 = 69;
pub const STONE_PLATE: u8 = 70;
pub const IRON_DOOR: u8 = 71;
pub const WOOD_PLATE: u8 = 72;
pub const REDSTONE_ORE: u8 = 73;
pub const LIT_REDSTONE_ORE: u8 = 74;
pub const REDSTONE_TORCH_OFF: u8 = 75;
pub const REDSTONE_TORCH_ON: u8 = 76;
pub const STONE_BUTTON: u8 = 77;
pub const SNOW: u8 = 78;
pub const ICE: u8 = 79;
pub const SNOW_BLOCK: u8 = 80;
pub const CACTUS: u8 = 81;
pub const CLAY: u8 = 82;
pub const REED: u8 = 83;
pub const JUKEBOX: u8 = 84;
pub const FENCE: u8 = 85;
pub const PUMPKIN: u8 = 86;
pub const NETHERRACK: u8 = 87;
pub const SOUL_SAND: u8 = 88;
pub const GLOWSTONE: u8 = 89;
pub const PORTAL: u8 = 90;
pub const JACK_O_LANTERN: u8 = 91;
pub const CAKE: u8 = 92;
pub const REPEATER_OFF: u8 = 93;
pub const REPEATER_ON: u8 = 94;
pub const LOCKED_CHEST: u8 = 95;
pub const TRAPDOOR: u8 = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    Air,
    Grass,
    Earth,
    Wood,
    Stone,
    Ore,
    Water,
    Lava,
    Leaves,
    Plant,
    Sponge,
    Cloth,
    Fire,
    Sand,
    Orientable,
    Shatterable,
    Tnt,
    Coral,
    Ice,
    SnowLayer,
    SnowBlock,
    Cactus,
    Clay,
    Pumpkin,
    Portal,
    Cake,
    Web,
    Piston,
}

impl Material {
    pub fn is_liquid(self) -> bool {
        matches!(self, Self::Water | Self::Lava)
    }

    pub fn is_solid(self) -> bool {
        !matches!(
            self,
            Self::Air
                | Self::Water
                | Self::Lava
                | Self::Plant
                | Self::Fire
                | Self::Orientable
                | Self::SnowLayer
                | Self::Portal
        )
    }

    /// Java `Material.h()`: the material is solid and it is not the one of
    /// the leaves, of the ice, of the cactus, of the layer of the snow, of
    /// the TNT and of the shatterable blocks (the flag `F` of the
    /// reference).
    pub fn is_solid_for_collision(self) -> bool {
        self.is_solid()
            && !matches!(
                self,
                Self::Leaves | Self::Ice | Self::Cactus | Self::SnowLayer | Self::Tnt
            )
    }

    pub fn is_buildable(self) -> bool {
        self.is_solid()
    }

    pub fn blocks_light(self) -> bool {
        !matches!(
            self,
            Self::Air
                | Self::Plant
                | Self::Fire
                | Self::Orientable
                | Self::SnowLayer
                | Self::Portal
        )
    }

    pub fn is_replaceable(self) -> bool {
        matches!(
            self,
            Self::Air | Self::Water | Self::Lava | Self::Fire | Self::SnowLayer
        )
    }

    pub fn is_burnable(self) -> bool {
        matches!(self, Self::Wood | Self::Leaves | Self::Cloth | Self::Tnt)
    }

    /// Java `Material.i()`: true when the material does not require a specific tool to harvest.
    pub fn requires_no_tool(self) -> bool {
        !matches!(
            self,
            Self::Stone | Self::Ore | Self::SnowLayer | Self::SnowBlock | Self::Web
        )
    }

    pub fn blocks_movement(self) -> bool {
        self.is_solid() && !matches!(self, Self::Web)
    }

    /// Java `Material.C.q` (`MaterialMapColor` index `0..=13`).
    pub fn map_color_index(self) -> u8 {
        match self {
            Self::Air
            | Self::Orientable
            | Self::Shatterable
            | Self::Portal
            | Self::Piston
            | Self::Cake => 0,
            Self::Grass => 1,
            Self::Sand => 2,
            Self::Cloth | Self::Sponge | Self::Web => 3,
            Self::Lava | Self::Tnt | Self::Fire => 4,
            Self::Ice => 5,
            Self::Ore => 6,
            Self::Plant | Self::Leaves | Self::Cactus | Self::Pumpkin | Self::Coral => 7,
            Self::SnowLayer | Self::SnowBlock => 8,
            Self::Clay => 9,
            Self::Earth => 10,
            Self::Stone => 11,
            Self::Water => 12,
            Self::Wood => 13,
        }
    }
}

pub fn material(id: u8) -> Material {
    match id {
        AIR => Material::Air,
        STONE | COBBLESTONE | GOLD_ORE | IRON_ORE | COAL_ORE | LAPIS_ORE | LAPIS_BLOCK
        | DISPENSER | SANDSTONE | DOUBLE_STEP | STEP | BRICK | MOSSY_COBBLESTONE | OBSIDIAN
        | MOB_SPAWNER | DIAMOND_ORE | FURNACE | LIT_FURNACE | COBBLESTONE_STAIRS | STONE_PLATE
        | REDSTONE_ORE | LIT_REDSTONE_ORE | NETHERRACK | GLOWSTONE => Material::Stone,
        GRASS => Material::Grass,
        DIRT | FARMLAND => Material::Earth,
        WOOD_PLANKS | LOG | NOTE_BLOCK | BOOKSHELF | WOOD_STAIRS | CHEST | WORKBENCH | SIGN
        | WOODEN_DOOR | WALL_SIGN | WOOD_PLATE | JUKEBOX | FENCE | LOCKED_CHEST | TRAPDOOR => {
            Material::Wood
        }
        SAPLING | TALL_GRASS | DEAD_BUSH | YELLOW_FLOWER | RED_ROSE | BROWN_MUSHROOM
        | RED_MUSHROOM | CROPS | REED => Material::Plant,
        BEDROCK => Material::Stone,
        WATER | STATIONARY_WATER => Material::Water,
        LAVA | STATIONARY_LAVA => Material::Lava,
        SAND | GRAVEL | SOUL_SAND => Material::Sand,
        LEAVES => Material::Leaves,
        SPONGE => Material::Sponge,
        GLASS => Material::Shatterable,
        BED | WOOL => Material::Cloth,
        POWERED_RAIL | DETECTOR_RAIL | TORCH | LADDER | RAIL | LEVER | REDSTONE_TORCH_OFF
        | REDSTONE_TORCH_ON | STONE_BUTTON | REPEATER_OFF | REPEATER_ON | REDSTONE_WIRE => {
            Material::Orientable
        }
        STICKY_PISTON | PISTON | PISTON_HEAD | PISTON_MOVING => Material::Piston,
        WEB => Material::Web,
        GOLD_BLOCK | IRON_BLOCK | DIAMOND_BLOCK | IRON_DOOR => Material::Ore,
        TNT => Material::Tnt,
        FIRE => Material::Fire,
        SNOW => Material::SnowLayer,
        ICE => Material::Ice,
        SNOW_BLOCK => Material::SnowBlock,
        CACTUS => Material::Cactus,
        CLAY => Material::Clay,
        PUMPKIN | JACK_O_LANTERN => Material::Pumpkin,
        PORTAL => Material::Portal,
        CAKE => Material::Cake,
        _ => Material::Air,
    }
}

/// Java `Block.strength` (hardness). Negative values mean unbreakable (Bedrock, Portal).
pub fn hardness(id: u8) -> f32 {
    match id {
        BEDROCK | PORTAL | PISTON_MOVING => -1.0,
        SAPLING | TALL_GRASS | DEAD_BUSH | YELLOW_FLOWER | RED_ROSE | BROWN_MUSHROOM
        | RED_MUSHROOM | TNT | TORCH | FIRE | REDSTONE_WIRE | CROPS | REDSTONE_TORCH_OFF
        | REDSTONE_TORCH_ON | REED | REPEATER_OFF | REPEATER_ON | LOCKED_CHEST | LAVA => 0.0,
        SNOW => 0.1,
        LEAVES | BED | SNOW_BLOCK => 0.2,
        GLASS | GLOWSTONE => 0.3,
        LADDER | CACTUS | NETHERRACK => 0.4,
        DIRT | SAND | STICKY_PISTON | PISTON | PISTON_HEAD | LEVER | STONE_PLATE | WOOD_PLATE
        | STONE_BUTTON | ICE | SOUL_SAND | CAKE => 0.5,
        GRASS | GRAVEL | SPONGE | FARMLAND | CLAY => 0.6,
        POWERED_RAIL | DETECTOR_RAIL | RAIL => 0.7,
        SANDSTONE | NOTE_BLOCK | WOOL => 0.8,
        SIGN | WALL_SIGN | PUMPKIN | JACK_O_LANTERN => 1.0,
        STONE | BOOKSHELF => 1.5,
        COBBLESTONE | WOOD_PLANKS | LOG | DOUBLE_STEP | STEP | BRICK | MOSSY_COBBLESTONE
        | WOOD_STAIRS | COBBLESTONE_STAIRS | JUKEBOX | FENCE => 2.0,
        CHEST | WORKBENCH => 2.5,
        GOLD_ORE | IRON_ORE | COAL_ORE | LAPIS_ORE | LAPIS_BLOCK | GOLD_BLOCK | DIAMOND_ORE
        | WOODEN_DOOR | REDSTONE_ORE | LIT_REDSTONE_ORE | TRAPDOOR => 3.0,
        DISPENSER | FURNACE | LIT_FURNACE => 3.5,
        WEB => 4.0,
        IRON_BLOCK | MOB_SPAWNER | DIAMOND_BLOCK | IRON_DOOR => 5.0,
        OBSIDIAN => 10.0,
        WATER | STATIONARY_WATER | STATIONARY_LAVA => 100.0,
        _ => 0.0,
    }
}

pub fn is_tile_entity(id: u8) -> bool {
    matches!(
        id,
        DISPENSER
            | NOTE_BLOCK
            | MOB_SPAWNER
            | CHEST
            | FURNACE
            | LIT_FURNACE
            | SIGN
            | WALL_SIGN
            | JUKEBOX
            | PISTON_MOVING
    )
}

/// Java `Block.n[]`: the blocks that the world ticks randomly.
///
/// The flag is set by `Block.a(boolean)` in the constructor of the block, so
/// the table is the list of the blocks of the reference whose flag is set.
/// The classes of the compiled reference gave the list (the tools of the
/// harness print it).
pub fn ticks_randomly(id: u8) -> bool {
    matches!(
        id,
        GRASS
            | SAPLING
            | WATER
            | LAVA
            | STATIONARY_LAVA
            | LEAVES
            | DETECTOR_RAIL
            | TALL_GRASS
            | DEAD_BUSH
            | YELLOW_FLOWER
            | RED_ROSE
            | BROWN_MUSHROOM
            | RED_MUSHROOM
            | TORCH
            | FIRE
            | CROPS
            | FARMLAND
            | STONE_PLATE
            | WOOD_PLATE
            | LIT_REDSTONE_ORE
            | REDSTONE_TORCH_OFF
            | REDSTONE_TORCH_ON
            | STONE_BUTTON
            | SNOW
            | ICE
            | SNOW_BLOCK
            | CACTUS
            | REED
            | PUMPKIN
            | JACK_O_LANTERN
            | CAKE
            | LOCKED_CHEST
    )
}

pub fn friction_factor(id: u8) -> f32 {
    if id == ICE {
        0.98
    } else {
        0.6
    }
}

pub fn light_opacity(id: u8) -> u8 {
    match id {
        STONE | GRASS | DIRT | COBBLESTONE | WOOD_PLANKS | BEDROCK | LAVA | STATIONARY_LAVA
        | SAND | GRAVEL | GOLD_ORE | IRON_ORE | COAL_ORE | LOG | SPONGE | LAPIS_ORE
        | LAPIS_BLOCK | DISPENSER | SANDSTONE | NOTE_BLOCK | WOOL | GOLD_BLOCK | IRON_BLOCK
        | DOUBLE_STEP | STEP | BRICK | TNT | BOOKSHELF | MOSSY_COBBLESTONE | OBSIDIAN
        | WOOD_STAIRS | CHEST | DIAMOND_ORE | DIAMOND_BLOCK | WORKBENCH | FARMLAND | FURNACE
        | LIT_FURNACE | COBBLESTONE_STAIRS | REDSTONE_ORE | LIT_REDSTONE_ORE | SNOW_BLOCK
        | CLAY | JUKEBOX | PUMPKIN | NETHERRACK | SOUL_SAND | GLOWSTONE | JACK_O_LANTERN
        | LOCKED_CHEST => 255,
        WATER | STATIONARY_WATER | ICE => 3,
        LEAVES | WEB => 1,
        _ => 0,
    }
}
pub fn light_emission(id: u8) -> u8 {
    match id {
        LAVA | STATIONARY_LAVA | FIRE | GLOWSTONE | JACK_O_LANTERN | LOCKED_CHEST => 15,
        TORCH => 14,
        LIT_FURNACE => 13,
        PORTAL => 11,
        LIT_REDSTONE_ORE | REPEATER_ON => 9,
        REDSTONE_TORCH_ON => 7,
        BROWN_MUSHROOM => 1,
        _ => 0,
    }
}

/// Java `Block.o[id]` — `Block.a()` (default `true`, overridden `false` for
/// non-full-cube/technical blocks). Used by `WorldProviderHell.canSpawn`.
/// Java `WorldProviderHell.canSpawn`: the block is an opaque cube
/// (`Block.o[]`).
pub fn is_spawnable(id: u8) -> bool {
    is_opaque_cube(id)
}
pub fn is_opaque(id: u8) -> bool {
    light_opacity(id) > 0
}

/// Java `World.e(int, int, int)`: the block is opaque (`Block.material.h()`)
/// and it is a full cube (`Block.b()`).
pub fn is_opaque_full_cube(id: u8) -> bool {
    is_opaque(id) && is_opaque_cube(id)
}

/// Java `Block.o` (the `opaqueCubeLookup` table) — whitelist dumped from the
/// reference runtime. Note the quirks: LEAVES and CHEST count as opaque,
/// while DOUBLE_STEP/STEP, GLASS, BED, CACTUS, ICE etc. do not.
pub fn is_opaque_cube(id: u8) -> bool {
    matches!(
        id,
        STONE
            | GRASS
            | DIRT
            | COBBLESTONE
            | WOOD_PLANKS
            | BEDROCK
            | SAND
            | GRAVEL
            | GOLD_ORE
            | IRON_ORE
            | COAL_ORE
            | LOG
            | LEAVES
            | SPONGE
            | LAPIS_ORE
            | LAPIS_BLOCK
            | DISPENSER
            | SANDSTONE
            | NOTE_BLOCK
            | WOOL
            | GOLD_BLOCK
            | IRON_BLOCK
            | BRICK
            | TNT
            | BOOKSHELF
            | MOSSY_COBBLESTONE
            | OBSIDIAN
            | CHEST
            | DIAMOND_ORE
            | DIAMOND_BLOCK
            | WORKBENCH
            | FURNACE
            | LIT_FURNACE
            | REDSTONE_ORE
            | LIT_REDSTONE_ORE
            | SNOW_BLOCK
            | CLAY
            | JUKEBOX
            | PUMPKIN
            | NETHERRACK
            | SOUL_SAND
            | GLOWSTONE
            | JACK_O_LANTERN
            | LOCKED_CHEST
    )
}
/// Java `Block.b()` (`renderAsNormalBlock`).
pub fn is_normal_cube(id: u8) -> bool {
    is_opaque_cube(id) && !matches!(id, LEAVES | CHEST)
}

/// Java `Block.a(Entity)`: the resistance of the block to the explosion.
///
/// The resistance is the durability of the block divided by five, the
/// table is the one of the reference (the tool
/// `tools/poseidon_mobs/BlockExplosionResistance.java` prints it).
pub fn explosion_resistance(id: u8) -> f32 {
    match id {
        BEDROCK => 3600000.0,
        OBSIDIAN => 1200.0,
        WATER | STATIONARY_WATER | STATIONARY_LAVA => 100.0,
        STONE | COBBLESTONE | GOLD_BLOCK | IRON_BLOCK | DOUBLE_STEP | STEP | BRICK
        | MOSSY_COBBLESTONE | DIAMOND_BLOCK | COBBLESTONE_STAIRS | JUKEBOX => 6.0,
        MOB_SPAWNER | IRON_DOOR => 5.0,
        WEB => 4.0,
        DISPENSER | FURNACE | LIT_FURNACE => 3.5,
        WOOD_PLANKS | GOLD_ORE | IRON_ORE | COAL_ORE | LAPIS_ORE | LAPIS_BLOCK | WOOD_STAIRS
        | DIAMOND_ORE | WOODEN_DOOR | REDSTONE_ORE | LIT_REDSTONE_ORE | FENCE | TRAPDOOR => 3.0,
        CHEST | WORKBENCH => 2.5,
        LOG => 2.0,
        BOOKSHELF => 1.5,
        SIGN | WALL_SIGN | PUMPKIN | JACK_O_LANTERN => 1.0,
        SANDSTONE | NOTE_BLOCK | WOOL => 0.8,
        POWERED_RAIL | DETECTOR_RAIL | RAIL => 0.7,
        GRASS | GRAVEL | SPONGE | FARMLAND | CLAY => 0.6,
        DIRT | SAND | STICKY_PISTON | PISTON | PISTON_HEAD | LEVER | STONE_PLATE | WOOD_PLATE
        | STONE_BUTTON | ICE | SOUL_SAND | CAKE => 0.5,
        LADDER | CACTUS | NETHERRACK => 0.4,
        GLASS | GLOWSTONE => 0.3,
        LEAVES | BED | SNOW_BLOCK => 0.2,
        SNOW => 0.1,
        SAPLING | LAVA | TALL_GRASS | DEAD_BUSH | PISTON_MOVING | YELLOW_FLOWER | RED_ROSE
        | BROWN_MUSHROOM | RED_MUSHROOM | TNT | TORCH | FIRE | REDSTONE_WIRE | CROPS
        | REDSTONE_TORCH_OFF | REDSTONE_TORCH_ON | REED | PORTAL | REPEATER_OFF | REPEATER_ON
        | LOCKED_CHEST => 0.0,
        _ => 0.0,
    }
}

/// Java `Block.t[]`: the block notifies the clients when its data changes.
///
/// The table is the one of the reference (`tools/poseidon_mobs/BlockTable.java`
/// prints it).
pub fn notifies_on_data_change(id: u8) -> bool {
    matches!(
        id,
        WOOD_PLANKS
            | SAPLING
            | WATER
            | STATIONARY_WATER
            | LAVA
            | STATIONARY_LAVA
            | LOG
            | LEAVES
            | DISPENSER
            | NOTE_BLOCK
            | BED
            | POWERED_RAIL
            | DETECTOR_RAIL
            | STICKY_PISTON
            | PISTON
            | PISTON_HEAD
            | WOOL
            | TORCH
            | FIRE
            | WOOD_STAIRS
            | CHEST
            | REDSTONE_WIRE
            | CROPS
            | FURNACE
            | LIT_FURNACE
            | SIGN
            | WOODEN_DOOR
            | LADDER
            | RAIL
            | COBBLESTONE_STAIRS
            | WALL_SIGN
            | LEVER
            | STONE_PLATE
            | IRON_DOOR
            | WOOD_PLATE
            | REDSTONE_ORE
            | LIT_REDSTONE_ORE
            | REDSTONE_TORCH_OFF
            | REDSTONE_TORCH_ON
            | STONE_BUTTON
            | JUKEBOX
            | FENCE
            | PUMPKIN
            | JACK_O_LANTERN
            | CAKE
            | REPEATER_OFF
            | REPEATER_ON
            | LOCKED_CHEST
            | TRAPDOOR
    )
}

/// Java `World.e(x, y, z)`: the material of the block is solid and the
/// block itself is one (`block.material.h() && block.b()`).
///
/// The table is the one of the reference (`tools/poseidon_mobs/BlockTable.java`
/// prints it).
pub fn is_solid_normal_block(id: u8) -> bool {
    matches!(
        id,
        STONE
            | GRASS
            | DIRT
            | COBBLESTONE
            | WOOD_PLANKS
            | BEDROCK
            | SAND
            | GRAVEL
            | GOLD_ORE
            | IRON_ORE
            | COAL_ORE
            | LOG
            | SPONGE
            | LAPIS_ORE
            | LAPIS_BLOCK
            | DISPENSER
            | SANDSTONE
            | NOTE_BLOCK
            | WOOL
            | GOLD_BLOCK
            | IRON_BLOCK
            | DOUBLE_STEP
            | BRICK
            | BOOKSHELF
            | MOSSY_COBBLESTONE
            | OBSIDIAN
            | MOB_SPAWNER
            | CHEST
            | DIAMOND_ORE
            | DIAMOND_BLOCK
            | WORKBENCH
            | FURNACE
            | LIT_FURNACE
            | REDSTONE_ORE
            | LIT_REDSTONE_ORE
            | SNOW_BLOCK
            | CLAY
            | JUKEBOX
            | PUMPKIN
            | NETHERRACK
            | SOUL_SAND
            | GLOWSTONE
            | JACK_O_LANTERN
            | LOCKED_CHEST
    )
}

pub fn is_liquid(id: u8) -> bool {
    matches!(id, WATER | STATIONARY_WATER | LAVA | STATIONARY_LAVA)
}

/// Java `Material.isSolid()`: the material of the block is solid.
///
/// The table is the one of the reference (the tool
/// `tools/poseidon_mobs/BlockMaterial.java` prints it).
///
/// It is the same table as `is_solid`: the function is kept for the
/// readability of the call sites that speak about the material.
pub fn material_is_solid(id: u8) -> bool {
    is_solid(id)
}

/// Java `Material.isBuildable()`: the material of the block can be built
/// on.
///
/// In this reference the method has the same value as `Material.isSolid()`
/// (both are overridden by `MaterialLogic`, `MaterialLiquid`,
/// `MaterialPortal` and `MaterialTransparent`).
pub fn material_is_buildable(id: u8) -> bool {
    is_solid(id)
}

/// Java `Material.isSolid()`: the material of the block is solid.
///
/// The table is the one of the reference (the tool
/// `tools/poseidon_mobs/BlockMaterial.java` prints it).
pub fn is_solid(id: u8) -> bool {
    matches!(
        id,
        STONE
            | GRASS
            | DIRT
            | COBBLESTONE
            | WOOD_PLANKS
            | BEDROCK
            | SAND
            | GRAVEL
            | GOLD_ORE
            | IRON_ORE
            | COAL_ORE
            | LOG
            | LEAVES
            | SPONGE
            | GLASS
            | LAPIS_ORE
            | LAPIS_BLOCK
            | DISPENSER
            | SANDSTONE
            | NOTE_BLOCK
            | BED
            | STICKY_PISTON
            | WEB
            | PISTON
            | PISTON_HEAD
            | WOOL
            | PISTON_MOVING
            | GOLD_BLOCK
            | IRON_BLOCK
            | DOUBLE_STEP
            | STEP
            | BRICK
            | TNT
            | BOOKSHELF
            | MOSSY_COBBLESTONE
            | OBSIDIAN
            | MOB_SPAWNER
            | WOOD_STAIRS
            | CHEST
            | DIAMOND_ORE
            | DIAMOND_BLOCK
            | WORKBENCH
            | FARMLAND
            | FURNACE
            | LIT_FURNACE
            | SIGN
            | WOODEN_DOOR
            | COBBLESTONE_STAIRS
            | WALL_SIGN
            | STONE_PLATE
            | IRON_DOOR
            | WOOD_PLATE
            | REDSTONE_ORE
            | LIT_REDSTONE_ORE
            | ICE
            | SNOW_BLOCK
            | CACTUS
            | CLAY
            | JUKEBOX
            | FENCE
            | PUMPKIN
            | NETHERRACK
            | SOUL_SAND
            | GLOWSTONE
            | JACK_O_LANTERN
            | CAKE
            | LOCKED_CHEST
            | TRAPDOOR
    )
}
pub fn is_water(id: u8) -> bool {
    matches!(id, WATER | STATIONARY_WATER)
}

pub fn is_lava(id: u8) -> bool {
    matches!(id, LAVA | STATIONARY_LAVA)
}

pub fn is_ice(id: u8) -> bool {
    id == ICE
}

pub fn is_replacable(id: u8) -> bool {
    matches!(id, AIR | FIRE | SNOW)
}

pub fn is_normal_solid(id: u8) -> bool {
    is_solid(id)
        && !matches!(
            id,
            GLASS
                | ICE
                | TNT
                | SNOW
                | CACTUS
                | LEAVES
                | SAPLING
                | TALL_GRASS
                | DEAD_BUSH
                | YELLOW_FLOWER
                | RED_ROSE
                | BROWN_MUSHROOM
                | RED_MUSHROOM
                | CROPS
                | REED
        )
}

/// Java `Block.a_(int metadata)` — dropped item damage/metadata.
pub fn drop_damage(block_id: u8, metadata: u8) -> i16 {
    match block_id {
        LOG | WOOL | STEP | DOUBLE_STEP => metadata as i16,
        SAPLING | LEAVES => (metadata & 3) as i16,
        LAPIS_ORE => 4,
        _ => 0,
    }
}

/// A block that is dropped into the world.
///
/// Java `Block.a(World, int, int, int, ItemStack)`: the item entity of the
/// drop.  The position is a part of the drop — the reference draws it from
/// the random of the world ✓.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDrop {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub item: ItemStack,
}

/// Java `Block.a(World, int, int, int, ItemStack)` — the position of the item
/// entity of a drop.
///
/// The reference draws three numbers of the random of the world:
/// `world.random.nextFloat() * 0.7F + (1.0F - 0.7F) * 0.5F` ✓.  The offset is
/// added to the coordinate in `float` ✓, so the sum is rounded the same way
/// as in the reference ✓.
fn drop_position(rng: &mut JavaRandom, x: i32, y: i32, z: i32) -> (f64, f64, f64) {
    const OFFSET: f32 = 0.7;
    let dx = rng.next_f32() * OFFSET + (1.0f32 - OFFSET) * 0.5f32;
    let dy = rng.next_f32() * OFFSET + (1.0f32 - OFFSET) * 0.5f32;
    let dz = rng.next_f32() * OFFSET + (1.0f32 - OFFSET) * 0.5f32;
    (
        (x as f32 + dx) as f64,
        (y as f32 + dy) as f64,
        (z as f32 + dz) as f64,
    )
}

/// Java `Block.dropNaturally(World, int, int, int, int, float)` — the drops
/// of a block and the positions of the items.
///
/// The reference draws from the random of the world: the number of the items
/// ✓, the chance of every item ✓, the item itself ✓ and the position of the
/// item entity ✓.  The order of the draws decides the whole stream of the
/// world, so it has to be the same as in the reference ✓.
pub fn drop_naturally(
    block_id: u8,
    metadata: u8,
    x: i32,
    y: i32,
    z: i32,
    rng: &mut JavaRandom,
    held_item: Option<ItemStack>,
) -> Vec<BlockDrop> {
    // The chance of the item (`f` of the reference) is the yield of the
    // explosion, which is one ✓ — a plugin can change it, the port has no
    // plugins ✓.
    let chance = 1.0f32;
    let mut drops = Vec::new();

    // The leaves with the shears drop themselves (`BlockLeaves.a(World,
    // EntityHuman, int, int, int, int)`): the block itself, without the
    // draws of the normal path ✓.
    if block_id == LEAVES {
        if let Some(held) = held_item {
            if held.id == items::SHEARS {
                let (px, py, pz) = drop_position(rng, x, y, z);
                drops.push(BlockDrop {
                    x: px,
                    y: py,
                    z: pz,
                    item: ItemStack::new(LEAVES as i16, 1, (metadata & 3) as i16),
                });
                return drops;
            }
        }
    }

    // The snow layer drops a snow ball (`BlockSnow.a(World, EntityHuman, ...)`):
    // the item is created directly, so only the position is drawn ✓.
    if block_id == SNOW {
        let (px, py, pz) = drop_position(rng, x, y, z);
        drops.push(BlockDrop {
            x: px,
            y: py,
            z: pz,
            item: ItemStack::new(items::SNOW_BALL, 1, 0),
        });
        return drops;
    }

    let count = drop_count(block_id, rng);
    for _ in 0..count {
        // The chance of the item (`world.random.nextFloat() < f`) ✓.
        if rng.next_f32() < chance {
            let item_id = drop_item_id(block_id, metadata, rng);
            if item_id > 0 {
                let (px, py, pz) = drop_position(rng, x, y, z);
                drops.push(BlockDrop {
                    x: px,
                    y: py,
                    z: pz,
                    item: ItemStack::new(item_id, 1, drop_damage(block_id, metadata)),
                });
            }
        }
    }

    // The crops drop the seeds (`BlockCrops.dropNaturally`) ✓.
    if block_id == CROPS {
        for _ in 0..3 {
            if rng.next_i32_bound(15) <= metadata as i32 {
                let (px, py, pz) = drop_position(rng, x, y, z);
                drops.push(BlockDrop {
                    x: px,
                    y: py,
                    z: pz,
                    item: ItemStack::new(items::SEEDS, 1, 0),
                });
            }
        }
    }

    drops
}

/// Java `Block.a(Random)` — base drop count before per-item roll.
pub fn drop_count(block_id: u8, rng: &mut JavaRandom) -> usize {
    match block_id {
        AIR | WATER | STATIONARY_WATER | LAVA | STATIONARY_LAVA | GLASS | BOOKSHELF | FIRE
        | MOB_SPAWNER | SNOW | ICE | PORTAL | CAKE | PISTON_HEAD | PISTON_MOVING | DEAD_BUSH
        | TNT => 0,
        LEAVES => {
            if rng.next_i32_bound(20) == 0 {
                1
            } else {
                0
            }
        }
        LAPIS_ORE => (4 + rng.next_i32_bound(5)) as usize,
        DOUBLE_STEP => 2,
        REDSTONE_ORE | LIT_REDSTONE_ORE => (4 + rng.next_i32_bound(2)) as usize,
        SNOW_BLOCK | CLAY => 4,
        GLOWSTONE => (2 + rng.next_i32_bound(3)) as usize,
        _ => 1,
    }
}

/// Java `Block.a(int metadata, Random)` — dropped item ID (`<= 0` means nothing).
pub fn drop_item_id(block_id: u8, metadata: u8, rng: &mut JavaRandom) -> i16 {
    match block_id {
        AIR | WATER | STATIONARY_WATER | LAVA | STATIONARY_LAVA | GLASS | BOOKSHELF | FIRE
        | MOB_SPAWNER | ICE | PORTAL | CAKE | PISTON_HEAD | PISTON_MOVING | DEAD_BUSH | TNT => -1,
        // The bedrock and the locked chest drop themselves
        // (`Block.a(int, Random)`).
        BEDROCK => BEDROCK as i16,
        LOCKED_CHEST => LOCKED_CHEST as i16,
        // The stairs drop the block they are made of (`BlockStairs`).
        WOOD_STAIRS => WOOD_PLANKS as i16,
        COBBLESTONE_STAIRS => COBBLESTONE as i16,
        STONE => COBBLESTONE as i16,
        GRASS | FARMLAND => DIRT as i16,
        GRAVEL => {
            if rng.next_i32_bound(10) == 0 {
                items::FLINT
            } else {
                GRAVEL as i16
            }
        }
        COAL_ORE => items::COAL,
        DIAMOND_ORE => items::DIAMOND,
        LAPIS_ORE => items::INK_SACK,
        LEAVES => SAPLING as i16,
        BED => {
            if (metadata & 8) != 0 {
                -1
            } else {
                items::BED
            }
        }
        WEB => items::STRING,
        TALL_GRASS => {
            if rng.next_i32_bound(8) == 0 {
                items::SEEDS
            } else {
                -1
            }
        }
        DOUBLE_STEP | STEP => STEP as i16,
        REDSTONE_WIRE | REDSTONE_ORE | LIT_REDSTONE_ORE => items::REDSTONE,
        CROPS => {
            if metadata == 7 {
                items::WHEAT
            } else {
                -1
            }
        }
        LIT_FURNACE => FURNACE as i16,
        SIGN | WALL_SIGN => items::SIGN,
        WOODEN_DOOR => {
            if (metadata & 8) != 0 {
                -1
            } else {
                items::WOOD_DOOR
            }
        }
        IRON_DOOR => {
            if (metadata & 8) != 0 {
                -1
            } else {
                items::IRON_DOOR
            }
        }
        REDSTONE_TORCH_OFF | REDSTONE_TORCH_ON => REDSTONE_TORCH_ON as i16,
        SNOW | SNOW_BLOCK => items::SNOW_BALL,
        CLAY => items::CLAY_BALL,
        REED => items::SUGAR_CANE,
        GLOWSTONE => items::GLOWSTONE_DUST,
        REPEATER_OFF | REPEATER_ON => items::DIODE,
        other => other as i16,
    }
}

/// Complete reproduction of `Block.a(World, EntityHuman, x, y, z, data)` / `Block.dropNaturally`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepSound {
    pub name: &'static str,
    pub volume1: f32,
    pub volume2: f32,
}

impl StepSound {
    pub fn new(name: &'static str, volume1: f32, volume2: f32) -> Self {
        Self {
            name,
            volume1,
            volume2,
        }
    }
}

/// Java `Block.e()`: the mobility of the block for the pistons.
///
/// `0` is a normal block, `1` is a block that breaks and drops an item and
/// `2` is a block that cannot be moved.  The table is the one of the
/// reference (`tools/poseidon_mobs/BlockTable.java` prints it).
pub fn piston_mobility(id: u8) -> u8 {
    // The table of the reference (`Block.e()`), the tool
    // `tools/poseidon_mobs/BlockTable.java` prints it.
    match id {
        STONE | GRASS | DIRT | COBBLESTONE | WOOD_PLANKS | BEDROCK | SAND | GRAVEL | GOLD_ORE
        | IRON_ORE | COAL_ORE | LOG | SPONGE | GLASS | LAPIS_ORE | LAPIS_BLOCK | DISPENSER
        | SANDSTONE | NOTE_BLOCK | POWERED_RAIL | DETECTOR_RAIL | WOOL | GOLD_BLOCK
        | IRON_BLOCK | DOUBLE_STEP | STEP | BRICK | TNT | BOOKSHELF | MOSSY_COBBLESTONE
        | OBSIDIAN | MOB_SPAWNER | WOOD_STAIRS | CHEST | DIAMOND_ORE | DIAMOND_BLOCK
        | WORKBENCH | FARMLAND | FURNACE | LIT_FURNACE | SIGN | RAIL | COBBLESTONE_STAIRS
        | WALL_SIGN | REDSTONE_ORE | LIT_REDSTONE_ORE | ICE | SNOW_BLOCK | CLAY | JUKEBOX
        | FENCE | NETHERRACK | SOUL_SAND | GLOWSTONE | LOCKED_CHEST | TRAPDOOR => 0,
        SAPLING | WATER | STATIONARY_WATER | LAVA | STATIONARY_LAVA | LEAVES | BED | WEB
        | TALL_GRASS | DEAD_BUSH | YELLOW_FLOWER | RED_ROSE | BROWN_MUSHROOM | RED_MUSHROOM
        | TORCH | FIRE | REDSTONE_WIRE | CROPS | WOODEN_DOOR | LADDER | LEVER | STONE_PLATE
        | IRON_DOOR | WOOD_PLATE | REDSTONE_TORCH_OFF | REDSTONE_TORCH_ON | STONE_BUTTON | SNOW
        | CACTUS | REED | PUMPKIN | JACK_O_LANTERN | CAKE | REPEATER_OFF | REPEATER_ON => 1,
        STICKY_PISTON | PISTON | PISTON_HEAD | PISTON_MOVING | PORTAL => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drops of a block draw from the random of the world.
    ///
    /// Java `Block.dropNaturally`: the number of the items ✓, the chance of
    /// every item ✓, the item itself ✓ and the position of the item entity
    /// ✓.  The draws decide the whole stream of the world, so their number
    /// and their order have to be the same as in the reference ✓.
    #[test]
    fn the_drops_of_a_block_draw_from_the_random_of_the_world() {
        // The stone drops a cobblestone: the number of the items (no draw),
        // the chance of the item (one draw), the item (no draw for the
        // stone) and the position of the item entity (three draws).
        let mut rng = JavaRandom::new(12345);
        let drops = drop_naturally(STONE, 0, 10, 64, 20, &mut rng, None);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].item.id, COBBLESTONE as i16);
        assert_eq!(rng.draws, 4, "the chance and the position of the item");
        // The position of the item is inside the block (`0.15` to `0.85`).
        assert!(drops[0].x >= 10.0 && drops[0].x < 11.0, "{}", drops[0].x);
        assert!(drops[0].y >= 64.0 && drops[0].y < 65.0, "{}", drops[0].y);
        assert!(drops[0].z >= 20.0 && drops[0].z < 21.0, "{}", drops[0].z);

        // The gravel draws the item itself (`Block.a(int, Random)`): the
        // flint is one item out of ten.
        let mut rng = JavaRandom::new(12345);
        let drops = drop_naturally(GRAVEL, 0, 0, 64, 0, &mut rng, None);
        assert_eq!(drops.len(), 1);
        assert_eq!(rng.draws, 5, "the chance, the item and the position");
    }

    /// The leaves with the shears drop themselves.
    ///
    /// Java `BlockLeaves.a(World, EntityHuman, int, int, int, int)`: the
    /// block itself, without the draws of the normal path ✓.
    #[test]
    fn the_leaves_with_the_shears_drop_themselves() {
        let mut rng = JavaRandom::new(12345);
        let drops = drop_naturally(
            LEAVES,
            0,
            0,
            64,
            0,
            &mut rng,
            Some(ItemStack::new(items::SHEARS, 1, 0)),
        );
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].item.id, LEAVES as i16);
        assert_eq!(rng.draws, 3, "only the position of the item is drawn");
    }

    /// The tables of the blocks are the tables of the reference.
    ///
    /// The tool `tools/poseidon_mobs/BlockTable.java` prints the tables of
    /// the reference, they are saved in `tests/golden/block_tables.txt`.
    #[test]
    fn the_tables_of_the_blocks_are_the_tables_of_the_reference() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/block_tables.txt");
        let table = std::fs::read_to_string(&path).expect("the table of the reference");
        let mut checked = 0;
        for line in table.lines() {
            let mut parts = line.split_whitespace();
            let Some(id) = parts.next().and_then(|v| v.parse::<u8>().ok()) else {
                continue;
            };
            // The format of the line: the id, the name of the class of the
            // block and the values of the tables.
            let values: Vec<&str> = parts.collect();
            let flag = |name: &str| -> bool {
                values
                    .iter()
                    .find(|v| v.starts_with(name))
                    .and_then(|v| v.split('=').nth(1))
                    .map(|v| v == "true")
                    .unwrap_or(false)
            };
            let number = |name: &str| -> i32 {
                values
                    .iter()
                    .find(|v| v.starts_with(name))
                    .and_then(|v| v.split('=').nth(1))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            };
            assert_eq!(
                is_opaque_cube(id),
                flag("opaque"),
                "the opaque cube of the block {}",
                id
            );
            assert_eq!(
                light_opacity(id),
                u8::try_from(number("light")).unwrap_or(255),
                "the opacity of the light of the block {}",
                id
            );
            assert_eq!(
                is_solid_normal_block(id),
                flag("solid"),
                "the solid block {}",
                id
            );
            assert_eq!(
                piston_mobility(id),
                u8::try_from(number("mobility")).unwrap_or(0),
                "the mobility of the block {}",
                id
            );
            assert_eq!(
                notifies_on_data_change(id),
                flag("notify"),
                "the notification of the data of the block {}",
                id
            );
            checked += 1;
        }
        assert!(checked > 90, "the table of the reference is complete");

        // The table of the solid material of the blocks.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/block_material.txt");
        let table = std::fs::read_to_string(&path).expect("the table of the material");
        let mut checked = 0;
        for line in table.lines() {
            let mut parts = line.split_whitespace();
            let Some(id) = parts.next().and_then(|v| v.parse::<u8>().ok()) else {
                continue;
            };
            let solid = parts.nth(1).map(|v| v == "true").unwrap_or(false);
            assert_eq!(
                is_solid(id),
                solid,
                "the solid material of the block {}",
                id
            );
            checked += 1;
        }
        assert!(checked > 90, "the table of the material is complete");

        // The table of the resistance to the explosion.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/block_explosion_resistance.txt");
        let table = std::fs::read_to_string(&path).expect("the table of the resistance");
        let mut checked = 0;
        for line in table.lines() {
            let mut parts = line.split_whitespace();
            let Some(id) = parts.next().and_then(|v| v.parse::<u8>().ok()) else {
                continue;
            };
            let value: f32 = parts.nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            assert_eq!(
                explosion_resistance(id),
                value,
                "the resistance to the explosion of the block {}",
                id
            );
            checked += 1;
        }
        assert!(checked > 90, "the table of the resistance is complete");
    }

    /// The table of the random ticks is the table of the reference.
    ///
    /// Java `Block.n[]`: the flag is set by `Block.a(boolean)` in the
    /// constructor of the block. The classes of the compiled reference gave
    /// the list (the tools of the harness print it).
    #[test]
    fn the_table_of_the_random_ticks_is_the_table_of_the_reference() {
        let ticking = [
            GRASS,
            SAPLING,
            WATER,
            LAVA,
            STATIONARY_LAVA,
            LEAVES,
            DETECTOR_RAIL,
            TALL_GRASS,
            DEAD_BUSH,
            YELLOW_FLOWER,
            RED_ROSE,
            BROWN_MUSHROOM,
            RED_MUSHROOM,
            TORCH,
            FIRE,
            CROPS,
            FARMLAND,
            STONE_PLATE,
            WOOD_PLATE,
            LIT_REDSTONE_ORE,
            REDSTONE_TORCH_OFF,
            REDSTONE_TORCH_ON,
            STONE_BUTTON,
            SNOW,
            ICE,
            SNOW_BLOCK,
            CACTUS,
            REED,
            PUMPKIN,
            JACK_O_LANTERN,
            CAKE,
            LOCKED_CHEST,
        ];
        for id in ticking {
            assert!(
                ticks_randomly(id),
                "the block {} ticks randomly in the reference",
                id
            );
        }
        // The blocks that do not tick in the reference: the plain water, the
        // redstone ore, the portal and the chest.
        for id in [STATIONARY_WATER, REDSTONE_ORE, PORTAL, CHEST] {
            assert!(
                !ticks_randomly(id),
                "the block {} does not tick randomly in the reference",
                id
            );
        }
    }
}
