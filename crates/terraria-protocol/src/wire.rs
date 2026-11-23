use bytes::{BufMut, Bytes, BytesMut};
use thiserror::Error;

pub const MAX_FRAME: usize = u16::MAX as usize;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("frame length {0} is outside 3..=65535")]
    Length(usize),
    #[error("incomplete field")]
    Truncated,
    #[error("invalid 7-bit string length")]
    StringLength,
    #[error("invalid UTF-8")]
    Utf8,
    #[error("packet has {0} trailing bytes")]
    Trailing(usize),
    #[error("unsupported message ID {0}")]
    Unknown(u8),
    #[error("invalid {0}")]
    Invalid(&'static str),
}

pub fn frame_len(prefix: [u8; 2]) -> Result<usize, Error> {
    let len = usize::from(u16::from_le_bytes(prefix));
    if len < 3 {
        Err(Error::Length(len))
    } else {
        Ok(len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: u8,
    pub payload: Bytes,
}

impl Frame {
    /// Consume one whole frame. An incomplete frame stays in the input buffer.
    pub fn split(input: &mut BytesMut) -> Result<Option<Self>, Error> {
        let Some(prefix) = input.first_chunk() else {
            return Ok(None);
        };
        let len = frame_len(*prefix)?;
        if input.len() < len {
            return Ok(None);
        }
        let data = input.split_to(len).freeze();
        Ok(Some(Self {
            id: data[2],
            payload: data.slice(3..),
        }))
    }
}

pub(crate) struct Reader<'a>(pub &'a [u8]);

impl<'a> Reader<'a> {
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
    pub fn string(&mut self) -> Result<String, Error> {
        let mut len = 0u32;
        for shift in (0..35).step_by(7) {
            let part = self.byte()?;
            if shift == 28 && part > 7 {
                return Err(Error::StringLength);
            }
            len |= u32::from(part & 0x7f) << shift;
            if part < 128 {
                let (value, rest) = self
                    .0
                    .split_at_checked(len as usize)
                    .ok_or(Error::Truncated)?;
                self.0 = rest;
                return Ok(std::str::from_utf8(value)
                    .map_err(|_| Error::Utf8)?
                    .to_owned());
            }
        }
        Err(Error::StringLength)
    }
}

pub(crate) fn put_string(out: &mut BytesMut, value: &str) -> Result<(), Error> {
    let mut len = value.len();
    let total = out.len().saturating_add(len);
    if total >= MAX_FRAME {
        return Err(Error::Length(total.saturating_add(1)));
    }
    while len >= 128 {
        out.put_u8((len as u8 & 0x7f) | 0x80);
        len >>= 7;
    }
    out.put_u8(len as u8);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
