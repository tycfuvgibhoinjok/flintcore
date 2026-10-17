use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

pub const MAX_DISCONNECT_LENGTH: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KickDisconnect {
    pub reason: String,
}

impl KickDisconnect {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            reason: reader.read_string(MAX_DISCONNECT_LENGTH)?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_string(&self.reason, MAX_DISCONNECT_LENGTH)
    }
}
