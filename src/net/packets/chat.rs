use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

pub const MAX_CHAT_LENGTH: usize = 119;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub message: String,
}

impl Chat {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            message: reader.read_string(MAX_CHAT_LENGTH)?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_string(&self.message, MAX_CHAT_LENGTH)
    }
}
