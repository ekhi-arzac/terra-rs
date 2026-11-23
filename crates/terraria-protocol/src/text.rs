use crate::wire::{Error, Reader, put_string};
use bytes::{BufMut, BytesMut};

/// A localizable string as sent by the game. Literal strings have no child count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkText {
    Literal(String),
    Formattable(String, Vec<Self>),
    Key(String, Vec<Self>),
}

impl NetworkText {
    pub fn key(key: &str) -> Self {
        Self::Key(key.into(), Vec::new())
    }

    pub(crate) fn read(r: &mut Reader<'_>, depth: usize) -> Result<Self, Error> {
        if depth >= 32 {
            return Err(Error::Invalid("NetworkText nesting depth"));
        }
        let mode = r.byte()?;
        let value = r.string()?.to_owned();
        if mode == 0 {
            return Ok(Self::Literal(value));
        }
        if mode > 2 {
            return Err(Error::Invalid("NetworkText mode"));
        }
        let count = r.byte()?;
        let mut children = Vec::new();
        for _ in 0..count {
            children.push(Self::read(r, depth + 1)?);
        }
        Ok(if mode == 1 {
            Self::Formattable(value, children)
        } else {
            Self::Key(value, children)
        })
    }
    pub(crate) fn write(&self, out: &mut BytesMut, depth: usize) -> Result<(), Error> {
        if depth >= 32 {
            return Err(Error::Invalid("NetworkText nesting depth"));
        }
        let (mode, value, children) = match self {
            Self::Literal(v) => (0, v, None),
            Self::Formattable(v, c) => (1, v, Some(c)),
            Self::Key(v, c) => (2, v, Some(c)),
        };
        out.put_u8(mode);
        put_string(out, value)?;
        if let Some(children) = children {
            out.put_u8(
                children
                    .len()
                    .try_into()
                    .map_err(|_| Error::Invalid("substitution count"))?,
            );
            for child in children {
                child.write(out, depth + 1)?;
            }
        }
        Ok(())
    }
}
