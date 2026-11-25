use crate::Error;
use crate::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liquid {
    Water,
    Lava,
    Honey,
    Shimmer,
}

/// A tile as stored on disk, before the engine's framing and terrain fixups.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub block: Option<u16>,
    pub frame: Option<(i16, i16)>,
    pub wall: u16,
    pub block_paint: u8,
    pub wall_paint: u8,
    pub liquid: Option<(Liquid, u8)>,
    /// Original chained header bytes, including wire, shape, actuator and coating bits.
    pub flags: [u8; 4],
}

pub fn read_run(
    r: &mut Reader<'_>,
    important: &[u8],
    type_count: usize,
) -> Result<(Tile, u32), Error> {
    let mut flags = [0u8; 4];
    flags[0] = r.byte()?;
    for i in 1..4 {
        if flags[i - 1] & 1 == 0 {
            break;
        }
        flags[i] = r.byte()?;
    }
    let [first, _, third, _] = flags;
    let mut tile = Tile {
        block: None,
        frame: None,
        wall: 0,
        block_paint: 0,
        wall_paint: 0,
        liquid: None,
        flags,
    };
    if first & 2 != 0 {
        let id = if first & 32 != 0 {
            r.u16()?
        } else {
            u16::from(r.byte()?)
        };
        let i = usize::from(id);
        if i >= type_count || i / 8 >= important.len() {
            return Err(Error::Tile("tile type exceeds the frame table"));
        }
        tile.block = Some(id);
        if important[i / 8] & (1 << (i % 8)) != 0 {
            tile.frame = Some((r.i16()?, r.i16()?));
        }
        if third & 8 != 0 {
            tile.block_paint = r.byte()?;
        }
    }
    if first & 4 != 0 {
        tile.wall = u16::from(r.byte()?);
        if third & 16 != 0 {
            tile.wall_paint = r.byte()?;
        }
    }
    let liquid = (first >> 3) & 3;
    if liquid != 0 {
        let kind = match liquid {
            _ if third & 128 != 0 => Liquid::Shimmer,
            2 => Liquid::Lava,
            3 => Liquid::Honey,
            _ => Liquid::Water,
        };
        tile.liquid = Some((kind, r.byte()?));
    }
    if third & 64 != 0 {
        tile.wall |= u16::from(r.byte()?) << 8;
    }
    let repeats = match first >> 6 {
        0 => 0,
        1 => u32::from(r.byte()?),
        _ => u32::try_from(r.i16()?).map_err(|_| Error::Tile("negative tile repeat count"))?,
    };
    Ok((tile, 1 + repeats))
}

pub fn write_run(
    out: &mut bytes::BytesMut,
    t: Tile,
    count: u16,
    important: &[u8],
    types: usize,
) -> Result<(), Error> {
    use bytes::BufMut;
    if count == 0 {
        return Err(Error::Tile("empty tile run"));
    }

    let [_, second, third, fourth] = t.flags;
    let mut second = second & 0x7e;
    let mut third = third & 0x26;
    let fourth = fourth & 0x1e;
    let mut first = 0;
    if let Some(block) = t.block {
        let id = usize::from(block);
        if id >= types
            || important
                .get(id / 8)
                .is_none_or(|b| (b & (1 << (id % 8)) != 0) != t.frame.is_some())
        {
            return Err(Error::Tile("block or frame"));
        }
        first |= 2;
        if block > 255 {
            first |= 32;
        }
        if t.block_paint != 0 {
            third |= 8;
        }
    }
    if t.wall != 0 {
        first |= 4;
        if t.wall_paint != 0 {
            third |= 16;
        }
        if t.wall > 255 {
            third |= 64;
        }
    }
    if let Some((kind, _)) = t.liquid {
        first |= match kind {
            Liquid::Water => 8,
            Liquid::Lava => 16,
            Liquid::Honey => 24,
            Liquid::Shimmer => {
                third |= 128;
                8
            }
        };
    }
    if fourth != 0 {
        third |= 1;
    }
    if third != 0 {
        second |= 1;
    }
    if second != 0 {
        first |= 1;
    }
    let repeats = count - 1;
    first |= match repeats {
        0 => 0,
        1..=255 => 64,
        _ => 128,
    };
    out.put_u8(first);
    if second != 0 {
        out.put_u8(second);
    }
    if third != 0 {
        out.put_u8(third);
    }
    if fourth != 0 {
        out.put_u8(fourth);
    }
    if let Some(block) = t.block {
        if block > 255 {
            out.put_u16_le(block)
        } else {
            out.put_u8(block as u8)
        }
        if let Some((x, y)) = t.frame {
            out.put_i16_le(x);
            out.put_i16_le(y);
        }
        if t.block_paint != 0 {
            out.put_u8(t.block_paint);
        }
    }
    if t.wall != 0 {
        out.put_u8(t.wall as u8);
        if t.wall_paint != 0 {
            out.put_u8(t.wall_paint);
        }
    }
    if let Some((_, amount)) = t.liquid {
        out.put_u8(amount);
    }
    if t.wall > 255 {
        out.put_u8((t.wall >> 8) as u8);
    }
    if repeats > 255 {
        out.put_u16_le(repeats)
    } else if repeats > 0 {
        out.put_u8(repeats as u8);
    }
    Ok(())
}
