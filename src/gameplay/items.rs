use crate::gameplay::blocks::{self, Material};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMaterial {
    Wood,
    Stone,
    Iron,
    Diamond,
    Gold,
}

impl ToolMaterial {
    /// Java `EnumToolMaterial.d()` — harvest level (0..=3).
    pub fn harvest_level(self) -> i32 {
        match self {
            Self::Wood | Self::Gold => 0,
            Self::Stone => 1,
            Self::Iron => 2,
            Self::Diamond => 3,
        }
    }

    /// Java `EnumToolMaterial.a()` — max durability.
    pub fn max_uses(self) -> i16 {
        match self {
            Self::Wood => 59,
            Self::Stone => 131,
            Self::Iron => 250,
            Self::Diamond => 1561,
            Self::Gold => 32,
        }
    }

    /// Java `EnumToolMaterial.b()` — mining efficiency multiplier.
    pub fn efficiency(self) -> f32 {
        match self {
            Self::Wood => 2.0,
            Self::Stone => 4.0,
            Self::Iron => 6.0,
            Self::Diamond => 8.0,
            Self::Gold => 12.0,
        }
    }

    /// Java `EnumToolMaterial.c()` — damage vs entity bonus.
    pub fn damage_bonus(self) -> i16 {
        match self {
            Self::Wood | Self::Gold => 0,
            Self::Stone => 1,
            Self::Iron => 2,
            Self::Diamond => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Pickaxe(ToolMaterial),
    Spade(ToolMaterial),
    Axe(ToolMaterial),
    Sword(ToolMaterial),
    Hoe(ToolMaterial),
    Shears,
    FlintAndSteel,
    Bow,
    FishingRod,
}

pub fn tool_kind(id: i16) -> Option<ToolKind> {
    match id {
        WOOD_PICKAXE => Some(ToolKind::Pickaxe(ToolMaterial::Wood)),
        STONE_PICKAXE => Some(ToolKind::Pickaxe(ToolMaterial::Stone)),
        IRON_PICKAXE => Some(ToolKind::Pickaxe(ToolMaterial::Iron)),
        DIAMOND_PICKAXE => Some(ToolKind::Pickaxe(ToolMaterial::Diamond)),
        GOLD_PICKAXE => Some(ToolKind::Pickaxe(ToolMaterial::Gold)),

        WOOD_SPADE => Some(ToolKind::Spade(ToolMaterial::Wood)),
        STONE_SPADE => Some(ToolKind::Spade(ToolMaterial::Stone)),
        IRON_SPADE => Some(ToolKind::Spade(ToolMaterial::Iron)),
        DIAMOND_SPADE => Some(ToolKind::Spade(ToolMaterial::Diamond)),
        GOLD_SPADE => Some(ToolKind::Spade(ToolMaterial::Gold)),

        WOOD_AXE => Some(ToolKind::Axe(ToolMaterial::Wood)),
        STONE_AXE => Some(ToolKind::Axe(ToolMaterial::Stone)),
        IRON_AXE => Some(ToolKind::Axe(ToolMaterial::Iron)),
        DIAMOND_AXE => Some(ToolKind::Axe(ToolMaterial::Diamond)),
        GOLD_AXE => Some(ToolKind::Axe(ToolMaterial::Gold)),

        WOOD_SWORD => Some(ToolKind::Sword(ToolMaterial::Wood)),
        STONE_SWORD => Some(ToolKind::Sword(ToolMaterial::Stone)),
        IRON_SWORD => Some(ToolKind::Sword(ToolMaterial::Iron)),
        DIAMOND_SWORD => Some(ToolKind::Sword(ToolMaterial::Diamond)),
        GOLD_SWORD => Some(ToolKind::Sword(ToolMaterial::Gold)),

        WOOD_HOE => Some(ToolKind::Hoe(ToolMaterial::Wood)),
        STONE_HOE => Some(ToolKind::Hoe(ToolMaterial::Stone)),
        IRON_HOE => Some(ToolKind::Hoe(ToolMaterial::Iron)),
        DIAMOND_HOE => Some(ToolKind::Hoe(ToolMaterial::Diamond)),
        GOLD_HOE => Some(ToolKind::Hoe(ToolMaterial::Gold)),

        SHEARS => Some(ToolKind::Shears),
        FLINT_AND_STEEL => Some(ToolKind::FlintAndSteel),
        BOW => Some(ToolKind::Bow),
        FISHING_ROD => Some(ToolKind::FishingRod),
        _ => None,
    }
}

/// Java `Item.d()` (`usesData` / hasSubtypes).
pub fn uses_data(id: i16) -> bool {
    matches!(
        id,
        6 | 17 | 18 | 35 | 44 | COAL | INK_SACK // Sapling, Log, Leaves, Wool, Step, Coal, Dye
    )
}

/// Java `ItemBlock.filterData(int damage)` when placing a block from an `ItemStack`.
pub fn filter_block_data(id: i16, damage: i16) -> u8 {
    match id {
        6 | 17 | 35 | 44 => (damage & 15) as u8,
        18 => ((damage as u8) & 3) | 8,
        29 | 33 => 7,
        _ => 0,
    }
}

/// Java `Item.h()` / `Item.i()`: container item left in crafting grid after crafting (e.g. Milk Bucket -> Bucket).
pub fn crafting_container_item(id: i16) -> Option<i16> {
    match id {
        WATER_BUCKET | LAVA_BUCKET | MILK_BUCKET => Some(BUCKET),
        _ => None,
    }
}

/// Java `Item.a(ItemStack, Block)` — tool destroy speed against `block_id`.
pub fn destroy_speed_against(item: Option<ItemStack>, block_id: u8) -> f32 {
    let Some(stack) = item else {
        return 1.0;
    };
    match tool_kind(stack.id) {
        Some(ToolKind::Pickaxe(mat)) => {
            if matches!(
                block_id,
                blocks::COBBLESTONE
                    | blocks::DOUBLE_STEP
                    | blocks::STEP
                    | blocks::STONE
                    | blocks::SANDSTONE
                    | blocks::MOSSY_COBBLESTONE
                    | blocks::IRON_ORE
                    | blocks::IRON_BLOCK
                    | blocks::COAL_ORE
                    | blocks::GOLD_BLOCK
                    | blocks::GOLD_ORE
                    | blocks::DIAMOND_ORE
                    | blocks::DIAMOND_BLOCK
                    | blocks::ICE
                    | blocks::NETHERRACK
                    | blocks::LAPIS_ORE
                    | blocks::LAPIS_BLOCK
            ) {
                mat.efficiency()
            } else {
                1.0
            }
        }
        Some(ToolKind::Spade(mat)) => {
            if matches!(
                block_id,
                blocks::GRASS
                    | blocks::DIRT
                    | blocks::SAND
                    | blocks::GRAVEL
                    | blocks::SNOW
                    | blocks::SNOW_BLOCK
                    | blocks::CLAY
                    | blocks::FARMLAND
            ) {
                mat.efficiency()
            } else {
                1.0
            }
        }
        Some(ToolKind::Axe(mat)) => {
            if matches!(
                block_id,
                blocks::WOOD_PLANKS | blocks::BOOKSHELF | blocks::LOG | blocks::CHEST
            ) {
                mat.efficiency()
            } else {
                1.0
            }
        }
        Some(ToolKind::Sword(_)) => {
            if block_id == blocks::WEB {
                15.0
            } else {
                1.5
            }
        }
        Some(ToolKind::Shears) => {
            if block_id == blocks::WEB || block_id == blocks::LEAVES {
                15.0
            } else if block_id == blocks::WOOL {
                5.0
            } else {
                1.0
            }
        }
        _ => 1.0,
    }
}

/// Java `InventoryPlayer.b(Block)` + `Item.a(Block)` — can the player harvest `block_id` with `item`?
pub fn can_harvest_block(item: Option<ItemStack>, block_id: u8) -> bool {
    let mat = blocks::material(block_id);
    if mat.requires_no_tool() {
        return true;
    }
    let Some(stack) = item else {
        return false;
    };
    match tool_kind(stack.id) {
        Some(ToolKind::Pickaxe(tool_mat)) => {
            let level = tool_mat.harvest_level();
            match block_id {
                blocks::OBSIDIAN => level == 3,
                blocks::DIAMOND_BLOCK
                | blocks::DIAMOND_ORE
                | blocks::GOLD_BLOCK
                | blocks::GOLD_ORE
                | blocks::REDSTONE_ORE
                | blocks::LIT_REDSTONE_ORE => level >= 2,
                blocks::IRON_BLOCK | blocks::IRON_ORE | blocks::LAPIS_BLOCK | blocks::LAPIS_ORE => {
                    level >= 1
                }
                _ => matches!(mat, Material::Stone | Material::Ore),
            }
        }
        Some(ToolKind::Spade(_)) => {
            matches!(block_id, blocks::SNOW | blocks::SNOW_BLOCK)
        }
        Some(ToolKind::Sword(_) | ToolKind::Shears) => block_id == blocks::WEB,
        _ => false,
    }
}

/// Java `Block.getDamage(EntityHuman)` — progress added per tick (1.0 = broken).
pub fn calc_block_damage_per_tick(
    held_item: Option<ItemStack>,
    block_id: u8,
    in_water: bool,
    on_ground: bool,
) -> f32 {
    let strength = blocks::hardness(block_id);
    if strength < 0.0 {
        return 0.0;
    }
    if !can_harvest_block(held_item, block_id) {
        return 1.0 / strength / 100.0;
    }
    if strength == 0.0 {
        return 1.0;
    }
    let mut speed = destroy_speed_against(held_item, block_id);
    if in_water {
        speed /= 5.0;
    }
    if !on_ground {
        speed /= 5.0;
    }
    speed / strength / 30.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStack {
    pub id: i16,
    pub count: u8,
    pub damage: i16,
}

impl ItemStack {
    pub fn new(id: i16, count: u8, damage: i16) -> Self {
        Self { id, count, damage }
    }

    pub fn max_stack_size(&self) -> u8 {
        match self.id {
            // Unstackable tools, armor, doors, buckets, vehicles, records, cake, bed, map
            256..=259
            | 261
            | 267..=279
            | 282..=286
            | 290..=294
            | 298..=317
            | 320
            | 322..=330
            | 333
            | 335
            | 342..=343
            | 346
            | 350
            | 354..=355
            | 358
            | 359
            | 2256..=2257 => 1,
            // Wait: ItemFood sets maxStackSize = 1 for all food except ItemCookie (8)!
            260 | 297 | 319 | 349 => 1,
            COOKIE => 8,
            // 16-stackable items: Snowball, Egg
            SNOW_BALL | EGG => 16,
            _ => 64,
        }
    }

    pub fn is_stackable(&self) -> bool {
        self.max_stack_size() > 1 && (!self.is_damageable() || self.damage == 0)
    }

    pub fn is_damageable(&self) -> bool {
        self.max_damage() > 0 && !uses_data(self.id)
    }

    pub fn uses_data(&self) -> bool {
        uses_data(self.id)
    }

    pub fn max_damage(&self) -> i16 {
        match self.id {
            // Wood tools
            268 | 269 | 270 | 271 | 290 => 59,
            // Stone tools
            272 | 273 | 274 | 275 | 291 => 131,
            // Iron tools
            256 | 257 | 258 | 267 | 292 => 250,
            // Diamond tools
            276 | 277 | 278 | 279 | 293 => 1561,
            // Gold tools
            283 | 284 | 285 | 286 | 294 => 32,
            // Flint and steel
            259 => 64,
            // Bow
            261 => 384,
            // Fishing rod
            346 => 64,
            // Shears
            359 => 238,
            // Armor durability
            298 => 33,  // Leather helmet
            299 => 48,  // Leather chestplate
            300 => 45,  // Leather leggings
            301 => 39,  // Leather boots
            302 => 66,  // Chain helmet
            303 => 96,  // Chain chestplate
            304 => 90,  // Chain leggings
            305 => 78,  // Chain boots
            306 => 135, // Iron helmet
            307 => 192, // Iron chestplate
            308 => 180, // Iron leggings
            309 => 156, // Iron boots
            310 => 272, // Diamond helmet
            311 => 384, // Diamond chestplate
            312 => 360, // Diamond leggings
            313 => 312, // Diamond boots
            314 => 68,  // Gold helmet
            315 => 96,  // Gold chestplate
            316 => 90,  // Gold leggings
            317 => 78,  // Gold boots
            _ => 0,
        }
    }

    pub fn is_food(&self) -> bool {
        food_heal_amount(self.id) > 0
    }

    pub fn food_heal_amount(&self) -> i16 {
        food_heal_amount(self.id)
    }

    pub fn is_block(&self) -> bool {
        self.id > 0 && self.id < 256
    }

    pub fn is_armor(&self) -> bool {
        matches!(self.id, 298..=317)
    }

    pub fn armor_slot(&self) -> Option<usize> {
        match self.id {
            // Helmets
            298 | 302 | 306 | 310 | 314 => Some(3),
            // Chestplates
            299 | 303 | 307 | 311 | 315 => Some(2),
            // Leggings
            300 | 304 | 308 | 312 | 316 => Some(1),
            // Boots
            301 | 305 | 309 | 313 | 317 => Some(0),
            _ => None,
        }
    }

    pub fn armor_defense(&self) -> i32 {
        match self.id {
            298 => 1,
            299 => 3,
            300 => 2,
            301 => 1,
            302 => 2,
            303 => 5,
            304 => 4,
            305 => 1,
            306 => 2,
            307 => 6,
            308 => 5,
            309 => 2,
            310 => 3,
            311 => 8,
            312 => 6,
            313 => 3,
            314 => 2,
            315 => 5,
            316 => 3,
            317 => 1,
            _ => 0,
        }
    }

    pub fn is_tool(&self) -> bool {
        matches!(
            self.id,
            256..=259
                | 261
                | 268..=271
                | 272..=275
                | 276..=279
                | 283..=286
                | 290..=294
                | 346
                | 359
        )
    }

    pub fn is_weapon(&self) -> bool {
        matches!(self.id, 267 | 268 | 272 | 276 | 283)
    }

    pub fn attack_damage(&self) -> i16 {
        match self.id {
            276 => 10, // Diamond Sword: 4 + 3*2 = 10
            267 => 8,  // Iron Sword: 4 + 2*2 = 8
            272 => 6,  // Stone Sword: 4 + 1*2 = 6
            268 => 4,  // Wood Sword: 4 + 0*2 = 4
            283 => 4,  // Gold Sword: 4 + 0*2 = 4
            279 => 6,  // Diamond Axe: 3 + 3 = 6
            258 => 5,  // Iron Axe: 3 + 2 = 5
            275 => 4,  // Stone Axe: 3 + 1 = 4
            271 => 3,  // Wood Axe: 3 + 0 = 3
            286 => 3,  // Gold Axe: 3 + 0 = 3
            278 => 5,  // Diamond Pickaxe: 2 + 3 = 5
            257 => 4,  // Iron Pickaxe: 2 + 2 = 4
            274 => 3,  // Stone Pickaxe: 2 + 1 = 3
            270 => 2,  // Wood Pickaxe: 2 + 0 = 2
            285 => 2,  // Gold Pickaxe: 2 + 0 = 2
            277 => 4,  // Diamond Shovel: 1 + 3 = 4
            256 => 3,  // Iron Shovel: 1 + 2 = 3
            273 => 2,  // Stone Shovel: 1 + 1 = 2
            269 => 1,  // Wood Shovel: 1 + 0 = 1
            284 => 1,  // Gold Shovel: 1 + 0 = 1
            _ => 1,
        }
    }

    pub fn damage_item(&mut self, amount: i16) -> bool {
        if !self.is_damageable() {
            return false;
        }
        let max = self.max_damage();
        self.damage += amount;
        self.damage > max
    }

    /// Java `ItemStack.a(block_id, x, y, z, entity)` — durability cost on breaking a block.
    pub fn on_block_destroyed(&mut self, block_id: u8) -> bool {
        match tool_kind(self.id) {
            Some(ToolKind::Pickaxe(_) | ToolKind::Spade(_) | ToolKind::Axe(_)) => {
                self.damage_item(1)
            }
            Some(ToolKind::Sword(_)) => self.damage_item(2),
            Some(ToolKind::Shears) if block_id == blocks::LEAVES || block_id == blocks::WEB => {
                self.damage_item(1)
            }
            _ => false,
        }
    }

    pub fn tool_dig_speed(&self) -> f32 {
        match self.id {
            // Wood
            268..=271 | 290 => 2.0,
            // Stone
            272..=275 | 291 => 4.0,
            // Iron
            256..=258 | 267 | 292 => 6.0,
            // Diamond
            276..=279 | 293 => 8.0,
            // Gold
            283..=286 | 294 => 12.0,
            _ => 1.0,
        }
    }
}

pub fn food_heal_amount(id: i16) -> i16 {
    match id {
        260 => 4,  // Apple
        282 => 10, // Mushroom soup
        297 => 5,  // Bread
        319 => 3,  // Raw pork
        320 => 8,  // Grilled pork
        322 => 42, // Golden apple (Beta full heal)
        349 => 2,  // Raw fish
        350 => 5,  // Cooked fish
        357 => 1,  // Cookie
        _ => 0,
    }
}

// Item constants (Beta 1.7.3)
pub const IRON_SPADE: i16 = 256;
pub const IRON_PICKAXE: i16 = 257;
pub const IRON_AXE: i16 = 258;
pub const FLINT_AND_STEEL: i16 = 259;
pub const APPLE: i16 = 260;
pub const BOW: i16 = 261;
pub const ARROW: i16 = 262;
pub const COAL: i16 = 263;
pub const DIAMOND: i16 = 264;
pub const IRON_INGOT: i16 = 265;
pub const GOLD_INGOT: i16 = 266;
pub const IRON_SWORD: i16 = 267;
pub const WOOD_SWORD: i16 = 268;
pub const WOOD_SPADE: i16 = 269;
pub const WOOD_PICKAXE: i16 = 270;
pub const WOOD_AXE: i16 = 271;
pub const STONE_SWORD: i16 = 272;
pub const STONE_SPADE: i16 = 273;
pub const STONE_PICKAXE: i16 = 274;
pub const STONE_AXE: i16 = 275;
pub const DIAMOND_SWORD: i16 = 276;
pub const DIAMOND_SPADE: i16 = 277;
pub const DIAMOND_PICKAXE: i16 = 278;
pub const DIAMOND_AXE: i16 = 279;
pub const STICK: i16 = 280;
pub const BOWL: i16 = 281;
pub const MUSHROOM_SOUP: i16 = 282;
pub const GOLD_SWORD: i16 = 283;
pub const GOLD_SPADE: i16 = 284;
pub const GOLD_PICKAXE: i16 = 285;
pub const GOLD_AXE: i16 = 286;
pub const STRING: i16 = 287;
pub const FEATHER: i16 = 288;
pub const SULPHUR: i16 = 289;
pub const WOOD_HOE: i16 = 290;
pub const STONE_HOE: i16 = 291;
pub const IRON_HOE: i16 = 292;
pub const DIAMOND_HOE: i16 = 293;
pub const GOLD_HOE: i16 = 294;
pub const SEEDS: i16 = 295;
pub const WHEAT: i16 = 296;
pub const BREAD: i16 = 297;
pub const LEATHER_HELMET: i16 = 298;
pub const LEATHER_CHESTPLATE: i16 = 299;
pub const LEATHER_LEGGINGS: i16 = 300;
pub const LEATHER_BOOTS: i16 = 301;
pub const CHAINMAIL_HELMET: i16 = 302;
pub const CHAINMAIL_CHESTPLATE: i16 = 303;
pub const CHAINMAIL_LEGGINGS: i16 = 304;
pub const CHAINMAIL_BOOTS: i16 = 305;
pub const IRON_HELMET: i16 = 306;
pub const IRON_CHESTPLATE: i16 = 307;
pub const IRON_LEGGINGS: i16 = 308;
pub const IRON_BOOTS: i16 = 309;
pub const DIAMOND_HELMET: i16 = 310;
pub const DIAMOND_CHESTPLATE: i16 = 311;
pub const DIAMOND_LEGGINGS: i16 = 312;
pub const DIAMOND_BOOTS: i16 = 313;
pub const GOLD_HELMET: i16 = 314;
pub const GOLD_CHESTPLATE: i16 = 315;
pub const GOLD_LEGGINGS: i16 = 316;
pub const GOLD_BOOTS: i16 = 317;
pub const FLINT: i16 = 318;
pub const PORK: i16 = 319;
pub const GRILLED_PORK: i16 = 320;
pub const PAINTING: i16 = 321;
pub const GOLDEN_APPLE: i16 = 322;
pub const SIGN: i16 = 323;
pub const WOOD_DOOR: i16 = 324;
pub const BUCKET: i16 = 325;
pub const WATER_BUCKET: i16 = 326;
pub const LAVA_BUCKET: i16 = 327;
pub const MINECART: i16 = 328;
pub const SADDLE: i16 = 329;
pub const IRON_DOOR: i16 = 330;
pub const REDSTONE: i16 = 331;
pub const SNOW_BALL: i16 = 332;
pub const BOAT: i16 = 333;
pub const LEATHER: i16 = 334;
pub const MILK_BUCKET: i16 = 335;
pub const CLAY_BRICK: i16 = 336;
pub const CLAY_BALL: i16 = 337;
pub const SUGAR_CANE: i16 = 338;
pub const PAPER: i16 = 339;
pub const BOOK: i16 = 340;
pub const SLIME_BALL: i16 = 341;
pub const STORAGE_MINECART: i16 = 342;
pub const POWERED_MINECART: i16 = 343;
pub const EGG: i16 = 344;
pub const COMPASS: i16 = 345;
pub const FISHING_ROD: i16 = 346;
pub const WATCH: i16 = 347;
pub const GLOWSTONE_DUST: i16 = 348;
pub const RAW_FISH: i16 = 349;
pub const COOKED_FISH: i16 = 350;
pub const INK_SACK: i16 = 351;
pub const BONE: i16 = 352;
pub const SUGAR: i16 = 353;
pub const CAKE: i16 = 354;
pub const BED: i16 = 355;
pub const DIODE: i16 = 356;
pub const COOKIE: i16 = 357;
pub const MAP: i16 = 358;
pub const SHEARS: i16 = 359;
pub const GOLD_RECORD: i16 = 2256;
pub const GREEN_RECORD: i16 = 2257;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_stack_stack_limits() {
        let sword = ItemStack::new(DIAMOND_SWORD, 1, 0);
        assert_eq!(sword.max_stack_size(), 1);
        let snow = ItemStack::new(SNOW_BALL, 1, 0);
        assert_eq!(snow.max_stack_size(), 16);
        let cookie = ItemStack::new(COOKIE, 1, 0);
        assert_eq!(cookie.max_stack_size(), 8);
        let pork = ItemStack::new(PORK, 1, 0);
        assert_eq!(pork.max_stack_size(), 1);
        let stone = ItemStack::new(1, 1, 0);
        assert_eq!(stone.max_stack_size(), 64);
    }

    #[test]
    fn food_healing() {
        assert_eq!(food_heal_amount(BREAD), 5);
        assert_eq!(food_heal_amount(GOLDEN_APPLE), 42);
        assert_eq!(food_heal_amount(ARROW), 0);
    }

    #[test]
    fn tool_harvest_levels_match_poseidon() {
        let wood_pick = Some(ItemStack::new(WOOD_PICKAXE, 1, 0));
        let stone_pick = Some(ItemStack::new(STONE_PICKAXE, 1, 0));
        let iron_pick = Some(ItemStack::new(IRON_PICKAXE, 1, 0));
        let diamond_pick = Some(ItemStack::new(DIAMOND_PICKAXE, 1, 0));

        assert!(!can_harvest_block(None, blocks::STONE));
        assert!(can_harvest_block(wood_pick, blocks::STONE));
        assert!(can_harvest_block(wood_pick, blocks::COAL_ORE));
        assert!(!can_harvest_block(wood_pick, blocks::IRON_ORE));
        assert!(can_harvest_block(stone_pick, blocks::IRON_ORE));
        assert!(can_harvest_block(stone_pick, blocks::LAPIS_ORE));
        assert!(!can_harvest_block(stone_pick, blocks::DIAMOND_ORE));
        assert!(can_harvest_block(iron_pick, blocks::DIAMOND_ORE));
        assert!(can_harvest_block(iron_pick, blocks::REDSTONE_ORE));
        assert!(!can_harvest_block(iron_pick, blocks::OBSIDIAN));
        assert!(can_harvest_block(diamond_pick, blocks::OBSIDIAN));

        assert!(can_harvest_block(None, blocks::DIRT));
        assert!(can_harvest_block(None, blocks::LOG));
        assert!(!can_harvest_block(None, blocks::SNOW));
        assert!(can_harvest_block(
            Some(ItemStack::new(WOOD_SPADE, 1, 0)),
            blocks::SNOW
        ));
    }
}
