//! Checked primitives shared by the managed file and network formats.
pub mod tiles;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("incomplete field")]
    Truncated,
    #[error("invalid 7-bit string length")]
    StringLength,
    #[error("invalid UTF-8")]
    Utf8,
    #[error("invalid tile record: {0}")]
    Tile(&'static str),
}

pub struct Reader<'a>(pub &'a [u8]);

impl<'a> Reader<'a> {
    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let (value, rest) = self.0.split_at_checked(count).ok_or(Error::Truncated)?;
        self.0 = rest;
        Ok(value)
    }
    pub fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    pub fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let (value, rest) = self.0.split_first_chunk().ok_or(Error::Truncated)?;
        self.0 = rest;
        Ok(*value)
    }
    pub fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take::<1>()?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    pub fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_le_bytes(self.take()?))
    }
    pub fn i32s<const N: usize>(&mut self) -> Result<[i32; N], Error> {
        let mut values = [0; N];
        for value in &mut values {
            *value = self.i32()?;
        }
        Ok(values)
    }
    pub fn i16s<const N: usize>(&mut self) -> Result<[i16; N], Error> {
        let mut values = [0; N];
        for value in &mut values {
            *value = self.i16()?;
        }
        Ok(values)
    }
    pub fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_le_bytes(self.take()?))
    }
    pub fn f64(&mut self) -> Result<f64, Error> {
        Ok(f64::from_le_bytes(self.take()?))
    }
    pub fn string(&mut self) -> Result<&'a str, Error> {
        let mut len = 0u32;
        for shift in (0..35).step_by(7) {
            let part = self.byte()?;
            if shift == 28 && part > 7 {
                return Err(Error::StringLength);
            }
            len |= u32::from(part & 0x7f) << shift;
            if part < 128 {
                let value = self.bytes(len as usize)?;
                return std::str::from_utf8(value).map_err(|_| Error::Utf8);
            }
        }
        Err(Error::StringLength)
    }
}

/// Write a .NET string with its 7-bit UTF-8 byte length.
pub fn put_string(out: &mut bytes::BytesMut, value: &str) {
    use bytes::BufMut;
    let mut len = value.len();
    while len >= 128 {
        out.put_u8((len as u8 & 0x7f) | 0x80);
        len >>= 7;
    }
    out.put_u8(len as u8);
    out.extend_from_slice(value.as_bytes());
}
