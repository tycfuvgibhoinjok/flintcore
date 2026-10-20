use crate::util::{Error, Result};

pub struct Reader<'a> {
    buffer: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer,
            position: 0,
        }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn remaining(&self) -> usize {
        self.buffer.len() - self.position
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.remaining() < count {
            return Err(Error::Incomplete);
        }
        let start = self.position;
        self.position += count;
        Ok(&self.buffer[start..self.position])
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(self.read_u16()? as i16)
    }

    pub fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(self.read_u32()? as i32)
    }

    pub fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.take(8)?;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub fn read_i64(&mut self) -> Result<i64> {
        Ok(self.read_u64()? as i64)
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_u64()?))
    }

    pub fn read_bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        self.take(count)
    }

    pub fn read_string(&mut self, max_chars: usize) -> Result<String> {
        let length = self.read_i16()?;
        if length < 0 {
            return Err(Error::NegativeStringLength(length));
        }
        let length = length as usize;
        if length > max_chars {
            return Err(Error::StringTooLong {
                max: max_chars,
                len: length,
            });
        }
        let bytes = self.take(length * 2)?;
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
        Ok(char::decode_utf16(units)
            .map(|unit| unit.unwrap_or('\u{FFFD}'))
            .collect())
    }

    /// Java `DataInput.readUTF`: the **modified** UTF-8.
    pub fn read_modified_utf8(&mut self, max_bytes: usize) -> Result<String> {
        let length = self.read_u16()? as usize;
        if length > max_bytes {
            return Err(Error::StringTooLong {
                max: max_bytes,
                len: length,
            });
        }
        let bytes = self.take(length)?;
        let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
        let mut index = 0usize;
        while index < bytes.len() {
            let first = bytes[index];
            if first < 0x80 {
                units.push(u16::from(first));
                index += 1;
            } else if first & 0xE0 == 0xC0 {
                let second = *bytes
                    .get(index + 1)
                    .ok_or(Error::Malformed("invalid utf-8"))?;
                units.push((u16::from(first & 0x1F) << 6) | u16::from(second & 0x3F));
                index += 2;
            } else if first & 0xF0 == 0xE0 {
                let second = *bytes
                    .get(index + 1)
                    .ok_or(Error::Malformed("invalid utf-8"))?;
                let third = *bytes
                    .get(index + 2)
                    .ok_or(Error::Malformed("invalid utf-8"))?;
                units.push(
                    (u16::from(first & 0x0F) << 12)
                        | (u16::from(second & 0x3F) << 6)
                        | u16::from(third & 0x3F),
                );
                index += 3;
            } else {
                return Err(Error::Malformed("invalid utf-8"));
            }
        }
        String::from_utf16(&units).map_err(|_| Error::Malformed("invalid utf-8"))
    }

    pub fn read_utf8(&mut self, max_bytes: usize) -> Result<String> {
        let length = self.read_u16()? as usize;
        if length > max_bytes {
            return Err(Error::StringTooLong {
                max: max_bytes,
                len: length,
            });
        }
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Malformed("invalid utf-8"))
    }
}

