use crate::wire::{Error, Reader};
use bytes::{BufMut, BytesMut};

pub const BANNER_TYPES: usize = 293;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerState {
    pub kills: [i32; BANNER_TYPES],
    pub claimable: [u16; BANNER_TYPES],
}
impl Default for BannerState {
    fn default() -> Self {
        Self {
            kills: [0; BANNER_TYPES],
            claimable: [0; BANNER_TYPES],
        }
    }
}
impl BannerState {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        if r.byte()? != 0 || r.i16()? != BANNER_TYPES as i16 {
            return Err(Error::Invalid("banner full state"));
        }
        let kills = r.i32s()?;
        if r.i16()? != BANNER_TYPES as i16 {
            return Err(Error::Invalid("banner claim count"));
        }
        let claimable = r.i16s::<BANNER_TYPES>()?.map(|v| v as u16);
        Ok(Self { kills, claimable })
    }
    pub(crate) fn write(&self, out: &mut BytesMut) {
        out.put_u8(0);
        out.put_i16_le(BANNER_TYPES as i16);
        for &n in &self.kills {
            out.put_i32_le(n);
        }
        out.put_i16_le(BANNER_TYPES as i16);
        for &n in &self.claimable {
            out.put_u16_le(n);
        }
    }
}
