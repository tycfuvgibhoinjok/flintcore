use crate::net::packets::Serverbound;
use crate::net::protocol::PacketId;
use crate::util::bytes::Reader;
use crate::util::{Error, Result};

pub fn decode_serverbound(buffer: &[u8]) -> Result<(Serverbound, usize)> {
    let mut reader = Reader::new(buffer);
    let raw_id = reader.read_u8()?;
    let id = PacketId::from_u8(raw_id).ok_or(Error::UnknownPacketId(raw_id))?;
    if !id.serverbound() {
        return Err(Error::UnknownPacketId(raw_id));
    }
    let packet = Serverbound::decode(id, &mut reader)?;
    Ok((packet, reader.position()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::packets::Handshake;
    use crate::util::bytes::Writer;

    #[test]
    fn decodes_handshake() {
        let mut writer = Writer::new();
        writer.write_u8(PacketId::Handshake.id());
        writer.write_string("Player", 32).unwrap();
        let (packet, consumed) = decode_serverbound(writer.as_slice()).unwrap();
        assert_eq!(consumed, writer.len());
        assert_eq!(
            packet,
            Serverbound::Handshake(Handshake {
                username: "Player".to_string()
            })
        );
    }

    #[test]
    fn incomplete_input_is_reported_without_consuming() {
        let mut writer = Writer::new();
        writer.write_u8(PacketId::Handshake.id());
        writer.write_i16(6);
        writer.write_u16(u16::from(b'A'));
        assert!(matches!(
            decode_serverbound(writer.as_slice()),
            Err(Error::Incomplete)
        ));
    }

    #[test]
    fn unknown_id_is_rejected() {
        assert!(matches!(
            decode_serverbound(&[26]),
            Err(Error::UnknownPacketId(26))
        ));
    }

    #[test]
    fn clientbound_only_id_is_rejected_on_serverbound_path() {
        assert!(matches!(
            decode_serverbound(&[PacketId::MapChunk.id()]),
            Err(Error::UnknownPacketId(51))
        ));
    }
}
