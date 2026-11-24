//! The two NPC lists and shimmer flags written by release 326.
use crate::Error;
use terraria_binary::Reader;

#[derive(Debug)]
pub struct Npc {
    pub kind: i32,
    pub position: [f32; 2],
    pub town: Option<TownNpc>,
}

#[derive(Debug)]
pub struct TownNpc {
    pub name: String,
    pub homeless: bool,
    pub home: [i32; 2],
    pub variation: Option<i32>,
    pub homeless_despawn: bool,
}

pub(crate) fn read(bytes: &[u8]) -> Result<(Vec<i32>, Vec<Npc>), Error> {
    let mut r = Reader(bytes);
    let count = usize::try_from(r.i32()?).map_err(|_| Error::Invalid("negative shimmer count"))?;
    if count > r.0.len() / 4 {
        return Err(Error::Invalid("shimmer count exceeds NPC section"));
    }
    let shimmered = (0..count).map(|_| r.i32()).collect::<Result<_, _>>()?;
    let mut npcs = Vec::new();
    for is_town in [true, false] {
        while r.byte()? != 0 {
            if npcs.len() >= 200 {
                return Err(Error::Invalid("saved NPC slot limit"));
            }
            let kind = r.i32()?;
            let name = if is_town {
                Some(r.string()?.to_owned())
            } else {
                None
            };
            let position = [r.f32()?, r.f32()?];
            if !position.iter().all(|n| n.is_finite()) {
                return Err(Error::Invalid("saved NPC position"));
            }
            let town = if let Some(name) = name {
                Some(TownNpc {
                    name,
                    homeless: r.byte()? != 0,
                    home: r.i32s()?,
                    variation: if r.byte()? & 1 != 0 {
                        Some(r.i32()?)
                    } else {
                        None
                    },
                    homeless_despawn: r.byte()? != 0,
                })
            } else {
                None
            };
            npcs.push(Npc {
                kind,
                position,
                town,
            });
        }
    }
    if !r.0.is_empty() {
        return Err(Error::Invalid("NPC section length"));
    }
    Ok((shimmered, npcs))
}
