use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseEntity {
    pub user: i32,
    pub target: i32,
    pub attack: bool,
}

impl UseEntity {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            user: reader.read_i32()?,
            target: reader.read_i32()?,
            attack: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.user);
        writer.write_i32(self.target);
        writer.write_bool(self.attack);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respawn {
    pub dimension: i8,
}

impl Respawn {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            dimension: reader.read_i8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i8(self.dimension);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDig {
    pub status: u8,
    pub x: i32,
    pub y: u8,
    pub z: i32,
    pub face: u8,
}

impl BlockDig {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            status: reader.read_u8()?,
            x: reader.read_i32()?,
            y: reader.read_u8()?,
            z: reader.read_i32()?,
            face: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.status);
        writer.write_i32(self.x);
        writer.write_u8(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.face);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub x: i32,
    pub y: u8,
    pub z: i32,
    pub direction: u8,
    pub item_id: i16,
    pub count: Option<u8>,
    pub damage: Option<i16>,
}

impl Place {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let x = reader.read_i32()?;
        let y = reader.read_u8()?;
        let z = reader.read_i32()?;
        let direction = reader.read_u8()?;
        let item_id = reader.read_i16()?;
        let (count, damage) = if item_id >= 0 {
            (Some(reader.read_u8()?), Some(reader.read_i16()?))
        } else {
            (None, None)
        };
        Ok(Self {
            x,
            y,
            z,
            direction,
            item_id,
            count,
            damage,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_u8(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.direction);
        writer.write_i16(self.item_id);
        if self.item_id >= 0 {
            writer.write_u8(self.count.unwrap_or(1));
            writer.write_i16(self.damage.unwrap_or(0));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockItemSwitch {
    pub slot: i16,
}

impl BlockItemSwitch {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            slot: reader.read_i16()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i16(self.slot);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmAnimation {
    pub entity_id: i32,
    pub animate: u8,
}

impl ArmAnimation {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            animate: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.animate);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityAction {
    pub entity_id: i32,
    pub action: u8,
}

impl EntityAction {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            action: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.action);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockChange {
    pub x: i32,
    pub y: u8,
    pub z: i32,
    pub block_type: u8,
    pub metadata: u8,
}

impl BlockChange {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.read_i32()?,
            y: reader.read_u8()?,
            z: reader.read_i32()?,
            block_type: reader.read_u8()?,
            metadata: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_u8(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.block_type);
        writer.write_u8(self.metadata);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_animation_round_trips() {
        let packet = ArmAnimation {
            entity_id: 1,
            animate: 1,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 5);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(ArmAnimation::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn block_dig_round_trips() {
        let packet = BlockDig {
            status: 2,
            x: 10,
            y: 64,
            z: -5,
            face: 1,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 11);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(BlockDig::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn place_with_and_without_item_round_trips() {
        let with_item = Place {
            x: 0,
            y: 65,
            z: 0,
            direction: 1,
            item_id: 1,
            count: Some(64),
            damage: Some(0),
        };
        let mut writer = Writer::new();
        with_item.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 15);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(Place::decode(&mut reader).unwrap(), with_item);

        let without_item = Place {
            x: 0,
            y: 65,
            z: 0,
            direction: 255,
            item_id: -1,
            count: None,
            damage: None,
        };
        let mut writer2 = Writer::new();
        without_item.encode(&mut writer2).unwrap();
        assert_eq!(writer2.len(), 12);
        let mut reader2 = Reader::new(writer2.as_slice());
        assert_eq!(Place::decode(&mut reader2).unwrap(), without_item);
    }

    #[test]
    fn block_change_round_trips() {
        let packet = BlockChange {
            x: 100,
            y: 64,
            z: -200,
            block_type: 2,
            metadata: 0,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.len(), 11);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(BlockChange::decode(&mut reader).unwrap(), packet);
    }
}
