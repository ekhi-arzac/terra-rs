use crate::wire::{Error, Reader};
use bytes::{BufMut, BytesMut};

pub const NEW_ITEM: i16 = 400;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroppedItem {
    pub slot: i16,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub stack: i16,
    pub prefix: u8,
    /// Spawn ownership mode in the low two bits of the wire flags.
    pub ownership: u8,
    pub item: i16,
    pub shimmer: Option<(bool, f32)>,
    pub enemy_delay: Option<u8>,
}

fn vector(r: &mut Reader<'_>) -> Result<[f32; 2], Error> {
    let v = [r.f32()?, r.f32()?];
    if v.iter().all(|n| n.is_finite()) {
        Ok(v)
    } else {
        Err(Error::Invalid("item coordinate"))
    }
}

impl DroppedItem {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let slot = r.i16()?;
        let position = vector(r)?;
        let velocity = vector(r)?;
        let stack = r.i16()?;
        let prefix = r.byte()?;
        let flags = r.byte()?;
        let item = r.i16()?;
        let shimmer = if flags & 4 != 0 {
            let value = (r.byte()? != 0, r.f32()?);
            if !value.1.is_finite() {
                return Err(Error::Invalid("item shimmer time"));
            }
            Some(value)
        } else {
            None
        };
        let enemy_delay = (flags & 8 != 0).then(|| r.byte()).transpose()?;
        Ok(Self {
            slot,
            position,
            velocity,
            stack,
            prefix,
            ownership: flags & 3,
            item,
            shimmer,
            enemy_delay,
        })
    }
    pub(crate) fn write(&self, w: &mut BytesMut) {
        w.put_i16_le(self.slot);
        for v in self.position.into_iter().chain(self.velocity) {
            w.put_f32_le(v);
        }
        w.put_i16_le(self.stack);
        w.put_u8(self.prefix);
        w.put_u8(
            self.ownership & 3
                | (u8::from(self.shimmer.is_some()) << 2)
                | (u8::from(self.enemy_delay.is_some()) << 3),
        );
        w.put_i16_le(self.item);
        if let Some((shimmered, time)) = self.shimmer {
            w.put_u8(shimmered.into());
            w.put_f32_le(time);
        }
        if let Some(delay) = self.enemy_delay {
            w.put_u8(delay);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemOwner {
    pub slot: i16,
    pub player: u8,
    pub reservation: u32,
    pub delayed_player: u8,
    pub delay: u32,
    pub position: [f32; 2],
}

fn ticks(r: &mut Reader<'_>) -> Result<u32, Error> {
    let mut n = 0;
    for shift in (0..=28).step_by(7) {
        let byte = r.byte()?;
        if shift == 28 && byte > 7 {
            return Err(Error::Invalid("item timer"));
        }
        n |= u32::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(n);
        }
    }
    Err(Error::Invalid("item timer"))
}
fn put_ticks(w: &mut BytesMut, mut n: u32) -> Result<(), Error> {
    if n > i32::MAX as u32 {
        return Err(Error::Invalid("item timer"));
    }
    while n >= 128 {
        w.put_u8(n as u8 | 128);
        n >>= 7;
    }
    w.put_u8(n as u8);
    Ok(())
}
impl ItemOwner {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            slot: r.i16()?,
            player: r.byte()?,
            reservation: ticks(r)?,
            delayed_player: r.byte()?,
            delay: ticks(r)?,
            position: vector(r)?,
        })
    }
    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        w.put_i16_le(self.slot);
        w.put_u8(self.player);
        put_ticks(w, self.reservation)?;
        w.put_u8(self.delayed_player);
        put_ticks(w, self.delay)?;
        for v in self.position {
            w.put_f32_le(v);
        }
        Ok(())
    }
}