#[derive(Debug, Default)]
pub struct Writer {
    buffer: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        if self.buffer.capacity() > 131_072 {
            self.buffer.shrink_to(65536);
        }
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buffer
    }

    pub fn write_u8(&mut self, value: u8) {
        self.buffer.push(value);
    }

    pub fn write_i8(&mut self, value: i8) {
        self.write_u8(value as u8);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(value as u8);
    }

    pub fn write_u16(&mut self, value: u16) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    pub fn write_i16(&mut self, value: i16) {
        self.write_u16(value as u16);
    }

    pub fn write_u32(&mut self, value: u32) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    pub fn write_i32(&mut self, value: i32) {
        self.write_u32(value as u32);
    }

    pub fn write_u64(&mut self, value: u64) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    pub fn write_i64(&mut self, value: i64) {
        self.write_u64(value as u64);
    }

    pub fn write_f32(&mut self, value: f32) {
        self.write_u32(value.to_bits());
    }

    pub fn write_f64(&mut self, value: f64) {
        self.write_u64(value.to_bits());
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub fn write_string(&mut self, value: &str, max_chars: usize) -> Result<()> {
        let units: Vec<u16> = value.encode_utf16().collect();
        if units.len() > max_chars || units.len() > i16::MAX as usize {
            return Err(Error::StringTooLong {
                max: max_chars,
                len: units.len(),
            });
        }
        self.write_i16(units.len() as i16);
        for unit in units {
            self.write_u16(unit);
        }
        Ok(())
    }

    /// Java `DataOutput.writeUTF`: the **modified** UTF-8.
    ///
    /// The length is the number of the bytes, the null character is written
    /// as two bytes (`0xC0 0x80`) and the characters outside the basic
    /// plane are written as two sequences of three bytes (the surrogates).
    pub fn write_modified_utf8(&mut self, value: &str) -> Result<()> {
        let mut encoded = Vec::with_capacity(value.len());
        for unit in value.encode_utf16() {
            if unit == 0 {
                encoded.push(0xC0);
                encoded.push(0x80);
            } else if unit < 0x80 {
                encoded.push(unit as u8);
            } else if unit < 0x800 {
                encoded.push(0xC0 | (unit >> 6) as u8);
                encoded.push(0x80 | (unit & 0x3F) as u8);
            } else {
                encoded.push(0xE0 | (unit >> 12) as u8);
                encoded.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                encoded.push(0x80 | (unit & 0x3F) as u8);
            }
        }
        if encoded.len() > u16::MAX as usize {
            return Err(Error::StringTooLong {
                max: u16::MAX as usize,
                len: encoded.len(),
            });
        }
        self.write_u16(encoded.len() as u16);
        self.write_bytes(&encoded);
        Ok(())
    }

    pub fn write_utf8(&mut self, value: &str) -> Result<()> {
        let bytes = value.as_bytes();
        if bytes.len() > u16::MAX as usize {
            return Err(Error::StringTooLong {
                max: u16::MAX as usize,
                len: bytes.len(),
            });
        }
        self.write_u16(bytes.len() as u16);
        self.write_bytes(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The modified UTF-8 of the reference (`DataOutput.writeUTF`).
    ///
    /// The null character is written as two bytes and the characters outside
    /// the basic plane are written as two sequences of three bytes; the
    /// length is the number of the bytes.
    #[test]
    fn the_modified_utf8_of_the_reference_is_used() {
        let mut writer = Writer::new();
        writer
            .write_modified_utf8("a\0b")
            .expect("the string is written");
        let bytes = writer.into_inner();
        assert_eq!(&bytes[..2], &[0x00, 0x04], "the length is in bytes");
        assert_eq!(
            &bytes[2..],
            &[b'a', 0xC0, 0x80, b'b'],
            "the null character is two bytes"
        );

        // A character outside the basic plane is two sequences of three
        // bytes.
        let mut writer = Writer::new();
        writer
            .write_modified_utf8("\u{1F600}")
            .expect("the string is written");
        let bytes = writer.into_inner();
        assert_eq!(bytes.len(), 8, "the length is in bytes");
        assert_eq!(
            &bytes[2..],
            &[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80],
            "the surrogates are written apart"
        );

        // The round trip.
        let mut writer = Writer::new();
        writer
            .write_modified_utf8("a\0b\u{1F600}")
            .expect("the string is written");
        let bytes = writer.into_inner();
        let mut reader = Reader::new(&bytes);
        assert_eq!(
            reader.read_modified_utf8(255).expect("the string is read"),
            "a\0b\u{1F600}"
        );
    }

    #[test]
    fn integers_round_trip_big_endian() {
        let mut writer = Writer::new();
        writer.write_u8(0xAB);
        writer.write_i16(-2);
        writer.write_i32(0x0102_0304);
        writer.write_i64(-1);
        assert_eq!(
            writer.as_slice(),
            &[
                0xAB, 0xFF, 0xFE, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                0xFF
            ]
        );

        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(reader.read_u8().unwrap(), 0xAB);
        assert_eq!(reader.read_i16().unwrap(), -2);
        assert_eq!(reader.read_i32().unwrap(), 0x0102_0304);
        assert_eq!(reader.read_i64().unwrap(), -1);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn floats_round_trip() {
        let mut writer = Writer::new();
        writer.write_f32(1.5);
        writer.write_f64(-2.25);
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(reader.read_f32().unwrap(), 1.5);
        assert_eq!(reader.read_f64().unwrap(), -2.25);
    }

    #[test]
    fn strings_round_trip() {
        let mut writer = Writer::new();
        writer.write_string("Hello", 32).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(reader.read_string(32).unwrap(), "Hello");
    }

    #[test]
    fn partial_read_is_incomplete() {
        let mut reader = Reader::new(&[0x00, 0x01]);
        assert!(matches!(reader.read_i32(), Err(Error::Incomplete)));
        assert_eq!(reader.position(), 0);
    }

    #[test]
    fn string_length_is_bounded() {
        let mut writer = Writer::new();
        assert!(matches!(
            writer.write_string("toolong", 3),
            Err(Error::StringTooLong { .. })
        ));

        let mut reader = Reader::new(&[0x00, 0x05, 0x00, b'a']);
        assert!(matches!(
            reader.read_string(2),
            Err(Error::StringTooLong { .. })
        ));
    }
}
