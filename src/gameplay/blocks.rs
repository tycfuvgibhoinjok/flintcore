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

pub fn light_opacity(id: u8) -> u8 {
    match id {
        AIR => 0,
        WATER | STATIONARY_WATER | ICE => 3,
        LEAVES | WEB => 1,
        SAPLING | GLASS | BED | POWERED_RAIL | DETECTOR_RAIL | STICKY_PISTON | TALL_GRASS
        | DEAD_BUSH | PISTON | PISTON_HEAD | YELLOW_FLOWER | RED_ROSE | BROWN_MUSHROOM
        | RED_MUSHROOM | STEP | TORCH | FIRE | MOB_SPAWNER | WOOD_STAIRS | REDSTONE_WIRE
        | CROPS | FARMLAND | SIGN | WOODEN_DOOR | LADDER | COBBLESTONE_STAIRS | WALL_SIGN
        | LEVER | STONE_PLATE | IRON_DOOR | WOOD_PLATE | STONE_BUTTON | SNOW | CACTUS | REED
        | FENCE | PORTAL | CAKE | REPEATER_OFF | REPEATER_ON | TRAPDOOR => 0,
        _ => 255,
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

pub fn is_opaque(id: u8) -> bool {
    light_opacity(id) > 0
}

pub fn is_opaque_cube(id: u8) -> bool {
    !matches!(
        id,
        AIR | SAPLING
            | WATER
            | STATIONARY_WATER
            | LAVA
            | STATIONARY_LAVA
            | LEAVES
            | GLASS
            | BED
            | POWERED_RAIL
            | DETECTOR_RAIL
            | STICKY_PISTON
            | WEB
            | TALL_GRASS
            | DEAD_BUSH
            | PISTON
            | PISTON_HEAD
            | YELLOW_FLOWER
            | RED_ROSE
            | BROWN_MUSHROOM
            | RED_MUSHROOM
            | STEP
            | TORCH
            | FIRE
            | MOB_SPAWNER
            | WOOD_STAIRS
            | REDSTONE_WIRE
            | CROPS
            | FARMLAND
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
            | REDSTONE_TORCH_OFF
            | REDSTONE_TORCH_ON
            | STONE_BUTTON
            | SNOW
            | ICE
            | CACTUS
            | REED
            | FENCE
            | PORTAL
            | CAKE
            | REPEATER_OFF
            | REPEATER_ON
            | TRAPDOOR
    )
}

pub fn is_liquid(id: u8) -> bool {
    matches!(id, WATER | STATIONARY_WATER | LAVA | STATIONARY_LAVA)
}

pub fn is_solid(id: u8) -> bool {
    !matches!(
        id,
        AIR | WATER
            | STATIONARY_WATER
            | LAVA
            | STATIONARY_LAVA
            | SAPLING
            | TALL_GRASS
            | DEAD_BUSH
            | YELLOW_FLOWER
            | RED_ROSE
            | BROWN_MUSHROOM
            | RED_MUSHROOM
            | CROPS
            | SNOW
            | FIRE
            | PORTAL
    )
}

pub fn is_water(id: u8) -> bool {
    matches!(id, WATER | STATIONARY_WATER)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_matches_reference() {
        assert_eq!(light_opacity(AIR), 0);
        assert_eq!(light_opacity(STONE), 255);
        assert_eq!(light_opacity(WATER), 3);
        assert_eq!(light_opacity(ICE), 3);
        assert_eq!(light_opacity(LEAVES), 1);
        assert_eq!(light_opacity(WEB), 1);
        assert_eq!(light_opacity(LAVA), 255);
        assert_eq!(light_opacity(GLASS), 0);
        assert_eq!(light_opacity(FARMLAND), 0);
        assert_eq!(light_opacity(STEP), 0);
        assert_eq!(light_opacity(DOUBLE_STEP), 255);
    }

    #[test]
    fn emission_matches_reference() {
        assert_eq!(light_emission(TORCH), 14);
        assert_eq!(light_emission(LAVA), 15);
        assert_eq!(light_emission(LIT_FURNACE), 13);
        assert_eq!(light_emission(GLOWSTONE), 15);
        assert_eq!(light_emission(BROWN_MUSHROOM), 1);
        assert_eq!(light_emission(STONE), 0);
    }
}
