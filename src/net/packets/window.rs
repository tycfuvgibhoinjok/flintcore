use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotItem {
    pub item_id: i16,
    pub count: i8,
    pub damage: i16,
}

impl SlotItem {
    pub fn new(item_id: i16, count: i8, damage: i16) -> Self {
        Self {
            item_id,
            count,
            damage,
        }
    }

    pub fn decode(reader: &mut Reader<'_>) -> Result<Option<Self>> {
        let item_id = reader.read_i16()?;
        if item_id >= 0 {
            let count = reader.read_i8()?;
            let damage = reader.read_i16()?;
            Ok(Some(Self {
                item_id,
                count,
                damage,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn encode(item: Option<&Self>, writer: &mut Writer) {
        if let Some(item) = item {
            writer.write_i16(item.item_id);
            writer.write_i8(item.count);
            writer.write_i16(item.damage);
        } else {
            writer.write_i16(-1);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    pub window_id: u8,
    pub inventory_type: u8,
    pub title: String,
    pub slots_count: u8,
}

impl OpenWindow {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            window_id: reader.read_u8()?,
            inventory_type: reader.read_u8()?,
            title: reader.read_modified_utf8(255)?,
            slots_count: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.window_id);
        writer.write_u8(self.inventory_type);
        writer.write_modified_utf8(&self.title)?;
        writer.write_u8(self.slots_count);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSlot {
    pub window_id: i8,
    pub slot: i16,
    pub item: Option<SlotItem>,
}

impl SetSlot {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let window_id = reader.read_i8()?;
        let slot = reader.read_i16()?;
        let item = SlotItem::decode(reader)?;
        Ok(Self {
            window_id,
            slot,
            item,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i8(self.window_id);
        writer.write_i16(self.slot);
        SlotItem::encode(self.item.as_ref(), writer);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowItems {
    pub window_id: u8,
    pub items: Vec<Option<SlotItem>>,
}

impl WindowItems {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let window_id = reader.read_u8()?;
        let count = reader.read_i16()? as usize;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(SlotItem::decode(reader)?);
        }
        Ok(Self { window_id, items })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.window_id);
        writer.write_i16(self.items.len() as i16);
        for item in &self.items {
            SlotItem::encode(item.as_ref(), writer);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftProgressBar {
    pub window_id: u8,
    pub property_id: i16,
    pub value: i16,
}

impl CraftProgressBar {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            window_id: reader.read_u8()?,
            property_id: reader.read_i16()?,
            value: reader.read_i16()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.window_id);
        writer.write_i16(self.property_id);
        writer.write_i16(self.value);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseWindow {
    pub window_id: u8,
}

impl CloseWindow {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            window_id: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.window_id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowClick {
    pub window_id: u8,
    pub slot: i16,
    pub button: u8,
    pub action_number: i16,
    pub shift: bool,
    pub item_id: i16,
    pub count: Option<u8>,
    pub damage: Option<i16>,
}

impl WindowClick {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let window_id = reader.read_u8()?;
        let slot = reader.read_i16()?;
        let button = reader.read_u8()?;
        let action_number = reader.read_i16()?;
        let shift = reader.read_bool()?;
        let item_id = reader.read_i16()?;
        let (count, damage) = if item_id >= 0 {
            (Some(reader.read_u8()?), Some(reader.read_i16()?))
        } else {
            (None, None)
        };
        Ok(Self {
            window_id,
            slot,
            button,
            action_number,
            shift,
            item_id,
            count,
            damage,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.window_id);
        writer.write_i16(self.slot);
        writer.write_u8(self.button);
        writer.write_i16(self.action_number);
        writer.write_bool(self.shift);
        writer.write_i16(self.item_id);
        if self.item_id >= 0 {
            writer.write_u8(self.count.unwrap_or(1));
            writer.write_i16(self.damage.unwrap_or(0));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub window_id: u8,
    pub action_number: i16,
    pub accepted: bool,
}

impl Transaction {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            window_id: reader.read_u8()?,
            action_number: reader.read_i16()?,
            accepted: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.window_id);
        writer.write_i16(self.action_number);
        writer.write_bool(self.accepted);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_slot_round_trips() {
        let packet = SetSlot {
            window_id: 0,
            slot: 36,
            item: Some(SlotItem::new(1, 64, 0)),
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(SetSlot::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn window_items_round_trips() {
        let packet = WindowItems {
            window_id: 0,
            items: vec![Some(SlotItem::new(276, 1, 0)), None],
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(WindowItems::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn open_window_round_trips() {
        let packet = OpenWindow {
            window_id: 1,
            inventory_type: 0,
            title: "Chest".to_string(),
            slots_count: 27,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(OpenWindow::decode(&mut reader).unwrap(), packet);
    }
}
