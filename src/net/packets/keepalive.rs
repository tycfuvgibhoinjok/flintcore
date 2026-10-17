use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeepAlive;

impl KeepAlive {
    pub fn decode(_reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self)
    }

    pub fn encode(&self, _writer: &mut Writer) -> Result<()> {
        Ok(())
    }
}
