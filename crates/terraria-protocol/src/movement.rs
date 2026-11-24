use crate::wire::{Error, Reader};
use bytes::{BufMut, BytesMut};

/// PlayerControls (13). The presence bits are derived from the optional values on write.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Movement {
    pub player: u8,
    pub flags: [u8; 4],
    pub selected: u8,
    pub position: [f32; 2],
    pub velocity: Option<[f32; 2]>,
    pub mount: Option<u16>,
    pub return_points: Option<[[f32; 2]; 2]>,
    pub camera: Option<[f32; 2]>,
}

fn vector(r: &mut Reader<'_>) -> Result<[f32; 2], Error> {
    let v = [r.f32()?, r.f32()?];
    if v.iter().all(|n| n.is_finite()) {
        Ok(v)
    } else {
        Err(Error::Invalid("non-finite player coordinate"))
    }
}
impl Movement {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let player = r.byte()?;
        let flags = r.take::<4>()?;
        let selected = r.byte()?;
        let position = vector(r)?;
        let velocity = (flags[1] & 4 != 0).then(|| vector(r)).transpose()?;
        let mount = (flags[1] & 128 != 0).then(|| r.u16()).transpose()?;
        let return_points = if flags[2] & 64 != 0 {
            Some([vector(r)?, vector(r)?])
        } else {
            None
        };
        let camera = (flags[3] & 32 != 0).then(|| vector(r)).transpose()?;
        Ok(Self {
            player,
            flags,
            selected,
            position,
            velocity,
            mount,
            return_points,
            camera,
        })
    }
    pub(crate) fn write(&self, out: &mut BytesMut) -> Result<(), Error> {
        let mut flags = self.flags;
        flags[1] = (flags[1] & !132)
            | (u8::from(self.velocity.is_some()) << 2)
            | (u8::from(self.mount.is_some()) << 7);
        flags[2] = (flags[2] & !64) | (u8::from(self.return_points.is_some()) << 6);
        flags[3] = (flags[3] & !32) | (u8::from(self.camera.is_some()) << 5);
        out.put_u8(self.player);
        out.extend_from_slice(&flags);
        out.put_u8(self.selected);
        fn put(out: &mut BytesMut, v: [f32; 2]) -> Result<(), Error> {
            for n in v {
                if !n.is_finite() {
                    return Err(Error::Invalid("non-finite player coordinate"));
                }
                out.put_f32_le(n)
            }
            Ok(())
        }
        put(out, self.position)?;
        if let Some(v) = self.velocity {
            put(out, v)?
        }
        if let Some(v) = self.mount {
            out.put_u16_le(v)
        }
        if let Some(pair) = self.return_points {
            for v in pair {
                put(out, v)?
            }
        }
        if let Some(v) = self.camera {
            put(out, v)?
        }
        Ok(())
    }
}
