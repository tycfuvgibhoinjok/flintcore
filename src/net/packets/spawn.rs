use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPosition {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl SpawnPosition {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.read_i32()?,
            y: reader.read_i32()?,
            z: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateTime {
    pub time: i64,
}

impl UpdateTime {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            time: reader.read_i64()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i64(self.time);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_position_round_trips() {
        let packet = SpawnPosition { x: 1, y: 64, z: -3 };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.as_slice().len(), 12);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(SpawnPosition::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn update_time_round_trips() {
        let packet = UpdateTime { time: 60_000 };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(UpdateTime::decode(&mut reader).unwrap(), packet);
    }
}
