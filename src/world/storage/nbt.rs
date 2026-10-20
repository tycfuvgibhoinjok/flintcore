use indexmap::IndexMap;

use crate::util::compress::{
    deflate_gzip, deflate_zlib, inflate_gzip, inflate_zlib, CHUNK_DEFLATE_LEVEL,
};
use crate::util::{Error, Result};

pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_BYTE_ARRAY: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;

#[derive(Debug, Clone, PartialEq)]
pub enum NbtTag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<NbtTag>),
    Compound(IndexMap<String, NbtTag>),
    IntArray(Vec<i32>),
}

impl NbtTag {
    pub fn tag_id(&self) -> u8 {
        match self {
            Self::Byte(_) => TAG_BYTE,
            Self::Short(_) => TAG_SHORT,
            Self::Int(_) => TAG_INT,
            Self::Long(_) => TAG_LONG,
            Self::Float(_) => TAG_FLOAT,
            Self::Double(_) => TAG_DOUBLE,
            Self::ByteArray(_) => TAG_BYTE_ARRAY,
            Self::String(_) => TAG_STRING,
            Self::List(_) => TAG_LIST,
            Self::Compound(_) => TAG_COMPOUND,
            Self::IntArray(_) => TAG_INT_ARRAY,
        }
    }

    pub fn compound() -> Self {
        Self::Compound(IndexMap::new())
    }

    pub fn insert(&mut self, key: impl Into<String>, value: NbtTag) {
        if let Self::Compound(map) = self {
            map.insert(key.into(), value);
        }
    }

