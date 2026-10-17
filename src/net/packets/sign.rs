use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

pub const SIGN_MAX_LINE_LENGTH: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSign {
    pub x: i32,
    pub y: i16,
    pub z: i32,
    pub lines: [String; 4],
}

impl UpdateSign {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let x = reader.read_i32()?;
        let y = reader.read_i16()?;
        let z = reader.read_i32()?;
        let mut lines = [String::new(), String::new(), String::new(), String::new()];
        for line in &mut lines {
            *line = reader.read_string(SIGN_MAX_LINE_LENGTH)?;
        }
        Ok(Self { x, y, z, lines })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_i16(self.y);
        writer.write_i32(self.z);
        for line in &self.lines {
            writer.write_string(line, SIGN_MAX_LINE_LENGTH)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_sign_round_trips() {
        let packet = UpdateSign {
            x: 10,
            y: 64,
            z: -20,
            lines: [
                "Hello".to_string(),
                "World".to_string(),
                "".to_string(),
                "Beta 1.7.3".to_string(),
            ],
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(UpdateSign::decode(&mut reader).unwrap(), packet);
    }
}
