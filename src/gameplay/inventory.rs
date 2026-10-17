use crate::gameplay::items::ItemStack;
use crate::net::packets::SlotItem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryPlayer {
    pub items: [Option<ItemStack>; 36],
    pub armor: [Option<ItemStack>; 4],
    pub cursor: Option<ItemStack>,
    pub craft_grid: [Option<ItemStack>; 4],
    pub craft_result: Option<ItemStack>,
    pub held_slot: usize,
}

impl Default for InventoryPlayer {
    fn default() -> Self {
        Self {
            items: [None; 36],
            armor: [None; 4],
            cursor: None,
            craft_grid: [None; 4],
            craft_result: None,
            held_slot: 0,
        }
    }
}

impl InventoryPlayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn item_in_hand(&self) -> Option<ItemStack> {
        self.items[self.held_slot.min(8)]
    }

    pub fn set_item_in_hand(&mut self, item: Option<ItemStack>) {
        self.items[self.held_slot.min(8)] = item;
    }

    pub fn get_equipment(&self, slot: i16) -> Option<ItemStack> {
        match slot {
            0 => self.item_in_hand(),
            1 => self.armor[0], // Boots
            2 => self.armor[1], // Leggings
            3 => self.armor[2], // Chestplate
            4 => self.armor[3], // Helmet
            _ => None,
        }
    }

    pub fn total_armor_points(&self) -> i32 {
        let mut total = 0;
        for stack in self.armor.iter().flatten() {
            total += stack.armor_defense();
        }
        total
    }

    pub fn damage_armor(&mut self, damage: i16) {
        let piece_damage = (damage / 4).max(1);
        for piece in &mut self.armor {
            if let Some(mut stack) = *piece {
                stack.damage += piece_damage;
                if stack.damage >= stack.max_damage() {
                    *piece = None;
                } else {
                    *piece = Some(stack);
                }
            }
        }
    }

    pub fn damage_held_item(&mut self, amount: i16) -> bool {
        let slot = self.held_slot.min(8);
        if let Some(mut stack) = self.items[slot] {
            if stack.max_damage() > 0 {
                stack.damage += amount;
                if stack.damage >= stack.max_damage() {
                    self.items[slot] = None;
                    return true;
                }
                self.items[slot] = Some(stack);
            }
        }
        false
    }

    pub fn add_item(&mut self, mut item: ItemStack) -> bool {
        let max_stack = item.max_stack_size();
        if max_stack > 1 {
            for existing in self.items.iter_mut().flatten() {
                if existing.id == item.id
                    && existing.damage == item.damage
                    && existing.count < max_stack
                {
                    let space = max_stack - existing.count;
                    let take = item.count.min(space);
                    existing.count += take;
                    item.count -= take;
                    if item.count == 0 {
                        return true;
                    }
                }
            }
        }

        for slot in &mut self.items {
            if slot.is_none() {
                *slot = Some(item);
                return true;
            }
        }
        false
    }

    pub fn to_slot_item(item: Option<ItemStack>) -> Option<SlotItem> {
        item.map(|s| SlotItem::new(s.id, s.count as i8, s.damage))
    }

    pub fn to_window_items(&self) -> Vec<Option<SlotItem>> {
        let mut window = Vec::with_capacity(45);
        window.push(Self::to_slot_item(self.craft_result));
        for &craft in &self.craft_grid {
            window.push(Self::to_slot_item(craft));
        }
        window.push(Self::to_slot_item(self.armor[3])); // 5: Helmet
        window.push(Self::to_slot_item(self.armor[2])); // 6: Chestplate
        window.push(Self::to_slot_item(self.armor[1])); // 7: Leggings
        window.push(Self::to_slot_item(self.armor[0])); // 8: Boots
        for i in 9..36 {
            window.push(Self::to_slot_item(self.items[i]));
        }
        for i in 0..9 {
            window.push(Self::to_slot_item(self.items[i]));
        }
        window
    }

    pub fn get_window_slot(&self, slot: i16) -> Option<ItemStack> {
        match slot {
            0 => self.craft_result,
            1..=4 => self.craft_grid[(slot - 1) as usize],
            5 => self.armor[3],
            6 => self.armor[2],
            7 => self.armor[1],
            8 => self.armor[0],
            9..=35 => self.items[slot as usize],
            36..=44 => self.items[(slot - 36) as usize],
            _ => None,
        }
    }

    pub fn set_window_slot(&mut self, slot: i16, item: Option<ItemStack>) {
        match slot {
            0 => self.craft_result = item,
            1..=4 => self.craft_grid[(slot - 1) as usize] = item,
            5 => self.armor[3] = item,
            6 => self.armor[2] = item,
            7 => self.armor[1] = item,
            8 => self.armor[0] = item,
            9..=35 => self.items[slot as usize] = item,
            36..=44 => self.items[(slot - 36) as usize] = item,
            _ => {}
        }
    }

    pub fn handle_window_click(&mut self, slot: i16, button: u8, _shift: bool) {
        if slot == -999 {
            // Click outside drops cursor item (drop half if right click)
            if let Some(cursor) = self.cursor {
                if button == 1 && cursor.count > 1 {
                    self.cursor = Some(ItemStack::new(cursor.id, cursor.count - 1, cursor.damage));
                } else {
                    self.cursor = None;
                }
            }
            return;
        }

        if !(0..=44).contains(&slot) {
            return;
        }

        let clicked = self.get_window_slot(slot);
        let cursor = self.cursor;

        match (clicked, cursor) {
            (Some(c), None) => {
                if button == 0 {
                    // Left click with empty hand picks up whole stack
                    self.cursor = Some(c);
                    self.set_window_slot(slot, None);
                } else if button == 1 {
                    // Right click with empty hand splits stack
                    let half = c.count.div_ceil(2);
                    let rest = c.count - half;
                    self.cursor = Some(ItemStack::new(c.id, half, c.damage));
                    self.set_window_slot(
                        slot,
                        if rest > 0 {
                            Some(ItemStack::new(c.id, rest, c.damage))
                        } else {
                            None
                        },
                    );
                }
            }
            (None, Some(cur)) => {
                if button == 0 {
                    // Left click places whole stack
                    self.set_window_slot(slot, Some(cur));
                    self.cursor = None;
                } else if button == 1 {
                    // Right click places one item
                    self.set_window_slot(slot, Some(ItemStack::new(cur.id, 1, cur.damage)));
                    if cur.count > 1 {
                        self.cursor = Some(ItemStack::new(cur.id, cur.count - 1, cur.damage));
                    } else {
                        self.cursor = None;
                    }
                }
            }
            (Some(c), Some(cur)) => {
                if c.id == cur.id && c.damage == cur.damage && cur.max_stack_size() > 1 {
                    let max_stack = cur.max_stack_size();
                    if button == 0 {
                        let space = max_stack.saturating_sub(c.count);
                        let transfer = cur.count.min(space);
                        self.set_window_slot(
                            slot,
                            Some(ItemStack::new(c.id, c.count + transfer, c.damage)),
                        );
                        let rem = cur.count - transfer;
                        self.cursor = if rem > 0 {
                            Some(ItemStack::new(cur.id, rem, cur.damage))
                        } else {
                            None
                        };
                    } else if button == 1 && c.count < max_stack {
                        self.set_window_slot(
                            slot,
                            Some(ItemStack::new(c.id, c.count + 1, c.damage)),
                        );
                        if cur.count > 1 {
                            self.cursor = Some(ItemStack::new(cur.id, cur.count - 1, cur.damage));
                        } else {
                            self.cursor = None;
                        }
                    }
                } else if button == 0 {
                    // Swap items
                    self.set_window_slot(slot, Some(cur));
                    self.cursor = Some(c);
                }
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::items::*;

    #[test]
    fn inventory_add_and_stack() {
        let mut inv = InventoryPlayer::new();
        assert!(inv.add_item(ItemStack::new(1, 32, 0)));
        assert_eq!(inv.items[0], Some(ItemStack::new(1, 32, 0)));
        assert!(inv.add_item(ItemStack::new(1, 32, 0)));
        assert_eq!(inv.items[0], Some(ItemStack::new(1, 64, 0)));
    }

    #[test]
    fn inventory_window_slot_mapping() {
        let mut inv = InventoryPlayer::new();
        inv.items[0] = Some(ItemStack::new(DIAMOND_SWORD, 1, 0));
        assert_eq!(
            inv.get_window_slot(36),
            Some(ItemStack::new(DIAMOND_SWORD, 1, 0))
        );
        inv.set_window_slot(36, None);
        assert_eq!(inv.items[0], None);
    }

    #[test]
    fn inventory_click_swap() {
        let mut inv = InventoryPlayer::new();
        inv.items[0] = Some(ItemStack::new(IRON_PICKAXE, 1, 0));
        inv.handle_window_click(36, 0, false);
        assert_eq!(inv.items[0], None);
        assert_eq!(inv.cursor, Some(ItemStack::new(IRON_PICKAXE, 1, 0)));
        inv.handle_window_click(36, 0, false);
        assert_eq!(inv.items[0], Some(ItemStack::new(IRON_PICKAXE, 1, 0)));
        assert_eq!(inv.cursor, None);
    }
}
