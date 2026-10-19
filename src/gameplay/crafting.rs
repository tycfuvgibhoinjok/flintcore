use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use crate::gameplay::blocks::{self, Material};
use crate::gameplay::items::{self, ItemStack};

static CRAFTING_FLAGS: AtomicU8 = AtomicU8::new(0);

pub fn set_crafting_mechanics(
    glowstone_pre_1_6_6: bool,
    wool_recipe_pre_1_6_6: bool,
    old_slab_recipe: bool,
    allow_ladder_gap: bool,
) {
    let mut bits = 0u8;
    if glowstone_pre_1_6_6 {
        bits |= 1;
    }
    if wool_recipe_pre_1_6_6 {
        bits |= 2;
    }
    if old_slab_recipe {
        bits |= 4;
    }
    if allow_ladder_gap {
        bits |= 8;
    }
    CRAFTING_FLAGS.store(bits, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ingredient {
    pub id: i16,
    /// `-1` matches any damage/metadata (wildcard).
    pub damage: i16,
}

impl Ingredient {
    pub const fn item(id: i16) -> Self {
        Self { id, damage: 0 }
    }

    pub const fn block(id: u8) -> Self {
        Self {
            id: id as i16,
            damage: -1,
        }
    }

    pub const fn block_exact(id: u8, damage: i16) -> Self {
        Self {
            id: id as i16,
            damage,
        }
    }

    pub const fn stack(id: i16, damage: i16) -> Self {
        Self { id, damage }
    }

    pub fn matches(self, stack: ItemStack) -> bool {
        stack.id == self.id && (self.damage == -1 || stack.damage == self.damage)
    }
}

#[derive(Debug, Clone)]
pub struct ShapedRecipe {
    pub width: usize,
    pub height: usize,
    pub pattern: Vec<Option<Ingredient>>,
    pub result: ItemStack,
}

impl ShapedRecipe {
    pub fn size(&self) -> usize {
        self.width * self.height
    }

    pub fn matches(&self, grid: &[Option<ItemStack>; 9]) -> bool {
        for offset_x in 0..=(3 - self.width) {
            for offset_y in 0..=(3 - self.height) {
                if self.check_match(grid, offset_x, offset_y, true) {
                    return true;
                }
                if self.check_match(grid, offset_x, offset_y, false) {
                    return true;
                }
            }
        }
        false
    }

    fn check_match(
        &self,
        grid: &[Option<ItemStack>; 9],
        offset_x: usize,
        offset_y: usize,
        mirror: bool,
    ) -> bool {
        for x in 0..3 {
            for y in 0..3 {
                let expected = if x >= offset_x
                    && y >= offset_y
                    && (x - offset_x) < self.width
                    && (y - offset_y) < self.height
                {
                    let rx = x - offset_x;
                    let ry = y - offset_y;
                    let pat_idx = if mirror {
                        (self.width - rx - 1) + ry * self.width
                    } else {
                        rx + ry * self.width
                    };
                    self.pattern[pat_idx]
                } else {
                    None
                };

                let actual = grid[x + y * 3];
                match (expected, actual) {
                    (None, None) => {}
                    (Some(ing), Some(stack)) => {
                        if !ing.matches(stack) {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
pub struct ShapelessRecipe {
    pub ingredients: Vec<Ingredient>,
    pub result: ItemStack,
}

impl ShapelessRecipe {
    pub fn size(&self) -> usize {
        self.ingredients.len()
    }

    pub fn matches(&self, grid: &[Option<ItemStack>; 9]) -> bool {
        let mut remaining = self.ingredients.clone();
        for slot in grid.iter().flatten() {
            if let Some(pos) = remaining.iter().position(|ing| ing.matches(*slot)) {
                remaining.swap_remove(pos);
            } else {
                return false;
            }
        }
        remaining.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum Recipe {
    Shaped(ShapedRecipe),
    Shapeless(ShapelessRecipe),
}

impl Recipe {
    pub fn matches(&self, grid: &[Option<ItemStack>; 9]) -> bool {
        match self {
            Self::Shaped(r) => r.matches(grid),
            Self::Shapeless(r) => r.matches(grid),
        }
    }

    pub fn result(&self) -> ItemStack {
        match self {
            Self::Shaped(r) => r.result,
            Self::Shapeless(r) => r.result,
        }
    }
}

pub struct CraftingManager {
    recipes: Vec<Recipe>,
}

static INSTANCE: OnceLock<CraftingManager> = OnceLock::new();

impl CraftingManager {
    pub fn instance() -> &'static Self {
        INSTANCE.get_or_init(Self::new)
    }

    pub fn new() -> Self {
        let mut mgr = Self {
            recipes: Vec::with_capacity(160),
        };
        mgr.register_tools();
        mgr.register_weapons();
        mgr.register_ingots();
        mgr.register_food();
        mgr.register_crafting_blocks();
        mgr.register_armor();
        mgr.register_dyes();
        mgr.register_misc();

        // Mirror RecipeSorter: Shaped before Shapeless, then descending recipe size
        mgr.recipes.sort_by(|a, b| match (a, b) {
            (Recipe::Shapeless(_), Recipe::Shaped(_)) => std::cmp::Ordering::Greater,
            (Recipe::Shaped(_), Recipe::Shapeless(_)) => std::cmp::Ordering::Less,
            (Recipe::Shaped(ra), Recipe::Shaped(rb)) => rb.size().cmp(&ra.size()),
            (Recipe::Shapeless(ra), Recipe::Shapeless(rb)) => rb.size().cmp(&ra.size()),
        });

        mgr
    }

    pub fn add_shaped(
        &mut self,
        result: ItemStack,
        rows: &[&str],
        bindings: &[(char, Ingredient)],
    ) {
        let height = rows.len();
        let width = rows.first().map(|r| r.len()).unwrap_or(0);
        let mut pattern = Vec::with_capacity(width * height);
        for row in rows {
            for ch in row.chars() {
                let found = bindings
                    .iter()
                    .find_map(|&(c, ing)| if c == ch { Some(ing) } else { None });
                pattern.push(found);
            }
        }
        self.recipes.push(Recipe::Shaped(ShapedRecipe {
            width,
            height,
            pattern,
            result,
        }));
    }

    pub fn add_shapeless(&mut self, result: ItemStack, ingredients: Vec<Ingredient>) {
        self.recipes.push(Recipe::Shapeless(ShapelessRecipe {
            ingredients,
            result,
        }));
    }

    fn register_tools(&mut self) {
        let materials = [
            Ingredient::block(blocks::WOOD_PLANKS),
            Ingredient::block(blocks::COBBLESTONE),
            Ingredient::item(items::IRON_INGOT),
            Ingredient::item(items::DIAMOND),
            Ingredient::item(items::GOLD_INGOT),
        ];
        let pickaxes = [
            items::WOOD_PICKAXE,
            items::STONE_PICKAXE,
            items::IRON_PICKAXE,
            items::DIAMOND_PICKAXE,
            items::GOLD_PICKAXE,
        ];
        let spades = [
            items::WOOD_SPADE,
            items::STONE_SPADE,
            items::IRON_SPADE,
            items::DIAMOND_SPADE,
            items::GOLD_SPADE,
        ];
        let axes = [
            items::WOOD_AXE,
            items::STONE_AXE,
            items::IRON_AXE,
            items::DIAMOND_AXE,
            items::GOLD_AXE,
        ];
        let hoes = [
            items::WOOD_HOE,
            items::STONE_HOE,
            items::IRON_HOE,
            items::DIAMOND_HOE,
            items::GOLD_HOE,
        ];
        let stick = Ingredient::item(items::STICK);

        for i in 0..5 {
            let mat = materials[i];
            self.add_shaped(
                ItemStack::new(pickaxes[i], 1, 0),
                &["XXX", " # ", " # "],
                &[('#', stick), ('X', mat)],
            );
            self.add_shaped(
                ItemStack::new(spades[i], 1, 0),
                &["X", "#", "#"],
                &[('#', stick), ('X', mat)],
            );
            self.add_shaped(
                ItemStack::new(axes[i], 1, 0),
                &["XX", "X#", " #"],
                &[('#', stick), ('X', mat)],
            );
            self.add_shaped(
                ItemStack::new(hoes[i], 1, 0),
                &["XX", " #", " #"],
                &[('#', stick), ('X', mat)],
            );
        }

        self.add_shaped(
            ItemStack::new(items::SHEARS, 1, 0),
            &[" #", "# "],
            &[('#', Ingredient::item(items::IRON_INGOT))],
        );
    }

    fn register_weapons(&mut self) {
        let materials = [
            Ingredient::block(blocks::WOOD_PLANKS),
            Ingredient::block(blocks::COBBLESTONE),
            Ingredient::item(items::IRON_INGOT),
            Ingredient::item(items::DIAMOND),
            Ingredient::item(items::GOLD_INGOT),
        ];
        let swords = [
            items::WOOD_SWORD,
            items::STONE_SWORD,
            items::IRON_SWORD,
            items::DIAMOND_SWORD,
            items::GOLD_SWORD,
        ];
        let stick = Ingredient::item(items::STICK);

        for i in 0..5 {
            self.add_shaped(
                ItemStack::new(swords[i], 1, 0),
                &["X", "X", "#"],
                &[('#', stick), ('X', materials[i])],
            );
        }

        self.add_shaped(
            ItemStack::new(items::BOW, 1, 0),
            &[" #X", "# X", " #X"],
            &[('X', Ingredient::item(items::STRING)), ('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(items::ARROW, 4, 0),
            &["X", "#", "Y"],
            &[
                ('Y', Ingredient::item(items::FEATHER)),
                ('X', Ingredient::item(items::FLINT)),
                ('#', stick),
            ],
        );
    }

    fn register_ingots(&mut self) {
        let pairs = [
            (blocks::GOLD_BLOCK, ItemStack::new(items::GOLD_INGOT, 9, 0)),
            (blocks::IRON_BLOCK, ItemStack::new(items::IRON_INGOT, 9, 0)),
            (blocks::DIAMOND_BLOCK, ItemStack::new(items::DIAMOND, 9, 0)),
            (blocks::LAPIS_BLOCK, ItemStack::new(items::INK_SACK, 9, 4)),
        ];
        for (block_id, ingots) in pairs {
            self.add_shaped(
                ItemStack::new(block_id as i16, 1, 0),
                &["###", "###", "###"],
                &[('#', Ingredient::stack(ingots.id, ingots.damage))],
            );
            self.add_shaped(ingots, &["#"], &[('#', Ingredient::block(block_id))]);
        }
    }

    fn register_food(&mut self) {
        self.add_shaped(
            ItemStack::new(items::MUSHROOM_SOUP, 1, 0),
            &["Y", "X", "#"],
            &[
                ('X', Ingredient::block(blocks::BROWN_MUSHROOM)),
                ('Y', Ingredient::block(blocks::RED_MUSHROOM)),
                ('#', Ingredient::item(items::BOWL)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::MUSHROOM_SOUP, 1, 0),
            &["Y", "X", "#"],
            &[
                ('X', Ingredient::block(blocks::RED_MUSHROOM)),
                ('Y', Ingredient::block(blocks::BROWN_MUSHROOM)),
                ('#', Ingredient::item(items::BOWL)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::COOKIE, 8, 0),
            &["#X#"],
            &[
                ('X', Ingredient::stack(items::INK_SACK, 3)),
                ('#', Ingredient::item(items::WHEAT)),
            ],
        );
    }

    fn register_crafting_blocks(&mut self) {
        self.add_shaped(
            ItemStack::new(blocks::CHEST as i16, 1, 0),
            &["###", "# #", "###"],
            &[('#', Ingredient::block(blocks::WOOD_PLANKS))],
        );
        self.add_shaped(
            ItemStack::new(blocks::FURNACE as i16, 1, 0),
            &["###", "# #", "###"],
            &[('#', Ingredient::block(blocks::COBBLESTONE))],
        );
        self.add_shaped(
            ItemStack::new(blocks::WORKBENCH as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::block(blocks::WOOD_PLANKS))],
        );
        self.add_shaped(
            ItemStack::new(blocks::SANDSTONE as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::block(blocks::SAND))],
        );
    }

    fn register_armor(&mut self) {
        let materials = [
            Ingredient::item(items::LEATHER),
            Ingredient::block(blocks::FIRE),
            Ingredient::item(items::IRON_INGOT),
            Ingredient::item(items::DIAMOND),
            Ingredient::item(items::GOLD_INGOT),
        ];
        let helmets = [
            items::LEATHER_HELMET,
            items::CHAINMAIL_HELMET,
            items::IRON_HELMET,
            items::DIAMOND_HELMET,
            items::GOLD_HELMET,
        ];
        let chestplates = [
            items::LEATHER_CHESTPLATE,
            items::CHAINMAIL_CHESTPLATE,
            items::IRON_CHESTPLATE,
            items::DIAMOND_CHESTPLATE,
            items::GOLD_CHESTPLATE,
        ];
        let leggings = [
            items::LEATHER_LEGGINGS,
            items::CHAINMAIL_LEGGINGS,
            items::IRON_LEGGINGS,
            items::DIAMOND_LEGGINGS,
            items::GOLD_LEGGINGS,
        ];
        let boots = [
            items::LEATHER_BOOTS,
            items::CHAINMAIL_BOOTS,
            items::IRON_BOOTS,
            items::DIAMOND_BOOTS,
            items::GOLD_BOOTS,
        ];

        for i in 0..5 {
            let mat = materials[i];
            self.add_shaped(
                ItemStack::new(helmets[i], 1, 0),
                &["XXX", "X X"],
                &[('X', mat)],
            );
            self.add_shaped(
                ItemStack::new(chestplates[i], 1, 0),
                &["X X", "XXX", "XXX"],
                &[('X', mat)],
            );
            self.add_shaped(
                ItemStack::new(leggings[i], 1, 0),
                &["XXX", "X X", "X X"],
                &[('X', mat)],
            );
            self.add_shaped(
                ItemStack::new(boots[i], 1, 0),
                &["X X", "X X"],
                &[('X', mat)],
            );
        }
    }

    fn register_dyes(&mut self) {
        for i in 0..16i16 {
            let wool_color = (!i) & 15;
            self.add_shapeless(
                ItemStack::new(blocks::WOOL as i16, 1, wool_color),
                vec![
                    Ingredient::stack(items::INK_SACK, i),
                    Ingredient::block_exact(blocks::WOOL, 0),
                ],
            );
        }

        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 11),
            vec![Ingredient::block_exact(blocks::YELLOW_FLOWER, 0)],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 1),
            vec![Ingredient::block_exact(blocks::RED_ROSE, 0)],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 3, 15),
            vec![Ingredient::item(items::BONE)],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 9),
            vec![
                Ingredient::stack(items::INK_SACK, 1),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 14),
            vec![
                Ingredient::stack(items::INK_SACK, 1),
                Ingredient::stack(items::INK_SACK, 11),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 10),
            vec![
                Ingredient::stack(items::INK_SACK, 2),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 8),
            vec![
                Ingredient::stack(items::INK_SACK, 0),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 7),
            vec![
                Ingredient::stack(items::INK_SACK, 8),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 3, 7),
            vec![
                Ingredient::stack(items::INK_SACK, 0),
                Ingredient::stack(items::INK_SACK, 15),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 12),
            vec![
                Ingredient::stack(items::INK_SACK, 4),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 6),
            vec![
                Ingredient::stack(items::INK_SACK, 4),
                Ingredient::stack(items::INK_SACK, 2),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 5),
            vec![
                Ingredient::stack(items::INK_SACK, 4),
                Ingredient::stack(items::INK_SACK, 1),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 2, 13),
            vec![
                Ingredient::stack(items::INK_SACK, 5),
                Ingredient::stack(items::INK_SACK, 9),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 3, 13),
            vec![
                Ingredient::stack(items::INK_SACK, 4),
                Ingredient::stack(items::INK_SACK, 1),
                Ingredient::stack(items::INK_SACK, 9),
            ],
        );
        self.add_shapeless(
            ItemStack::new(items::INK_SACK, 4, 13),
            vec![
                Ingredient::stack(items::INK_SACK, 4),
                Ingredient::stack(items::INK_SACK, 1),
                Ingredient::stack(items::INK_SACK, 1),
                Ingredient::stack(items::INK_SACK, 15),
            ],
        );
    }

    fn register_misc(&mut self) {
        let wood = Ingredient::block(blocks::WOOD_PLANKS);
        let cobble = Ingredient::block(blocks::COBBLESTONE);
        let stone = Ingredient::block(blocks::STONE);
        let stick = Ingredient::item(items::STICK);
        let iron = Ingredient::item(items::IRON_INGOT);
        let gold = Ingredient::item(items::GOLD_INGOT);
        let redstone = Ingredient::item(items::REDSTONE);

        self.add_shaped(
            ItemStack::new(items::PAPER, 3, 0),
            &["###"],
            &[('#', Ingredient::item(items::SUGAR_CANE))],
        );
        self.add_shaped(
            ItemStack::new(items::BOOK, 1, 0),
            &["#", "#", "#"],
            &[('#', Ingredient::item(items::PAPER))],
        );
        self.add_shaped(
            ItemStack::new(blocks::FENCE as i16, 2, 0),
            &["###", "###"],
            &[('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(blocks::JUKEBOX as i16, 1, 0),
            &["###", "#X#", "###"],
            &[('#', wood), ('X', Ingredient::item(items::DIAMOND))],
        );
        self.add_shaped(
            ItemStack::new(blocks::NOTE_BLOCK as i16, 1, 0),
            &["###", "#X#", "###"],
            &[('#', wood), ('X', redstone)],
        );
        self.add_shaped(
            ItemStack::new(blocks::BOOKSHELF as i16, 1, 0),
            &["###", "XXX", "###"],
            &[('#', wood), ('X', Ingredient::item(items::BOOK))],
        );
        self.add_shaped(
            ItemStack::new(blocks::SNOW_BLOCK as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::item(items::SNOW_BALL))],
        );
        self.add_shaped(
            ItemStack::new(blocks::CLAY as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::item(items::CLAY_BALL))],
        );
        self.add_shaped(
            ItemStack::new(blocks::BRICK as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::item(items::CLAY_BRICK))],
        );
        self.add_shaped(
            ItemStack::new(blocks::GLOWSTONE as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::item(items::GLOWSTONE_DUST))],
        );
        self.add_shaped(
            ItemStack::new(blocks::WOOL as i16, 1, 0),
            &["##", "##"],
            &[('#', Ingredient::item(items::STRING))],
        );
        self.add_shaped(
            ItemStack::new(blocks::TNT as i16, 1, 0),
            &["X#X", "#X#", "X#X"],
            &[
                ('X', Ingredient::item(items::SULPHUR)),
                ('#', Ingredient::block(blocks::SAND)),
            ],
        );
        self.add_shaped(
            ItemStack::new(blocks::STEP as i16, 3, 3),
            &["###"],
            &[('#', cobble)],
        );
        self.add_shaped(
            ItemStack::new(blocks::STEP as i16, 3, 0),
            &["###"],
            &[('#', stone)],
        );
        self.add_shaped(
            ItemStack::new(blocks::STEP as i16, 3, 1),
            &["###"],
            &[('#', Ingredient::block(blocks::SANDSTONE))],
        );
        self.add_shaped(
            ItemStack::new(blocks::STEP as i16, 3, 2),
            &["###"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(blocks::LADDER as i16, 2, 0),
            &["# #", "###", "# #"],
            &[('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(items::WOOD_DOOR, 1, 0),
            &["##", "##", "##"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(blocks::TRAPDOOR as i16, 2, 0),
            &["###", "###"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(items::IRON_DOOR, 1, 0),
            &["##", "##", "##"],
            &[('#', iron)],
        );
        self.add_shaped(
            ItemStack::new(items::SIGN, 1, 0),
            &["###", "###", " X "],
            &[('#', wood), ('X', stick)],
        );
        self.add_shaped(
            ItemStack::new(items::CAKE, 1, 0),
            &["AAA", "BEB", "CCC"],
            &[
                ('A', Ingredient::item(items::MILK_BUCKET)),
                ('B', Ingredient::item(items::SUGAR)),
                ('C', Ingredient::item(items::WHEAT)),
                ('E', Ingredient::item(items::EGG)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::SUGAR, 1, 0),
            &["#"],
            &[('#', Ingredient::item(items::SUGAR_CANE))],
        );
        self.add_shaped(
            ItemStack::new(blocks::WOOD_PLANKS as i16, 4, 0),
            &["#"],
            &[('#', Ingredient::block(blocks::LOG))],
        );
        self.add_shaped(
            ItemStack::new(items::STICK, 4, 0),
            &["#", "#"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(blocks::TORCH as i16, 4, 0),
            &["X", "#"],
            &[('X', Ingredient::stack(items::COAL, 0)), ('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(blocks::TORCH as i16, 4, 0),
            &["X", "#"],
            &[('X', Ingredient::stack(items::COAL, 1)), ('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(items::BOWL, 4, 0),
            &["# #", " # "],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(blocks::RAIL as i16, 16, 0),
            &["X X", "X#X", "X X"],
            &[('X', iron), ('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(blocks::POWERED_RAIL as i16, 6, 0),
            &["X X", "X#X", "XRX"],
            &[('X', gold), ('R', redstone), ('#', stick)],
        );
        self.add_shaped(
            ItemStack::new(blocks::DETECTOR_RAIL as i16, 6, 0),
            &["X X", "X#X", "XRX"],
            &[
                ('X', iron),
                ('R', redstone),
                ('#', Ingredient::block(blocks::STONE_PLATE)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::MINECART, 1, 0),
            &["# #", "###"],
            &[('#', iron)],
        );
        self.add_shaped(
            ItemStack::new(blocks::JACK_O_LANTERN as i16, 1, 0),
            &["A", "B"],
            &[
                ('A', Ingredient::block(blocks::PUMPKIN)),
                ('B', Ingredient::block(blocks::TORCH)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::STORAGE_MINECART, 1, 0),
            &["A", "B"],
            &[
                ('A', Ingredient::block(blocks::CHEST)),
                ('B', Ingredient::item(items::MINECART)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::POWERED_MINECART, 1, 0),
            &["A", "B"],
            &[
                ('A', Ingredient::block(blocks::FURNACE)),
                ('B', Ingredient::item(items::MINECART)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::BOAT, 1, 0),
            &["# #", "###"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(items::BUCKET, 1, 0),
            &["# #", " # "],
            &[('#', iron)],
        );
        self.add_shaped(
            ItemStack::new(items::FLINT_AND_STEEL, 1, 0),
            &["A ", " B"],
            &[('A', iron), ('B', Ingredient::item(items::FLINT))],
        );
        self.add_shaped(
            ItemStack::new(items::BREAD, 1, 0),
            &["###"],
            &[('#', Ingredient::item(items::WHEAT))],
        );
        self.add_shaped(
            ItemStack::new(blocks::WOOD_STAIRS as i16, 4, 0),
            &["#  ", "## ", "###"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(items::FISHING_ROD, 1, 0),
            &["  #", " #X", "# X"],
            &[('#', stick), ('X', Ingredient::item(items::STRING))],
        );
        self.add_shaped(
            ItemStack::new(blocks::COBBLESTONE_STAIRS as i16, 4, 0),
            &["#  ", "## ", "###"],
            &[('#', cobble)],
        );
        self.add_shaped(
            ItemStack::new(items::PAINTING, 1, 0),
            &["###", "#X#", "###"],
            &[('#', stick), ('X', Ingredient::block(blocks::WOOL))],
        );
        self.add_shaped(
            ItemStack::new(items::GOLDEN_APPLE, 1, 0),
            &["###", "#X#", "###"],
            &[
                ('#', Ingredient::block(blocks::GOLD_BLOCK)),
                ('X', Ingredient::item(items::APPLE)),
            ],
        );
        self.add_shaped(
            ItemStack::new(blocks::LEVER as i16, 1, 0),
            &["X", "#"],
            &[('#', cobble), ('X', stick)],
        );
        self.add_shaped(
            ItemStack::new(blocks::REDSTONE_TORCH_ON as i16, 1, 0),
            &["X", "#"],
            &[('#', stick), ('X', redstone)],
        );
        self.add_shaped(
            ItemStack::new(items::DIODE, 1, 0),
            &["#X#", "III"],
            &[
                ('#', Ingredient::block(blocks::REDSTONE_TORCH_ON)),
                ('X', redstone),
                ('I', stone),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::WATCH, 1, 0),
            &[" # ", "#X#", " # "],
            &[('#', gold), ('X', redstone)],
        );
        self.add_shaped(
            ItemStack::new(items::COMPASS, 1, 0),
            &[" # ", "#X#", " # "],
            &[('#', iron), ('X', redstone)],
        );
        self.add_shaped(
            ItemStack::new(items::MAP, 1, 0),
            &["###", "#X#", "###"],
            &[
                ('#', Ingredient::item(items::PAPER)),
                ('X', Ingredient::item(items::COMPASS)),
            ],
        );
        self.add_shaped(
            ItemStack::new(blocks::STONE_BUTTON as i16, 1, 0),
            &["#", "#"],
            &[('#', stone)],
        );
        self.add_shaped(
            ItemStack::new(blocks::STONE_PLATE as i16, 1, 0),
            &["##"],
            &[('#', stone)],
        );
        self.add_shaped(
            ItemStack::new(blocks::WOOD_PLATE as i16, 1, 0),
            &["##"],
            &[('#', wood)],
        );
        self.add_shaped(
            ItemStack::new(blocks::DISPENSER as i16, 1, 0),
            &["###", "#X#", "#R#"],
            &[
                ('#', cobble),
                ('X', Ingredient::item(items::BOW)),
                ('R', redstone),
            ],
        );
        self.add_shaped(
            ItemStack::new(blocks::PISTON as i16, 1, 0),
            &["TTT", "#X#", "#R#"],
            &[('T', wood), ('#', cobble), ('X', iron), ('R', redstone)],
        );
        self.add_shaped(
            ItemStack::new(blocks::STICKY_PISTON as i16, 1, 0),
            &["S", "P"],
            &[
                ('S', Ingredient::item(items::SLIME_BALL)),
                ('P', Ingredient::block(blocks::PISTON)),
            ],
        );
        self.add_shaped(
            ItemStack::new(items::BED, 1, 0),
            &["###", "XXX"],
            &[('#', Ingredient::block(blocks::WOOL)), ('X', wood)],
        );
    }

    pub fn craft_3x3(&self, grid: &[Option<ItemStack>; 9]) -> Option<ItemStack> {
        let flags = CRAFTING_FLAGS.load(Ordering::Relaxed);
        if (flags & 1) != 0
            && grid
                .iter()
                .all(|s| s.is_some_and(|st| st.id == items::GLOWSTONE_DUST))
        {
            return Some(ItemStack::new(blocks::GLOWSTONE as i16, 1, 0));
        }
        if (flags & 2) != 0
            && grid
                .iter()
                .all(|s| s.is_some_and(|st| st.id == items::STRING))
        {
            return Some(ItemStack::new(blocks::WOOL as i16, 1, 0));
        }

        for recipe in &self.recipes {
            if recipe.matches(grid) {
                let mut res = recipe.result();
                if (flags & 1) != 0 && res.id == blocks::GLOWSTONE as i16 {
                    continue;
                }
                if (flags & 2) != 0
                    && res.id == blocks::WOOL as i16
                    && grid.iter().flatten().any(|st| st.id == items::STRING)
                {
                    continue;
                }
                if (flags & 4) != 0 && res.id == blocks::STEP as i16 && res.damage == 3 {
                    res.damage = 0;
                }
                if (flags & 8) != 0 && res.id == blocks::LADDER as i16 {
                    res.count = 1;
                }
                return Some(res);
            }
        }
        None
    }

    pub fn craft_2x2(&self, grid_2x2: &[Option<ItemStack>; 4]) -> Option<ItemStack> {
        let grid_3x3 = [
            grid_2x2[0],
            grid_2x2[1],
            None,
            grid_2x2[2],
            grid_2x2[3],
            None,
            None,
            None,
            None,
        ];
        self.craft_3x3(&grid_3x3)
    }

    pub fn recipe_count(&self) -> usize {
        self.recipes.len()
    }
}

impl Default for CraftingManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Java `FurnaceRecipes.a(int itemId)` — smelting result for input item ID.
pub fn furnace_smelt_result(input_id: i16) -> Option<ItemStack> {
    match input_id {
        id if id == blocks::IRON_ORE as i16 => Some(ItemStack::new(items::IRON_INGOT, 1, 0)),
        id if id == blocks::GOLD_ORE as i16 => Some(ItemStack::new(items::GOLD_INGOT, 1, 0)),
        id if id == blocks::DIAMOND_ORE as i16 => Some(ItemStack::new(items::DIAMOND, 1, 0)),
        id if id == blocks::SAND as i16 => Some(ItemStack::new(blocks::GLASS as i16, 1, 0)),
        items::PORK => Some(ItemStack::new(items::GRILLED_PORK, 1, 0)),
        items::RAW_FISH => Some(ItemStack::new(items::COOKED_FISH, 1, 0)),
        id if id == blocks::COBBLESTONE as i16 => Some(ItemStack::new(blocks::STONE as i16, 1, 0)),
        items::CLAY_BALL => Some(ItemStack::new(items::CLAY_BRICK, 1, 0)),
        id if id == blocks::CACTUS as i16 => Some(ItemStack::new(items::INK_SACK, 1, 2)),
        id if id == blocks::LOG as i16 => Some(ItemStack::new(items::COAL, 1, 1)),
        _ => None,
    }
}

/// Java `TileEntityFurnace.fuelTime(ItemStack)` — burn time in ticks.
pub fn furnace_fuel_time(stack: Option<ItemStack>) -> i16 {
    let Some(item) = stack else {
        return 0;
    };
    if item.id > 0 && item.id < 256 && blocks::material(item.id as u8) == Material::Wood {
        300
    } else if item.id == items::STICK || item.id == blocks::SAPLING as i16 {
        100
    } else if item.id == items::COAL {
        1600
    } else if item.id == items::LAVA_BUCKET {
        20000
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_count_matches_poseidon() {
        let mgr = CraftingManager::instance();
        assert_eq!(mgr.recipe_count(), 151);
    }

    #[test]
    fn craft_2x2_planks_sticks_workbench_torches() {
        let mgr = CraftingManager::instance();

        // Any log metadata crafts 4 planks (meta 0)
        let log_grid = [
            Some(ItemStack::new(blocks::LOG as i16, 1, 2)),
            None,
            None,
            None,
        ];
        assert_eq!(
            mgr.craft_2x2(&log_grid),
            Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 4, 0))
        );

        // 4 planks craft workbench
        let plank = Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 1, 0));
        assert_eq!(
            mgr.craft_2x2(&[plank, plank, plank, plank]),
            Some(ItemStack::new(blocks::WORKBENCH as i16, 1, 0))
        );

        // 2 planks vertically craft 4 sticks (either column)
        assert_eq!(
            mgr.craft_2x2(&[plank, None, plank, None]),
            Some(ItemStack::new(items::STICK, 4, 0))
        );
        assert_eq!(
            mgr.craft_2x2(&[None, plank, None, plank]),
            Some(ItemStack::new(items::STICK, 4, 0))
        );

        // Charcoal + stick crafts 4 torches
        let charcoal = Some(ItemStack::new(items::COAL, 1, 1));
        let stick = Some(ItemStack::new(items::STICK, 1, 0));
        assert_eq!(
            mgr.craft_2x2(&[charcoal, None, stick, None]),
            Some(ItemStack::new(blocks::TORCH as i16, 4, 0))
        );
    }

    #[test]
    fn craft_3x3_tools_and_mirrored_recipes() {
        let mgr = CraftingManager::instance();
        let dia = Some(ItemStack::new(items::DIAMOND, 1, 0));
        let stick = Some(ItemStack::new(items::STICK, 1, 0));

        // Diamond pickaxe
        let pick_grid = [dia, dia, dia, None, stick, None, None, stick, None];
        assert_eq!(
            mgr.craft_3x3(&pick_grid),
            Some(ItemStack::new(items::DIAMOND_PICKAXE, 1, 0))
        );

        // Diamond axe normal and mirrored
        let axe_left = [dia, dia, None, dia, stick, None, None, stick, None];
        let axe_right = [None, dia, dia, None, stick, dia, None, stick, None];
        assert_eq!(
            mgr.craft_3x3(&axe_left),
            Some(ItemStack::new(items::DIAMOND_AXE, 1, 0))
        );
        assert_eq!(
            mgr.craft_3x3(&axe_right),
            Some(ItemStack::new(items::DIAMOND_AXE, 1, 0))
        );
    }

    #[test]
    fn shapeless_dye_and_furnace_recipes() {
        let mgr = CraftingManager::instance();
        let bone = Some(ItemStack::new(items::BONE, 1, 0));
        assert_eq!(
            mgr.craft_2x2(&[None, bone, None, None]),
            Some(ItemStack::new(items::INK_SACK, 3, 15))
        );

        // Red dye (damage=1) + White wool (damage=0) -> Red wool (damage = !1 & 15 = 14)
        let red_dye = Some(ItemStack::new(items::INK_SACK, 1, 1));
        let white_wool = Some(ItemStack::new(blocks::WOOL as i16, 1, 0));
        assert_eq!(
            mgr.craft_2x2(&[red_dye, white_wool, None, None]),
            Some(ItemStack::new(blocks::WOOL as i16, 1, 14))
        );

        assert_eq!(
            furnace_smelt_result(blocks::IRON_ORE as i16),
            Some(ItemStack::new(items::IRON_INGOT, 1, 0))
        );
        assert_eq!(
            furnace_smelt_result(blocks::LOG as i16),
            Some(ItemStack::new(items::COAL, 1, 1))
        );
        assert_eq!(
            furnace_fuel_time(Some(ItemStack::new(items::COAL, 1, 0))),
            1600
        );
        assert_eq!(
            furnace_fuel_time(Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 1, 0))),
            300
        );
    }

    #[test]
    fn uberbukkit_recipe_toggles() {
        let mgr = CraftingManager::instance();
        let dust = Some(ItemStack::new(items::GLOWSTONE_DUST, 1, 0));
        let stick = Some(ItemStack::new(items::STICK, 1, 0));
        let cobble = Some(ItemStack::new(blocks::COBBLESTONE as i16, 1, 0));

        set_crafting_mechanics(true, true, true, true);
        // 2x2 glowstone should not craft when `glowstone_pre_1_6_6 = true`, 3x3 should!
        assert_eq!(mgr.craft_2x2(&[dust, dust, dust, dust]), None);
        assert_eq!(
            mgr.craft_3x3(&[dust; 9]),
            Some(ItemStack::new(blocks::GLOWSTONE as i16, 1, 0))
        );
        // Ladder crafts 1 instead of 2 when `allow_ladder_gap = true`
        let ladder_grid = [stick, None, stick, stick, stick, stick, stick, None, stick];
        assert_eq!(
            mgr.craft_3x3(&ladder_grid),
            Some(ItemStack::new(blocks::LADDER as i16, 1, 0))
        );
        // Cobblestone crafts stone slab (damage 0) when `old_slab_recipe = true`
        let slab_grid = [cobble, cobble, cobble, None, None, None, None, None, None];
        assert_eq!(
            mgr.craft_3x3(&slab_grid),
            Some(ItemStack::new(blocks::STEP as i16, 3, 0))
        );
        set_crafting_mechanics(false, false, false, false);
    }
}
