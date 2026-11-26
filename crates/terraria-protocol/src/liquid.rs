//! The direct liquid update (48) and the server's batched liquid module (82/0).
use crate::wire::{Error, MAX_FRAME, Reader};
use bytes::{BufMut, BytesMut};
use terraria_binary::tiles::Liquid;

pub(crate) fn read_kind(r: &mut Reader<'_>) -> Result<Liquid, Error> {
    match r.byte()? {
        0 => Ok(Liquid::Water),
        1 => Ok(Liquid::Lava),
        2 => Ok(Liquid::Honey),
        3 => Ok(Liquid::Shimmer),
        _ => Err(Error::Invalid("liquid type")),
    }
}

pub(crate) fn write_kind(out: &mut BytesMut, kind: Liquid) {
    out.put_u8(match kind {
        Liquid::Water => 0,
        Liquid::Lava => 1,
        Liquid::Honey => 2,
        Liquid::Shimmer => 3,
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiquidUpdate {
    pub position: [i16; 2],
    pub amount: u8,
    pub kind: Liquid,
}

impl LiquidUpdate {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            position: r.i16s()?,
            amount: r.byte()?,
            kind: read_kind(r)?,
        })
    }

    pub(crate) fn write(self, out: &mut BytesMut) {
        for component in self.position {
            out.put_i16_le(component);
        }
        out.put_u8(self.amount);
        write_kind(out, self.kind);
    }
}

pub(crate) fn read_batch(r: &mut Reader<'_>) -> Result<Vec<LiquidUpdate>, Error> {
    let count = usize::from(r.u16()?);
    if count > r.0.len() / 6 {
        return Err(Error::Invalid("liquid batch length"));
    }
    let mut changes = Vec::with_capacity(count);
    for _ in 0..count {
        let y = r.i16()?;
        let x = r.i16()?;
        if x < 0 || y < 0 {
            return Err(Error::Invalid("liquid batch coordinate"));
        }
        changes.push(LiquidUpdate {
            position: [x, y],
            amount: r.byte()?,
            kind: read_kind(r)?,
        });
    }
    Ok(changes)
}

pub(crate) fn write_batch(out: &mut BytesMut, changes: &[LiquidUpdate]) -> Result<(), Error> {
    if changes.len() > (MAX_FRAME - 7) / 6 {
        return Err(Error::Invalid("liquid batch length"));
    }
    out.put_u16_le(0);
    out.put_u16_le(changes.len() as u16);
    for &LiquidUpdate {
        position: [x, y],
        amount,
        kind,
    } in changes
    {
        if x < 0 || y < 0 {
            return Err(Error::Invalid("liquid batch coordinate"));
        }
        out.put_i16_le(y);
        out.put_i16_le(x);
        out.put_u8(amount);
        write_kind(out, kind);
    }
    Ok(())
}
