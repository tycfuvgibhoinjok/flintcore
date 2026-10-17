use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

pub const MAX_HANDSHAKE_USERNAME: usize = 32;
pub const MAX_LOGIN_USERNAME: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub username: String,
}

impl Handshake {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            username: reader.read_string(MAX_HANDSHAKE_USERNAME)?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_string(&self.username, MAX_HANDSHAKE_USERNAME)
    }
}

// Field `protocol` carries the protocol version client->server and the assigned
// entity id server->client, matching b1.7.3's reused Packet1Login slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub protocol: i32,
    pub username: String,
    pub map_seed: i64,
    pub dimension: i8,
}

impl Login {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            protocol: reader.read_i32()?,
            username: reader.read_string(MAX_LOGIN_USERNAME)?,
            map_seed: reader.read_i64()?,
            dimension: reader.read_i8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.protocol);
        writer.write_string(&self.username, MAX_LOGIN_USERNAME)?;
        writer.write_i64(self.map_seed);
        writer.write_i8(self.dimension);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_round_trips() {
        let login = Login {
            protocol: 14,
            username: "Player".to_string(),
            map_seed: 1234,
            dimension: 0,
        };
        let mut writer = Writer::new();
        login.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(Login::decode(&mut reader).unwrap(), login);
    }

    #[test]
    fn handshake_round_trips() {
        let handshake = Handshake {
            username: "Player".to_string(),
        };
        let mut writer = Writer::new();
        handshake.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(Handshake::decode(&mut reader).unwrap(), handshake);
    }
}
