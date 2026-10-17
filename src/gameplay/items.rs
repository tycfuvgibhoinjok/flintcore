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
            // Unstackable tools and armor
            256..=259
            | 261
            | 267..=279
            | 282..=286
            | 290..=294
            | 298..=317
            | 324..=330
            | 333
            | 335
            | 342..=343
            | 346
            | 355
            | 358
            | 359
            | 2256..=2257 => 1,
            // 16-stackable items
            332 | 344 | 351 | 356 => 16,
            _ => 64,
        }
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
            276 => 7, // Diamond Sword
            267 => 6, // Iron Sword
            272 => 5, // Stone Sword
            268 => 4, // Wood Sword
            283 => 4, // Gold Sword
            279 => 6, // Diamond Axe
            258 => 5, // Iron Axe
            275 => 4, // Stone Axe
            271 => 3, // Wood Axe
            286 => 3, // Gold Axe
            278 => 5, // Diamond Pickaxe
            257 => 4, // Iron Pickaxe
            274 => 3, // Stone Pickaxe
            270 => 2, // Wood Pickaxe
            285 => 2, // Gold Pickaxe
            277 => 4, // Diamond Shovel
            256 => 3, // Iron Shovel
            273 => 2, // Stone Shovel
            269 => 1, // Wood Shovel
            284 => 1, // Gold Shovel
            _ => 1,
        }
    }

    pub fn damage_item(&mut self, amount: i16) -> bool {
        let max = self.max_damage();
        if max == 0 {
            return false;
        }
        self.damage += amount;
        self.damage >= max
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_stack_stack_limits() {
        let sword = ItemStack::new(DIAMOND_SWORD, 1, 0);
        assert_eq!(sword.max_stack_size(), 1);
        let snow = ItemStack::new(SNOW_BALL, 1, 0);
        assert_eq!(snow.max_stack_size(), 16);
        let stone = ItemStack::new(1, 1, 0);
        assert_eq!(stone.max_stack_size(), 64);
    }

    #[test]
    fn food_healing() {
        assert_eq!(food_heal_amount(BREAD), 5);
        assert_eq!(food_heal_amount(GOLDEN_APPLE), 42);
        assert_eq!(food_heal_amount(ARROW), 0);
    }
}
