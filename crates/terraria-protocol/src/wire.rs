use bytes::{BufMut, Bytes, BytesMut};
pub(crate) use terraria_binary::Reader;
use thiserror::Error;

pub const MAX_FRAME: usize = u16::MAX as usize;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("frame length {0} is outside 3..=65535")]
    Length(usize),
    #[error(transparent)]
    Field(#[from] terraria_binary::Error),
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
