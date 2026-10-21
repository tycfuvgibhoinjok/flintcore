use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sleep {
    pub entity_id: i32,
    pub in_bed_code: i8,
    pub bed_x: i32,
    pub bed_y: u8,
    pub bed_z: i32,
}

impl Sleep {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            in_bed_code: reader.read_i8()?,
            bed_x: reader.read_i32()?,
            bed_y: reader.read_u8()?,
            bed_z: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i8(self.in_bed_code);
        writer.write_i32(self.bed_x);
        writer.write_u8(self.bed_y);
        writer.write_i32(self.bed_z);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiBlockChange {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub coordinates: Vec<u16>,
    pub block_types: Vec<u8>,
    pub metadata: Vec<u8>,
}

impl MultiBlockChange {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let chunk_x = reader.read_i32()?;
        let chunk_z = reader.read_i32()?;
        let count = reader.read_i16()? as usize;
        let mut coordinates = Vec::with_capacity(count);
        for _ in 0..count {
            // Java `Packet52MultiBlockChange`: the coordinates are read with
            // the mask `& 0xffff`.
            coordinates.push(u16::from_be_bytes([reader.read_u8()?, reader.read_u8()?]));
        }
        let block_types = reader.read_bytes(count)?.to_vec();
        let metadata = reader.read_bytes(count)?.to_vec();
        Ok(Self {
            chunk_x,
            chunk_z,
            coordinates,
            block_types,
            metadata,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.chunk_x);
        writer.write_i32(self.chunk_z);
        writer.write_i16(self.coordinates.len() as i16);
        for coord in &self.coordinates {
            writer.write_u16(*coord);
        }
        writer.write_bytes(&self.block_types);
        writer.write_bytes(&self.metadata);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayNoteBlock {
    pub x: i32,
    pub y: i16,
    pub z: i32,
    pub instrument: u8,
    pub pitch: u8,
}

impl PlayNoteBlock {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.read_i32()?,
            y: reader.read_i16()?,
            z: reader.read_i32()?,
            instrument: reader.read_u8()?,
            pitch: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_i16(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.instrument);
        writer.write_u8(self.pitch);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub radius: f32,
    pub records: Vec<(i8, i8, i8)>,
}

impl Explosion {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let x = reader.read_f64()?;
        let y = reader.read_f64()?;
        let z = reader.read_f64()?;
        let radius = reader.read_f32()?;
        let count = reader.read_i32()? as usize;
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            records.push((reader.read_i8()?, reader.read_i8()?, reader.read_i8()?));
        }
        Ok(Self {
            x,
            y,
            z,
            radius,
            records,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.z);
        writer.write_f32(self.radius);
        writer.write_i32(self.records.len() as i32);
        for (rx, ry, rz) in &self.records {
            writer.write_i8(*rx);
            writer.write_i8(*ry);
            writer.write_i8(*rz);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelSound {
    pub effect_id: i32,
    pub x: i32,
    pub y: u8,
    pub z: i32,
    pub data: i32,
}

impl LevelSound {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            effect_id: reader.read_i32()?,
            x: reader.read_i32()?,
            y: reader.read_u8()?,
            z: reader.read_i32()?,
            data: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.effect_id);
        writer.write_i32(self.x);
        writer.write_u8(self.y);
        writer.write_i32(self.z);
        writer.write_i32(self.data);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bed {
    pub reason: u8,
}

impl Bed {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            reason: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.reason);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weather {
    pub entity_id: i32,
    pub weather_type: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Weather {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            weather_type: reader.read_u8()?,
            x: reader.read_i32()?,
            y: reader.read_i32()?,
            z: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.weather_type);
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapData {
    pub item_type: i16,
    pub item_id: i16,
    pub data: Vec<u8>,
}

impl MapData {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let item_type = reader.read_i16()?;
        let item_id = reader.read_i16()?;
        let len = reader.read_u8()? as usize;
        let data = reader.read_bytes(len)?.to_vec();
        Ok(Self {
            item_type,
            item_id,
            data,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i16(self.item_type);
        writer.write_i16(self.item_id);
        writer.write_u8(self.data.len() as u8);
        writer.write_bytes(&self.data);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statistic {
    pub statistic_id: i32,
    pub amount: u8,
}

impl Statistic {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            statistic_id: reader.read_i32()?,
            amount: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.statistic_id);
        writer.write_u8(self.amount);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_note_block_round_trips() {
        let packet = PlayNoteBlock {
            x: 10,
            y: 64,
            z: -5,
            instrument: 1,
            pitch: 12,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(PlayNoteBlock::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn explosion_round_trips() {
        let packet = Explosion {
            x: 100.5,
            y: 64.0,
            z: -200.25,
            radius: 4.0,
            records: vec![(0, 0, 0), (1, -1, 2)],
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(Explosion::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn level_sound_round_trips() {
        let packet = LevelSound {
            effect_id: 2001,
            x: 10,
            y: 64,
            z: 20,
            data: 1,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(LevelSound::decode(&mut reader).unwrap(), packet);
    }
}
