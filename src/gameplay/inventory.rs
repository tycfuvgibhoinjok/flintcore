use crate::gameplay::blocks;
use crate::gameplay::crafting::CraftingManager;
use crate::gameplay::items::{self, ItemStack};
use crate::net::packets::SlotItem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenContainer {
    Workbench {
        window_id: u8,
        x: i32,
        y: i32,
        z: i32,
        craft_grid: [Option<ItemStack>; 9],
        craft_result: Option<ItemStack>,
    },
    Chest {
        window_id: u8,
        positions: Vec<(i32, i32, i32)>,
        slots: Vec<Option<ItemStack>>,
    },
    Furnace {
        window_id: u8,
        x: i32,
        y: i32,
        z: i32,
        slots: [Option<ItemStack>; 3],
        cook_time: i16,
        burn_time: i16,
        ticks_for_current_fuel: i16,
    },
    Dispenser {
        window_id: u8,
        x: i32,
        y: i32,
        z: i32,
        slots: [Option<ItemStack>; 9],
    },
    MinecartChest {
        window_id: u8,
        entity_id: i32,
        slots: [Option<ItemStack>; 27],
    },
}

impl OpenContainer {
    pub fn window_id(&self) -> u8 {
        match self {
            Self::Workbench { window_id, .. }
            | Self::Chest { window_id, .. }
            | Self::Furnace { window_id, .. }
            | Self::Dispenser { window_id, .. }
            | Self::MinecartChest { window_id, .. } => *window_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickOutcome {
    /// Item that resided in the clicked slot BEFORE the click (`Container.a` return value),
    /// used to verify `Packet102WindowClick.e` for `Packet106Transaction`.
    pub clicked_before: Option<ItemStack>,
    /// Items dropped into the world (e.g., clicking outside window `-999`).
    pub dropped: Vec<ItemStack>,
}

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

    pub fn split_held_item(&mut self, count: u8) -> Option<ItemStack> {
        let slot = self.held_slot.min(8);
        let mut stack = self.items[slot]?;
        if stack.count <= count {
            self.items[slot] = None;
            Some(stack)
        } else {
            stack.count -= count;
            self.items[slot] = Some(stack);
            Some(ItemStack::new(stack.id, count, stack.damage))
        }
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
        let mut i = 0i32;
        let mut j = 0i32;
        let mut k = 0i32;
        for stack in self.armor.iter().flatten() {
            if stack.is_armor() {
                let max_dmg = stack.max_damage() as i32;
                let cur_dmg = stack.damage as i32;
                let rem = max_dmg - cur_dmg;
                j += rem;
                k += max_dmg;
                i += stack.armor_defense();
            }
        }
        if k == 0 {
            0
        } else {
            (i - 1) * j / k + 1
        }
    }

    pub fn damage_armor(&mut self, damage: i16) {
        let piece_damage = damage.max(1);
        for piece in &mut self.armor {
            if let Some(mut stack) = *piece {
                if stack.is_armor() {
                    if stack.damage_item(piece_damage) {
                        *piece = None;
                    } else {
                        *piece = Some(stack);
                    }
                }
            }
        }
    }

    pub fn damage_held_item(&mut self, amount: i16) -> bool {
        let slot = self.held_slot.min(8);
        if let Some(mut stack) = self.items[slot] {
            if stack.damage_item(amount) {
                self.items[slot] = None;
                return true;
            }
            self.items[slot] = Some(stack);
        }
        false
    }

    pub fn consume_item(&mut self, item_id: i16) -> bool {
        for slot in &mut self.items {
            if let Some(mut stack) = *slot {
                if stack.id == item_id && stack.count > 0 {
                    stack.count -= 1;
                    *slot = if stack.count > 0 { Some(stack) } else { None };
                    return true;
                }
            }
        }
        false
    }

    pub fn damage_held_on_block_break(&mut self, block_id: u8) -> bool {
        let slot = self.held_slot.min(8);
        if let Some(mut stack) = self.items[slot] {
            if stack.on_block_destroyed(block_id) {
                self.items[slot] = None;
                return true;
            }
            self.items[slot] = Some(stack);
        }
        false
    }

    /// Java `InventoryPlayer.canHold(ItemStack)` — how many items from `item` can fit.
    pub fn can_hold(&self, item: ItemStack) -> u8 {
        let mut remains = item.count as i32;
        let max_stack = item.max_stack_size() as i32;
        for slot in &self.items {
            match slot {
                None => return item.count,
                Some(existing) => {
                    if existing.id == item.id
                        && existing.is_stackable()
                        && (existing.count as i32) < max_stack
                        && (!existing.uses_data() || existing.damage == item.damage)
                    {
                        remains -= max_stack - existing.count as i32;
                    }
                }
            }
            if remains <= 0 {
                return item.count;
            }
        }
        (item.count as i32 - remains.max(0)) as u8
    }

    /// Java `InventoryPlayer.pickup(ItemStack)` — mutates `item.count` in place and returns true if any were picked up.
    pub fn pickup(&mut self, item: &mut ItemStack) -> bool {
        if item.count == 0 {
            return false;
        }
        if item.is_damageable() && item.damage > 0 {
            for slot in &mut self.items {
                if slot.is_none() {
                    *slot = Some(*item);
                    item.count = 0;
                    return true;
                }
            }
            return false;
        }

        let orig = item.count;
        let max_stack = item.max_stack_size();
        if max_stack > 1 {
            for existing in self.items.iter_mut().flatten() {
                if existing.id == item.id
                    && (!existing.uses_data() || existing.damage == item.damage)
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

        while item.count > 0 {
            let Some(empty_slot) = self.items.iter_mut().find(|s| s.is_none()) else {
                break;
            };
            let take = item.count.min(max_stack);
            *empty_slot = Some(ItemStack::new(item.id, take, item.damage));
            item.count -= take;
        }

        item.count < orig
    }

    pub fn add_item(&mut self, mut item: ItemStack) -> bool {
        self.pickup(&mut item) && item.count == 0
    }

    pub fn update_crafting_result(&mut self) {
        self.craft_result = CraftingManager::instance().craft_2x2(&self.craft_grid);
    }

    fn consume_craft_grid_2x2(&mut self) {
        for slot in &mut self.craft_grid {
            if let Some(mut stack) = *slot {
                let old_id = stack.id;
                stack.count = stack.count.saturating_sub(1);
                if let Some(container_id) = items::crafting_container_item(old_id) {
                    *slot = Some(ItemStack::new(container_id, 1, 0));
                } else if stack.count == 0 {
                    *slot = None;
                } else {
                    *slot = Some(stack);
                }
            }
        }
        self.update_crafting_result();
    }

    fn consume_craft_grid_3x3(
        craft_grid: &mut [Option<ItemStack>; 9],
        craft_result: &mut Option<ItemStack>,
    ) {
        for slot in craft_grid.iter_mut() {
            if let Some(mut stack) = *slot {
                let old_id = stack.id;
                stack.count = stack.count.saturating_sub(1);
                if let Some(container_id) = items::crafting_container_item(old_id) {
                    *slot = Some(ItemStack::new(container_id, 1, 0));
                } else if stack.count == 0 {
                    *slot = None;
                } else {
                    *slot = Some(stack);
                }
            }
        }
        *craft_result = CraftingManager::instance().craft_3x3(craft_grid);
    }

    /// Take all items from cursor + 2x2 craft grid on close/disconnect (`ContainerPlayer.a(EntityHuman)`).
    pub fn drain_transient_items(&mut self) -> Vec<ItemStack> {
        let mut dropped = Vec::new();
        if let Some(cursor) = self.cursor.take() {
            dropped.push(cursor);
        }
        for slot in &mut self.craft_grid {
            if let Some(item) = slot.take() {
                dropped.push(item);
            }
        }
        self.craft_result = None;
        dropped
    }

    /// Drop all inventory and armor items on death (`InventoryPlayer.h()`).
    pub fn drop_all_on_death(&mut self) -> Vec<ItemStack> {
        let mut dropped = self.drain_transient_items();
        for slot in &mut self.items {
            if let Some(item) = slot.take() {
                dropped.push(item);
            }
        }
        for slot in &mut self.armor {
            if let Some(item) = slot.take() {
                dropped.push(item);
            }
        }
        dropped
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

    pub fn container_window_items(&self, container: &OpenContainer) -> Vec<Option<SlotItem>> {
        let mut window = Vec::new();
        match container {
            OpenContainer::Workbench {
                craft_grid,
                craft_result,
                ..
            } => {
                window.push(Self::to_slot_item(*craft_result));
                for &s in craft_grid {
                    window.push(Self::to_slot_item(s));
                }
            }
            OpenContainer::Chest { slots, .. } => {
                for &s in slots {
                    window.push(Self::to_slot_item(s));
                }
            }
            OpenContainer::Furnace { slots, .. } => {
                for &s in slots {
                    window.push(Self::to_slot_item(s));
                }
            }
            OpenContainer::Dispenser { slots, .. } => {
                for &s in slots {
                    window.push(Self::to_slot_item(s));
                }
            }
            OpenContainer::MinecartChest { slots, .. } => {
                for &s in slots {
                    window.push(Self::to_slot_item(s));
                }
            }
        }
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
            1..=4 => {
                self.craft_grid[(slot - 1) as usize] = item;
                self.update_crafting_result();
            }
            5 => self.armor[3] = item,
            6 => self.armor[2] = item,
            7 => self.armor[1] = item,
            8 => self.armor[0] = item,
            9..=35 => self.items[slot as usize] = item,
            36..=44 => self.items[(slot - 36) as usize] = item,
            _ => {}
        }
    }

    fn slot_allows_player_window(slot: i16, item: ItemStack) -> bool {
        match slot {
            0 => false,
            5 => item.armor_slot() == Some(3) || item.id == blocks::PUMPKIN as i16,
            6 => item.armor_slot() == Some(2),
            7 => item.armor_slot() == Some(1),
            8 => item.armor_slot() == Some(0),
            _ => true,
        }
    }

    fn slot_max_stack_player_window(slot: i16, item: ItemStack) -> u8 {
        if (5..=8).contains(&slot) {
            1
        } else {
            item.max_stack_size()
        }
    }

    /// Merge `stack` into player window slots `[start..end)` (forward or reverse),
    /// matching `Container.a(ItemStack, int start, int end, boolean reverse)`.
    fn merge_into_player_slots(&mut self, stack: &mut ItemStack, start: i16, end: i16, rev: bool) {
        if stack.is_stackable() {
            let mut k = if rev { end - 1 } else { start };
            while stack.count > 0 && ((!rev && k < end) || (rev && k >= start)) {
                if let Some(mut existing) = self.get_window_slot(k) {
                    if existing.id == stack.id
                        && (!stack.uses_data() || existing.damage == stack.damage)
                    {
                        let max_s = existing.max_stack_size();
                        let combined = existing.count as u16 + stack.count as u16;
                        if combined <= max_s as u16 {
                            stack.count = 0;
                            existing.count = combined as u8;
                            self.set_window_slot(k, Some(existing));
                        } else if existing.count < max_s {
                            stack.count -= max_s - existing.count;
                            existing.count = max_s;
                            self.set_window_slot(k, Some(existing));
                        }
                    }
                }
                if rev {
                    k -= 1;
                } else {
                    k += 1;
                }
            }
        }

        if stack.count > 0 {
            let mut k = if rev { end - 1 } else { start };
            while (!rev && k < end) || (rev && k >= start) {
                if self.get_window_slot(k).is_none() && Self::slot_allows_player_window(k, *stack) {
                    let cap = Self::slot_max_stack_player_window(k, *stack);
                    let take = stack.count.min(cap);
                    self.set_window_slot(k, Some(ItemStack::new(stack.id, take, stack.damage)));
                    stack.count -= take;
                    if stack.count == 0 {
                        break;
                    }
                }
                if rev {
                    k -= 1;
                } else {
                    k += 1;
                }
            }
        }
    }

    pub fn handle_window_click(&mut self, slot: i16, button: u8, shift: bool) -> ClickOutcome {
        let mut outcome = ClickOutcome {
            clicked_before: None,
            dropped: Vec::new(),
        };

        if button != 0 && button != 1 {
            return outcome;
        }

        if slot == -999 {
            if let Some(cursor) = self.cursor {
                if button == 0 {
                    outcome.dropped.push(cursor);
                    self.cursor = None;
                } else if button == 1 {
                    outcome
                        .dropped
                        .push(ItemStack::new(cursor.id, 1, cursor.damage));
                    if cursor.count > 1 {
                        self.cursor =
                            Some(ItemStack::new(cursor.id, cursor.count - 1, cursor.damage));
                    } else {
                        self.cursor = None;
                    }
                }
            }
            return outcome;
        }

        if !(0..=44).contains(&slot) {
            return outcome;
        }

        if shift {
            outcome.clicked_before = self.shift_click_player_window(slot);
            return outcome;
        }

        let clicked = self.get_window_slot(slot);
        outcome.clicked_before = clicked;

        if slot == 0 {
            // Crafting result slot (SlotResult)
            let Some(res) = clicked else {
                return outcome;
            };
            match self.cursor {
                None => {
                    self.cursor = Some(res);
                    self.consume_craft_grid_2x2();
                }
                Some(cur)
                    if cur.id == res.id
                        && (!res.uses_data() || cur.damage == res.damage)
                        && cur.count.saturating_add(res.count) <= cur.max_stack_size() =>
                {
                    self.cursor = Some(ItemStack::new(cur.id, cur.count + res.count, cur.damage));
                    self.consume_craft_grid_2x2();
                }
                _ => {}
            }
            return outcome;
        }

        let cursor = self.cursor;
        match (clicked, cursor) {
            (Some(c), None) => {
                let take = if button == 0 {
                    c.count
                } else {
                    c.count.div_ceil(2)
                };
                let rest = c.count - take;
                self.cursor = Some(ItemStack::new(c.id, take, c.damage));
                self.set_window_slot(
                    slot,
                    if rest > 0 {
                        Some(ItemStack::new(c.id, rest, c.damage))
                    } else {
                        None
                    },
                );
            }
            (None, Some(cur)) => {
                if Self::slot_allows_player_window(slot, cur) {
                    let cap = Self::slot_max_stack_player_window(slot, cur);
                    let take = (if button == 0 { cur.count } else { 1 }).min(cap);
                    self.set_window_slot(slot, Some(ItemStack::new(cur.id, take, cur.damage)));
                    let rem = cur.count - take;
                    self.cursor = if rem > 0 {
                        Some(ItemStack::new(cur.id, rem, cur.damage))
                    } else {
                        None
                    };
                }
            }
            (Some(c), Some(cur)) => {
                if Self::slot_allows_player_window(slot, cur) {
                    let cap = Self::slot_max_stack_player_window(slot, cur);
                    if c.id == cur.id && (!c.uses_data() || c.damage == cur.damage) {
                        let req = if button == 0 { cur.count } else { 1 };
                        let space = cap.saturating_sub(c.count);
                        let transfer = req.min(space);
                        if transfer > 0 {
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
                        }
                    } else if cur.count <= cap {
                        self.set_window_slot(slot, Some(cur));
                        self.cursor = Some(c);
                    }
                }
            }
            (None, None) => {}
        }

        outcome
    }

    fn shift_click_player_window(&mut self, slot: i16) -> Option<ItemStack> {
        let first = self.get_window_slot(slot)?;
        if slot == 0 {
            let mut result_before = None;
            while let Some(mut res) = self.craft_result {
                if result_before.is_none() {
                    result_before = Some(res);
                }
                let orig_count = res.count;
                self.merge_into_player_slots(&mut res, 9, 45, true);
                if res.count == orig_count {
                    break;
                }
                self.consume_craft_grid_2x2();
                if res.count > 0 {
                    // Partial craft fit; stop
                    break;
                }
            }
            return result_before;
        }

        let mut moving = first;
        if (9..36).contains(&slot) {
            self.merge_into_player_slots(&mut moving, 36, 45, false);
        } else if (36..45).contains(&slot) {
            self.merge_into_player_slots(&mut moving, 9, 36, false);
        } else {
            self.merge_into_player_slots(&mut moving, 9, 45, false);
        }

        if moving.count == first.count {
            return None;
        }

        if moving.count == 0 {
            self.set_window_slot(slot, None);
        } else {
            self.set_window_slot(slot, Some(moving));
        }

        Some(first)
    }

    pub fn handle_container_click(
        &mut self,
        container: &mut OpenContainer,
        slot: i16,
        button: u8,
        shift: bool,
    ) -> ClickOutcome {
        let mut outcome = ClickOutcome {
            clicked_before: None,
            dropped: Vec::new(),
        };

        if button != 0 && button != 1 {
            return outcome;
        }

        if slot == -999 {
            if let Some(cursor) = self.cursor {
                if button == 0 {
                    outcome.dropped.push(cursor);
                    self.cursor = None;
                } else if button == 1 {
                    outcome
                        .dropped
                        .push(ItemStack::new(cursor.id, 1, cursor.damage));
                    if cursor.count > 1 {
                        self.cursor =
                            Some(ItemStack::new(cursor.id, cursor.count - 1, cursor.damage));
                    } else {
                        self.cursor = None;
                    }
                }
            }
            return outcome;
        }

        let total_slots = Self::container_total_slots(container);
        if slot < 0 || slot >= total_slots {
            return outcome;
        }

        if shift {
            outcome.clicked_before = self.shift_click_open_container(container, slot);
            return outcome;
        }

        let clicked = self.get_open_container_slot(container, slot);
        outcome.clicked_before = clicked;

        // Check if slot is a result-only slot (Workbench slot 0, Furnace slot 2)
        let is_result_slot = matches!(
            (&*container, slot),
            (OpenContainer::Workbench { .. }, 0) | (OpenContainer::Furnace { .. }, 2)
        );

        if is_result_slot {
            let Some(res) = clicked else {
                return outcome;
            };
            match self.cursor {
                None => {
                    self.cursor = Some(res);
                    if let OpenContainer::Workbench {
                        craft_grid,
                        craft_result,
                        ..
                    } = container
                    {
                        Self::consume_craft_grid_3x3(craft_grid, craft_result);
                    } else {
                        self.set_open_container_slot(container, slot, None);
                    }
                }
                Some(cur)
                    if cur.id == res.id
                        && (!res.uses_data() || cur.damage == res.damage)
                        && cur.count.saturating_add(res.count) <= cur.max_stack_size() =>
                {
                    if let OpenContainer::Workbench {
                        craft_grid,
                        craft_result,
                        ..
                    } = container
                    {
                        self.cursor =
                            Some(ItemStack::new(cur.id, cur.count + res.count, cur.damage));
                        Self::consume_craft_grid_3x3(craft_grid, craft_result);
                    } else {
                        let take = res.count;
                        if cur.count.saturating_add(take) <= cur.max_stack_size() {
                            self.cursor =
                                Some(ItemStack::new(cur.id, cur.count + take, cur.damage));
                            let rem = res.count - take;
                            self.set_open_container_slot(
                                container,
                                slot,
                                if rem > 0 {
                                    Some(ItemStack::new(res.id, rem, res.damage))
                                } else {
                                    None
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
            return outcome;
        }

        let cursor = self.cursor;
        match (clicked, cursor) {
            (Some(c), None) => {
                let take = if button == 0 {
                    c.count
                } else {
                    c.count.div_ceil(2)
                };
                let rest = c.count - take;
                self.cursor = Some(ItemStack::new(c.id, take, c.damage));
                self.set_open_container_slot(
                    container,
                    slot,
                    if rest > 0 {
                        Some(ItemStack::new(c.id, rest, c.damage))
                    } else {
                        None
                    },
                );
            }
            (None, Some(cur)) => {
                let cap = cur.max_stack_size();
                let take = (if button == 0 { cur.count } else { 1 }).min(cap);
                self.set_open_container_slot(
                    container,
                    slot,
                    Some(ItemStack::new(cur.id, take, cur.damage)),
                );
                let rem = cur.count - take;
                self.cursor = if rem > 0 {
                    Some(ItemStack::new(cur.id, rem, cur.damage))
                } else {
                    None
                };
            }
            (Some(c), Some(cur)) => {
                let cap = cur.max_stack_size();
                if c.id == cur.id && (!c.uses_data() || c.damage == cur.damage) {
                    let req = if button == 0 { cur.count } else { 1 };
                    let space = cap.saturating_sub(c.count);
                    let transfer = req.min(space);
                    if transfer > 0 {
                        self.set_open_container_slot(
                            container,
                            slot,
                            Some(ItemStack::new(c.id, c.count + transfer, c.damage)),
                        );
                        let rem = cur.count - transfer;
                        self.cursor = if rem > 0 {
                            Some(ItemStack::new(cur.id, rem, cur.damage))
                        } else {
                            None
                        };
                    }
                } else if cur.count <= cap {
                    self.set_open_container_slot(container, slot, Some(cur));
                    self.cursor = Some(c);
                }
            }
            (None, None) => {}
        }

        outcome
    }

    fn container_header_slots(container: &OpenContainer) -> i16 {
        match container {
            OpenContainer::Workbench { .. } => 10,
            OpenContainer::Chest { slots, .. } => slots.len() as i16,
            OpenContainer::Furnace { .. } => 3,
            OpenContainer::Dispenser { .. } => 9,
            OpenContainer::MinecartChest { .. } => 27,
        }
    }

    fn container_total_slots(container: &OpenContainer) -> i16 {
        Self::container_header_slots(container) + 36
    }

    pub fn get_open_container_slot(
        &self,
        container: &OpenContainer,
        slot: i16,
    ) -> Option<ItemStack> {
        let header = Self::container_header_slots(container);
        if slot < header {
            match container {
                OpenContainer::Workbench {
                    craft_grid,
                    craft_result,
                    ..
                } => {
                    if slot == 0 {
                        *craft_result
                    } else {
                        craft_grid[(slot - 1) as usize]
                    }
                }
                OpenContainer::Chest { slots, .. } => slots[slot as usize],
                OpenContainer::Furnace { slots, .. } => slots[slot as usize],
                OpenContainer::Dispenser { slots, .. } => slots[slot as usize],
                OpenContainer::MinecartChest { slots, .. } => slots[slot as usize],
            }
        } else {
            let rel = slot - header;
            if rel < 27 {
                self.items[(rel + 9) as usize]
            } else if rel < 36 {
                self.items[(rel - 27) as usize]
            } else {
                None
            }
        }
    }

    pub fn set_open_container_slot(
        &mut self,
        container: &mut OpenContainer,
        slot: i16,
        item: Option<ItemStack>,
    ) {
        let header = Self::container_header_slots(container);
        if slot < header {
            match container {
                OpenContainer::Workbench {
                    craft_grid,
                    craft_result,
                    ..
                } => {
                    if slot == 0 {
                        *craft_result = item;
                    } else {
                        craft_grid[(slot - 1) as usize] = item;
                        *craft_result = CraftingManager::instance().craft_3x3(craft_grid);
                    }
                }
                OpenContainer::Chest { slots, .. } => {
                    slots[slot as usize] = item;
                }
                OpenContainer::Furnace { slots, .. } => {
                    slots[slot as usize] = item;
                }
                OpenContainer::Dispenser { slots, .. } => {
                    slots[slot as usize] = item;
                }
                OpenContainer::MinecartChest { slots, .. } => {
                    slots[slot as usize] = item;
                }
            }
        } else {
            let rel = slot - header;
            if rel < 27 {
                self.items[(rel + 9) as usize] = item;
            } else if rel < 36 {
                self.items[(rel - 27) as usize] = item;
            }
        }
    }

    fn merge_into_open_container_slots(
        &mut self,
        container: &mut OpenContainer,
        stack: &mut ItemStack,
        start: i16,
        end: i16,
        rev: bool,
    ) {
        if stack.is_stackable() {
            let mut k = if rev { end - 1 } else { start };
            while stack.count > 0 && ((!rev && k < end) || (rev && k >= start)) {
                if let Some(mut existing) = self.get_open_container_slot(container, k) {
                    if existing.id == stack.id
                        && (!stack.uses_data() || existing.damage == stack.damage)
                    {
                        let max_s = existing.max_stack_size();
                        let combined = existing.count as u16 + stack.count as u16;
                        if combined <= max_s as u16 {
                            stack.count = 0;
                            existing.count = combined as u8;
                            self.set_open_container_slot(container, k, Some(existing));
                        } else if existing.count < max_s {
                            stack.count -= max_s - existing.count;
                            existing.count = max_s;
                            self.set_open_container_slot(container, k, Some(existing));
                        }
                    }
                }
                if rev {
                    k -= 1;
                } else {
                    k += 1;
                }
            }
        }

        if stack.count > 0 {
            let mut k = if rev { end - 1 } else { start };
            while (!rev && k < end) || (rev && k >= start) {
                if self.get_open_container_slot(container, k).is_none() {
                    let cap = stack.max_stack_size();
                    let take = stack.count.min(cap);
                    self.set_open_container_slot(
                        container,
                        k,
                        Some(ItemStack::new(stack.id, take, stack.damage)),
                    );
                    stack.count -= take;
                    if stack.count == 0 {
                        break;
                    }
                }
                if rev {
                    k -= 1;
                } else {
                    k += 1;
                }
            }
        }
    }

    fn shift_click_open_container(
        &mut self,
        container: &mut OpenContainer,
        slot: i16,
    ) -> Option<ItemStack> {
        let first = self.get_open_container_slot(container, slot)?;
        let header = Self::container_header_slots(container);
        let total = Self::container_total_slots(container);

        if matches!(container, OpenContainer::Workbench { .. }) && slot == 0 {
            let mut result_before = None;
            loop {
                let cur_res = match container {
                    OpenContainer::Workbench { craft_result, .. } => *craft_result,
                    _ => None,
                };
                let Some(mut res) = cur_res else {
                    break;
                };
                if result_before.is_none() {
                    result_before = Some(res);
                }
                let orig = res.count;
                self.merge_into_open_container_slots(container, &mut res, 10, 46, true);
                if res.count == orig {
                    break;
                }
                if let OpenContainer::Workbench {
                    craft_grid,
                    craft_result,
                    ..
                } = container
                {
                    Self::consume_craft_grid_3x3(craft_grid, craft_result);
                }
                if res.count > 0 {
                    break;
                }
            }
            return result_before;
        }

        let mut moving = first;
        match container {
            OpenContainer::Workbench { .. } => {
                if (10..37).contains(&slot) {
                    self.merge_into_open_container_slots(container, &mut moving, 37, 46, false);
                } else if (37..46).contains(&slot) {
                    self.merge_into_open_container_slots(container, &mut moving, 10, 37, false);
                } else {
                    self.merge_into_open_container_slots(container, &mut moving, 10, 46, false);
                }
            }
            OpenContainer::Chest { .. }
            | OpenContainer::Dispenser { .. }
            | OpenContainer::MinecartChest { .. } => {
                if slot < header {
                    self.merge_into_open_container_slots(
                        container,
                        &mut moving,
                        header,
                        total,
                        true,
                    );
                } else {
                    self.merge_into_open_container_slots(container, &mut moving, 0, header, false);
                }
            }
            OpenContainer::Furnace { .. } => {
                if slot == 2 {
                    self.merge_into_open_container_slots(container, &mut moving, 3, 39, true);
                } else if (3..30).contains(&slot) {
                    self.merge_into_open_container_slots(container, &mut moving, 30, 39, false);
                } else if (30..39).contains(&slot) {
                    self.merge_into_open_container_slots(container, &mut moving, 3, 30, false);
                } else {
                    self.merge_into_open_container_slots(container, &mut moving, 3, 39, false);
                }
            }
        }

        if moving.count == first.count {
            return None;
        }

        if moving.count == 0 {
            self.set_open_container_slot(container, slot, None);
        } else {
            self.set_open_container_slot(container, slot, Some(moving));
        }

        Some(first)
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

    #[test]
    fn inventory_2x2_crafting_and_ingredient_consumption() {
        let mut inv = InventoryPlayer::new();
        // Place 1 oak log in slot 1 (top-left of 2x2 craft grid)
        inv.set_window_slot(1, Some(ItemStack::new(blocks::LOG as i16, 2, 0)));
        assert_eq!(
            inv.craft_result,
            Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 4, 0))
        );

        // Click slot 0 (craft result)
        inv.handle_window_click(0, 0, false);
        assert_eq!(
            inv.cursor,
            Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 4, 0))
        );
        // 1 log remains in slot 1, so craft_result is still 4 planks
        assert_eq!(
            inv.get_window_slot(1),
            Some(ItemStack::new(blocks::LOG as i16, 1, 0))
        );
        assert_eq!(
            inv.craft_result,
            Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 4, 0))
        );

        // Click slot 0 again: stacks onto cursor (now 8 planks) and empties slot 1
        inv.handle_window_click(0, 0, false);
        assert_eq!(
            inv.cursor,
            Some(ItemStack::new(blocks::WOOD_PLANKS as i16, 8, 0))
        );
        assert_eq!(inv.get_window_slot(1), None);
        assert_eq!(inv.craft_result, None);
    }

    #[test]
    fn workbench_3x3_cake_returns_empty_buckets() {
        let mut inv = InventoryPlayer::new();
        let mut wb = OpenContainer::Workbench {
            window_id: 1,
            x: 0,
            y: 64,
            z: 0,
            craft_grid: [None; 9],
            craft_result: None,
        };
        // Top row: 3 Milk Buckets (slots 1, 2, 3)
        for s in 1..=3 {
            inv.set_open_container_slot(&mut wb, s, Some(ItemStack::new(MILK_BUCKET, 1, 0)));
        }
        // Middle row: Sugar, Egg, Sugar (slots 4, 5, 6)
        inv.set_open_container_slot(&mut wb, 4, Some(ItemStack::new(SUGAR, 1, 0)));
        inv.set_open_container_slot(&mut wb, 5, Some(ItemStack::new(EGG, 1, 0)));
        inv.set_open_container_slot(&mut wb, 6, Some(ItemStack::new(SUGAR, 1, 0)));
        // Bottom row: 3 Wheat (slots 7, 8, 9)
        for s in 7..=9 {
            inv.set_open_container_slot(&mut wb, s, Some(ItemStack::new(WHEAT, 1, 0)));
        }

        assert_eq!(
            inv.get_open_container_slot(&wb, 0),
            Some(ItemStack::new(CAKE, 1, 0))
        );

        // Click slot 0 to craft cake
        inv.handle_container_click(&mut wb, 0, 0, false);
        assert_eq!(inv.cursor, Some(ItemStack::new(CAKE, 1, 0)));

        // Slots 1, 2, 3 must now contain empty BUCKETs!
        for s in 1..=3 {
            assert_eq!(
                inv.get_open_container_slot(&wb, s),
                Some(ItemStack::new(BUCKET, 1, 0))
            );
        }
        for s in 4..=9 {
            assert_eq!(inv.get_open_container_slot(&wb, s), None);
        }
    }
}
