use crate::{
    liquid::{read_kind, write_kind},
    section::{FRAME_IMPORTANT, TILE_TYPES},
    wire::{Error, Reader},
};
use bytes::{BufMut, BytesMut};
use terraria_binary::tiles::Tile;

/// Active terrain update (20). Cells are ordered by column, from top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TilePatch {
    pub origin: [i16; 2],
    pub size: [u8; 2],
    pub reason: u8,
    pub tiles: Vec<Tile>,
}

fn framed(block: u16) -> Result<bool, Error> {
    let i = usize::from(block);
    if i >= TILE_TYPES {
        return Err(Error::Invalid("tile patch block type"));
    }
    Ok(FRAME_IMPORTANT[i / 8] & (1 << (i % 8)) != 0)
}

impl TilePatch {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let origin = r.i16s()?;
        let size = r.take::<2>()?;
        let reason = r.byte()?;
        let count = usize::from(size[0]) * usize::from(size[1]);
        if count == 0 || count > r.0.len() / 3 {
            return Err(Error::Invalid("tile patch size"));
        }
        let mut tiles = Vec::with_capacity(count);
        for _ in 0..count {
            let [a, b, c] = r.take::<3>()?;
            let block_paint = if b & 4 != 0 { r.byte()? } else { 0 };
            let wall_paint = if b & 8 != 0 { r.byte()? } else { 0 };
            let block = (a & 1 != 0).then(|| r.u16()).transpose()?;
            let frame = match block {
                Some(block) if framed(block)? => Some((r.i16()?, r.i16()?)),
                _ => None,
            };
            let wall = if a & 4 != 0 { r.u16()? } else { 0 };
            let liquid = if a & 8 != 0 {
                let amount = r.byte()?;
                Some((read_kind(r)?, amount))
            } else {
                None
            };
            let shape = if a & 32 != 0 {
                16
            } else if b & 0x70 != 0 {
                (b & 0x70) + 16
            } else {
                0
            };
            tiles.push(Tile {
                block,
                frame,
                wall,
                block_paint,
                wall_paint,
                liquid,
                flags: [
                    0,
                    shape | (a & 16) >> 3 | (b & 3) << 2,
                    (a & 0xc0) >> 5 | (b & 0x80) >> 2,
                    (c & 3) << 3 | (c & 12) >> 1,
                ],
            });
        }
        Ok(Self {
            origin,
            size,
            reason,
            tiles,
        })
    }

    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        let count = usize::from(self.size[0]) * usize::from(self.size[1]);
        if count == 0 || count != self.tiles.len() {
            return Err(Error::Invalid("tile patch size"));
        }
        for n in self.origin {
            w.put_i16_le(n);
        }
        w.extend_from_slice(&self.size);
        w.put_u8(self.reason);
        for t in &self.tiles {
            let [_, wires_shape, actuation, coating] = t.flags;
            let shape = wires_shape & 0x70;
            let a = u8::from(t.block.is_some())
                | (u8::from(t.wall != 0) << 2)
                | (u8::from(t.liquid.is_some()) << 3)
                | (wires_shape & 2) << 3
                | (u8::from(shape == 16) << 5)
                | (actuation & 6) << 5;
            // Send zero paint explicitly so a correction can also clear old paint.
            let b = (wires_shape & 12) >> 2 | 12 | shape.saturating_sub(16) | (actuation & 32) << 2;
            let c = (coating & 24) >> 3 | (coating & 6) << 1;
            w.extend_from_slice(&[a, b, c, t.block_paint, t.wall_paint]);
            if let Some(block) = t.block {
                w.put_u16_le(block);
                if framed(block)? != t.frame.is_some() {
                    return Err(Error::Invalid("tile patch frame"));
                }
                if let Some((x, y)) = t.frame {
                    w.put_i16_le(x);
                    w.put_i16_le(y);
                }
            }
            if t.wall != 0 {
                w.put_u16_le(t.wall);
            }
            if let Some((liquid, amount)) = t.liquid {
                w.put_u8(amount);
                write_kind(w, liquid);
            }
        }
        Ok(())
    }
}