    pub fn get(&self, key: &str) -> Option<&NbtTag> {
        match self {
            Self::Compound(map) => map.get(key),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut NbtTag> {
        match self {
            Self::Compound(map) => map.get_mut(key),
            _ => None,
        }
    }

    pub fn get_i8(&self, key: &str) -> Option<i8> {
        match self.get(key) {
            Some(Self::Byte(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_i8(key).map(|v| v != 0)
    }

    pub fn get_i16(&self, key: &str) -> Option<i16> {
        match self.get(key) {
            Some(Self::Short(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        match self.get(key) {
            Some(Self::Int(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.get(key) {
            Some(Self::Long(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_f32(&self, key: &str) -> Option<f32> {
        match self.get(key) {
            Some(Self::Float(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        match self.get(key) {
            Some(Self::Double(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Self::String(v)) => Some(v.as_str()),
            _ => None,
        }
    }

    pub fn get_compound(&self, key: &str) -> Option<&IndexMap<String, NbtTag>> {
        match self.get(key) {
            Some(Self::Compound(map)) => Some(map),
            _ => None,
        }
    }

    pub fn get_list(&self, key: &str) -> Option<&Vec<NbtTag>> {
        match self.get(key) {
            Some(Self::List(list)) => Some(list),
            _ => None,
        }
    }

    pub fn get_byte_array(&self, key: &str) -> Option<&[u8]> {
        match self.get(key) {
            Some(Self::ByteArray(bytes)) => Some(bytes.as_slice()),
            _ => None,
        }
    }

    pub fn to_bytes(&self, root_name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.tag_id());
        write_string(root_name, &mut out);
        self.write_payload(&mut out);
        out
    }

    pub fn from_bytes(mut data: &[u8]) -> Result<(String, Self)> {
        if data.is_empty() {
            return Err(Error::Malformed("NBT data is empty"));
        }
        let tag_id = read_u8(&mut data)?;
        if tag_id == TAG_END {
            return Ok((String::new(), Self::Compound(IndexMap::new())));
        }
        let name = read_string(&mut data)?;
        let tag = Self::read_payload(tag_id, &mut data)?;
        Ok((name, tag))
    }

    pub fn to_zlib(&self, root_name: &str) -> Result<Vec<u8>> {
        let raw = self.to_bytes(root_name);
        deflate_zlib(&raw, CHUNK_DEFLATE_LEVEL)
    }

    pub fn from_zlib(compressed: &[u8]) -> Result<(String, Self)> {
        // Assume maximum uncompressed chunk size of 256 KiB
        let decompressed = inflate_zlib(compressed, 256 * 1024)?;
        Self::from_bytes(&decompressed)
    }

    pub fn to_gzip(&self, root_name: &str) -> Result<Vec<u8>> {
        let raw = self.to_bytes(root_name);
        deflate_gzip(&raw, CHUNK_DEFLATE_LEVEL)
    }

    pub fn from_gzip(compressed: &[u8]) -> Result<(String, Self)> {
        // Assume maximum uncompressed size of 2 MiB for level.dat or player dat
        let decompressed = inflate_gzip(compressed, 2 * 1024 * 1024)?;
        Self::from_bytes(&decompressed)
    }

    fn write_payload(&self, out: &mut Vec<u8>) {
        match self {
            Self::Byte(v) => out.push(*v as u8),
            Self::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
            Self::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            Self::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
            Self::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
            Self::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
            Self::ByteArray(bytes) => {
                out.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
                out.extend_from_slice(bytes);
            }
            Self::String(s) => write_string(s, out),
            Self::List(items) => {
                let elem_type = items.first().map_or(TAG_END, |t| t.tag_id());
                out.push(elem_type);
                out.extend_from_slice(&(items.len() as i32).to_be_bytes());
                for item in items {
                    item.write_payload(out);
                }
            }
            Self::Compound(map) => {
                for (name, tag) in map {
                    out.push(tag.tag_id());
                    write_string(name, out);
                    tag.write_payload(out);
                }
                out.push(TAG_END);
            }
            Self::IntArray(ints) => {
                out.extend_from_slice(&(ints.len() as i32).to_be_bytes());
                for &v in ints {
                    out.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
    }

    fn read_payload(tag_id: u8, data: &mut &[u8]) -> Result<Self> {
        match tag_id {
            TAG_BYTE => Ok(Self::Byte(read_u8(data)? as i8)),
            TAG_SHORT => Ok(Self::Short(read_i16(data)?)),
            TAG_INT => Ok(Self::Int(read_i32(data)?)),
            TAG_LONG => Ok(Self::Long(read_i64(data)?)),
            TAG_FLOAT => Ok(Self::Float(f32::from_bits(read_u32(data)?))),
            TAG_DOUBLE => Ok(Self::Double(f64::from_bits(read_u64(data)?))),
            TAG_BYTE_ARRAY => {
                let len = read_i32(data)?;
                if len < 0 {
                    return Err(Error::Malformed("negative byte array length in NBT"));
                }
                let len = len as usize;
                if data.len() < len {
                    return Err(Error::Malformed("unexpected EOF in NBT byte array"));
                }
                let bytes = data[..len].to_vec();
                *data = &data[len..];
                Ok(Self::ByteArray(bytes))
            }
            TAG_STRING => Ok(Self::String(read_string(data)?)),
            TAG_LIST => {
                let elem_type = read_u8(data)?;
                let len = read_i32(data)?;
                if len < 0 {
                    return Err(Error::Malformed("negative list length in NBT"));
                }
                let len = len as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(Self::read_payload(elem_type, data)?);
                }
                Ok(Self::List(items))
            }
            TAG_COMPOUND => {
                let mut map = IndexMap::new();
                loop {
                    let sub_id = read_u8(data)?;
                    if sub_id == TAG_END {
                        break;
                    }
                    let name = read_string(data)?;
                    let sub_tag = Self::read_payload(sub_id, data)?;
                    map.insert(name, sub_tag);
                }
                Ok(Self::Compound(map))
            }
            TAG_INT_ARRAY => {
                let len = read_i32(data)?;
                if len < 0 {
                    return Err(Error::Malformed("negative int array length in NBT"));
                }
                let len = len as usize;
                let mut ints = Vec::with_capacity(len);
                for _ in 0..len {
                    ints.push(read_i32(data)?);
                }
                Ok(Self::IntArray(ints))
            }
            _ => Err(Error::Malformed("unknown NBT tag type")),
        }
    }
}

fn read_u8(data: &mut &[u8]) -> Result<u8> {
    if data.is_empty() {
        return Err(Error::Malformed("unexpected EOF reading NBT u8"));
    }
    let b = data[0];
    *data = &data[1..];
    Ok(b)
}

fn read_i16(data: &mut &[u8]) -> Result<i16> {
    if data.len() < 2 {
        return Err(Error::Malformed("unexpected EOF reading NBT i16"));
    }
    let val = i16::from_be_bytes([data[0], data[1]]);
    *data = &data[2..];
    Ok(val)
}

fn read_i32(data: &mut &[u8]) -> Result<i32> {
    if data.len() < 4 {
        return Err(Error::Malformed("unexpected EOF reading NBT i32"));
    }
    let val = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    *data = &data[4..];
    Ok(val)
}

fn read_u32(data: &mut &[u8]) -> Result<u32> {
    if data.len() < 4 {
        return Err(Error::Malformed("unexpected EOF reading NBT u32"));
    }
    let val = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    *data = &data[4..];
    Ok(val)
}

fn read_i64(data: &mut &[u8]) -> Result<i64> {
    if data.len() < 8 {
        return Err(Error::Malformed("unexpected EOF reading NBT i64"));
    }
    let val = i64::from_be_bytes(data[..8].try_into().unwrap());
    *data = &data[8..];
    Ok(val)
}

fn read_u64(data: &mut &[u8]) -> Result<u64> {
    if data.len() < 8 {
        return Err(Error::Malformed("unexpected EOF reading NBT u64"));
    }
    let val = u64::from_be_bytes(data[..8].try_into().unwrap());
    *data = &data[8..];
    Ok(val)
}

fn read_string(data: &mut &[u8]) -> Result<String> {
    let len = read_i16(data)? as usize;
    if data.len() < len {
        return Err(Error::Malformed("unexpected EOF reading NBT string"));
    }
    let s = std::str::from_utf8(&data[..len])
        .map_err(|_| Error::Malformed("invalid UTF-8 in NBT string"))?
        .to_string();
    *data = &data[len..];
    Ok(s)
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tags of a compound keep the order of their insertion.
    ///
    /// Java `NBTTagCompound` keeps the tags in a `HashMap`, the port keeps
    /// them in the order of their insertion: the bytes of the files of the
    /// world are the same on every run.
    #[test]
    fn the_tags_of_the_nbt_keep_the_order_of_the_insertion() {
        let mut tag = NbtTag::compound();
        tag.insert("zebra", NbtTag::Int(1));
        tag.insert("apple", NbtTag::Int(2));
        tag.insert("mango", NbtTag::Int(3));
        let bytes = tag.to_bytes("");
        let zebra = bytes
            .windows(5)
            .position(|window| window == b"zebra")
            .expect("the name of the tag");
        let apple = bytes
            .windows(5)
            .position(|window| window == b"apple")
            .expect("the name of the tag");
        let mango = bytes
            .windows(5)
            .position(|window| window == b"mango")
            .expect("the name of the tag");
        assert!(zebra < apple && apple < mango, "the order of the tags");
    }

    #[test]
    fn nbt_round_trip_primitives() {
        let mut root = NbtTag::compound();
        root.insert("byteVal", NbtTag::Byte(42));
        root.insert("shortVal", NbtTag::Short(-1234));
        root.insert("intVal", NbtTag::Int(100_000));
        root.insert("longVal", NbtTag::Long(123_456_789_012));
        root.insert("floatVal", NbtTag::Float(std::f32::consts::PI));
        root.insert("doubleVal", NbtTag::Double(std::f64::consts::E));
        root.insert("stringVal", NbtTag::String("flintcore b1.7.3".into()));
        root.insert("bytesVal", NbtTag::ByteArray(vec![1, 2, 3, 4, 5]));
        root.insert(
            "listVal",
            NbtTag::List(vec![
                NbtTag::String("first".into()),
                NbtTag::String("second".into()),
            ]),
        );

        let mut nested = NbtTag::compound();
        nested.insert("inner", NbtTag::Int(999));
        root.insert("nested", nested);

        let bytes = root.to_bytes("TestRoot");
        let (name, decoded) = NbtTag::from_bytes(&bytes).unwrap();
        assert_eq!(name, "TestRoot");
        assert_eq!(decoded.get_i8("byteVal"), Some(42));
        assert_eq!(decoded.get_i16("shortVal"), Some(-1234));
        assert_eq!(decoded.get_i32("intVal"), Some(100_000));
        assert_eq!(decoded.get_i64("longVal"), Some(123_456_789_012));
        assert_eq!(decoded.get_str("stringVal"), Some("flintcore b1.7.3"));
        assert_eq!(
            decoded.get_byte_array("bytesVal"),
            Some([1, 2, 3, 4, 5].as_slice())
        );
        assert_eq!(
            decoded.get("nested").and_then(|n| n.get_i32("inner")),
            Some(999)
        );
    }

    #[test]
    fn nbt_round_trip_gzip() {
        let mut level_data = NbtTag::compound();
        level_data.insert("RandomSeed", NbtTag::Long(987654321));
        level_data.insert("SpawnX", NbtTag::Int(0));
        level_data.insert("SpawnY", NbtTag::Int(64));
        level_data.insert("SpawnZ", NbtTag::Int(0));
        level_data.insert("LevelName", NbtTag::String("World1".into()));

        let mut root = NbtTag::compound();
        root.insert("Data", level_data);

        let compressed = root.to_gzip("").unwrap();
        let (name, decoded) = NbtTag::from_gzip(&compressed).unwrap();
        assert_eq!(name, "");
        let data = decoded.get_compound("Data").unwrap();
        assert_eq!(data.get("RandomSeed"), Some(&NbtTag::Long(987654321)));
        assert_eq!(
            data.get("LevelName"),
            Some(&NbtTag::String("World1".into()))
        );
    }

    #[test]
    fn nbt_round_trip_zlib() {
        let mut chunk_level = NbtTag::compound();
        chunk_level.insert("xPos", NbtTag::Int(4));
        chunk_level.insert("zPos", NbtTag::Int(-3));
        chunk_level.insert("Blocks", NbtTag::ByteArray(vec![1; 32768]));

        let mut root = NbtTag::compound();
        root.insert("Level", chunk_level);

        let compressed = root.to_zlib("").unwrap();
        let (name, decoded) = NbtTag::from_zlib(&compressed).unwrap();
        assert_eq!(name, "");
        let level = decoded.get_compound("Level").unwrap();
        assert_eq!(level.get("xPos"), Some(&NbtTag::Int(4)));
        assert_eq!(level.get("zPos"), Some(&NbtTag::Int(-3)));
    }
}
