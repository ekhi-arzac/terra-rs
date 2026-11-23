use crate::Error;
use terraria_binary::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liquid {
    Water,
    Lava,
    Honey,
    Shimmer,
}

/// A tile as stored on disk, before the engine's framing and terrain fixups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub(crate) fn read_run(
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
        if i >= type_count {
            return Err(Error::Invalid("tile type exceeds the frame table"));
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
        _ => u32::try_from(r.i16()?).map_err(|_| Error::Invalid("negative tile repeat count"))?,
    };
    Ok((tile, 1 + repeats))
}
