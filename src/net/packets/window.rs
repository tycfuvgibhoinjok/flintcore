use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

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
    fn close_window_round_trips() {
        let packet = CloseWindow { window_id: 2 };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 1);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(CloseWindow::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn window_click_round_trips() {
        let packet = WindowClick {
            window_id: 0,
            slot: 36,
            button: 0,
            action_number: 1,
            shift: false,
            item_id: -1,
            count: None,
            damage: None,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 9);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(WindowClick::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn transaction_round_trips() {
        let packet = Transaction {
            window_id: 0,
            action_number: 5,
            accepted: true,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 4);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(Transaction::decode(&mut reader).unwrap(), packet);
    }
}
