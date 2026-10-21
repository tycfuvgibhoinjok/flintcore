use crate::util::bytes::{Reader, Writer};
use crate::util::compress::{deflate_zlib, inflate_zlib, CHUNK_DEFLATE_LEVEL};
use crate::util::Result;
use crate::world::chunk::{Chunk, CHUNK_HEIGHT, CHUNK_WIDTH};

pub const DEFAULT_SIZE_X: u8 = 16;
pub const DEFAULT_SIZE_Y: u8 = 128;
pub const DEFAULT_SIZE_Z: u8 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreChunk {
    pub x: i32,
    pub z: i32,
    pub mode: bool,
}

impl PreChunk {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.read_i32()?,
            z: reader.read_i32()?,
            mode: reader.read_bool()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_i32(self.z);
        writer.write_bool(self.mode);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapChunk {
    pub x: i32,
    pub y: i16,
    pub z: i32,
    pub size_x: u8,
    pub size_y: u8,
    pub size_z: u8,
    pub compressed: Vec<u8>,
}

thread_local! {
    static CHUNK_PAYLOAD_BUF: std::cell::RefCell<Vec<u8>> =
        std::cell::RefCell::new(Vec::with_capacity(crate::world::chunk::PAYLOAD_LEN));
}

impl MapChunk {
    /// Java `Packet51MapChunk(int, int, int, int, int, int, World)`: the
    /// packet of a region of the chunk.
    pub fn from_region(
        chunk: &Chunk,
        x: i32,
        y: i16,
        z: i32,
        size_x: u8,
        size_y: u8,
        size_z: u8,
    ) -> Result<Self> {
        let compressed = CHUNK_PAYLOAD_BUF.with(|cell| {
            let mut payload = cell.borrow_mut();
            payload.clear();
            // The region inside the chunk (the coordinates of the packet are
            // the world ones).
            let x1 = (x - chunk.pos.x * 16).max(0) as usize;
            let z1 = (z - chunk.pos.z * 16).max(0) as usize;
            let y1 = (y.max(0)) as usize;
            let x2 = (x1 + size_x as usize).min(CHUNK_WIDTH);
            let z2 = (z1 + size_z as usize).min(CHUNK_WIDTH);
            let y2 = (y1 + size_y as usize).min(CHUNK_HEIGHT);
            chunk.write_region_payload(x1, y1, z1, x2, y2, z2, &mut payload);
            deflate_zlib(&payload, CHUNK_DEFLATE_LEVEL)
        })?;
        Ok(Self {
            x,
            y,
            z,
            size_x,
            size_y,
            size_z,
            compressed,
        })
    }

    pub fn from_chunk(chunk: &Chunk) -> Result<Self> {
        let compressed = CHUNK_PAYLOAD_BUF.with(|cell| {
            let mut payload = cell.borrow_mut();
            chunk.write_payload(&mut payload);
            deflate_zlib(&payload, CHUNK_DEFLATE_LEVEL)
        })?;
        Ok(Self {
            x: chunk.pos.x * 16,
            y: 0,
            z: chunk.pos.z * 16,
            size_x: DEFAULT_SIZE_X,
            size_y: DEFAULT_SIZE_Y,
            size_z: DEFAULT_SIZE_Z,
            compressed,
        })
    }

    pub fn payload_len(&self) -> usize {
        self.size_x as usize * self.size_y as usize * self.size_z as usize * 5 / 2
    }

    pub fn inflate(&self) -> Result<Vec<u8>> {
        inflate_zlib(&self.compressed, self.payload_len())
    }

    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let x = reader.read_i32()?;
        let y = reader.read_i16()?;
        let z = reader.read_i32()?;
        let size_x = reader.read_u8()?.wrapping_add(1);
        let size_y = reader.read_u8()?.wrapping_add(1);
        let size_z = reader.read_u8()?.wrapping_add(1);
        let length = reader.read_i32()?;
        if length < 0 {
            return Err(crate::util::Error::Malformed("negative chunk length"));
        }
        let compressed = reader.read_bytes(length as usize)?.to_vec();
        Ok(Self {
            x,
            y,
            z,
            size_x,
            size_y,
            size_z,
            compressed,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.x);
        writer.write_i16(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.size_x.wrapping_sub(1));
        writer.write_u8(self.size_y.wrapping_sub(1));
        writer.write_u8(self.size_z.wrapping_sub(1));
        writer.write_i32(self.compressed.len() as i32);
        writer.write_bytes(&self.compressed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::blocks;
    use crate::world::chunk::{ChunkPos, CHUNK_VOLUME, PAYLOAD_LEN};

    #[test]
    fn pre_chunk_round_trips() {
        let packet = PreChunk {
            x: -3,
            z: 7,
            mode: true,
        };
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        assert_eq!(writer.as_slice().len(), 9);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(PreChunk::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn map_chunk_round_trips_and_inflates() {
        let mut chunk = Chunk::new(ChunkPos::new(2, -5));
        chunk.set_block(0, 0, 0, blocks::STONE);
        chunk.set_block(15, 127, 15, blocks::GRASS);
        chunk.init_lighting(false);

        let packet = MapChunk::from_chunk(&chunk).unwrap();
        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();

        let header_and_len = 17 + packet.compressed.len();
        assert_eq!(writer.as_slice().len(), header_and_len);

        let mut reader = Reader::new(writer.as_slice());
        let decoded = MapChunk::decode(&mut reader).unwrap();
        assert_eq!(decoded, packet);

        let payload = decoded.inflate().unwrap();
        assert_eq!(payload.len(), PAYLOAD_LEN);
        assert_eq!(payload.len(), CHUNK_VOLUME + 3 * (CHUNK_VOLUME / 2));
        assert_eq!(payload[0], blocks::STONE);
        assert_eq!(payload[CHUNK_VOLUME - 1], blocks::GRASS);
    }
}
