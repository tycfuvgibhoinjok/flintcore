use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

// Wire order is x, y, stance, z; `y` is the eye height and `stance` the feet.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLookMove {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub stance: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl PlayerLookMove {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let x = reader.read_f64()?;
        let y = reader.read_f64()?;
        let stance = reader.read_f64()?;
        let z = reader.read_f64()?;
        Ok(Self {
            x,
            y,
            z,
            stance,
            yaw: reader.read_f32()?,
            pitch: reader.read_f32()?,
            on_ground: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.stance);
        writer.write_f64(self.z);
        writer.write_f32(self.yaw);
        writer.write_f32(self.pitch);
        writer.write_bool(self.on_ground);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flying {
    pub on_ground: bool,
}

impl Flying {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            on_ground: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_bool(self.on_ground);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub stance: f64,
    pub on_ground: bool,
}

impl PlayerPosition {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.read_f64()?,
            y: reader.read_f64()?,
            stance: reader.read_f64()?,
            z: reader.read_f64()?,
            on_ground: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.stance);
        writer.write_f64(self.z);
        writer.write_bool(self.on_ground);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLook {
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl PlayerLook {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            yaw: reader.read_f32()?,
            pitch: reader.read_f32()?,
            on_ground: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_f32(self.yaw);
        writer.write_f32(self.pitch);
        writer.write_bool(self.on_ground);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_look_move_round_trips() {
        let packet = PlayerLookMove {
            x: 0.5,
            y: 65.62,
            z: -12.25,
            stance: 64.0,
            yaw: 90.0,
            pitch: -15.5,
            on_ground: false,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.as_slice().len(), 41);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(PlayerLookMove::decode(&mut reader).unwrap(), packet);
    }
}
