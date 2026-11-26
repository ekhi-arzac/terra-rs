//! Projectile synchronization carries a client slot, local index and generation on the wire.
use crate::wire::{Error, Reader};
use bytes::{BufMut, BytesMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectileKey {
    pub spawner: u8,
    pub index: u16,
    pub generation: u16,
}

impl ProjectileKey {
    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let bits = u32::from_le_bytes(r.take()?);
        let key = Self {
            spawner: bits as u8,
            index: ((bits >> 8) & 1023) as u16,
            generation: (bits >> 18) as u16,
        };
        key.validate()?;
        Ok(key)
    }

    fn validate(self) -> Result<(), Error> {
        if self.index >= 1000 || self.generation >= 16384 {
            return Err(Error::Invalid("projectile key"));
        }
        Ok(())
    }

    fn write(self, out: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        out.put_u32_le(
            u32::from(self.spawner)
                | (u32::from(self.index) << 8)
                | (u32::from(self.generation) << 18),
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileSync {
    pub key: ProjectileKey,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub kind: i16,
    pub ai: [Option<f32>; 3],
    pub banner: Option<u16>,
    pub damage: Option<i16>,
    pub knockback: Option<f32>,
    pub original_damage: Option<i16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileRemove {
    pub key: ProjectileKey,
    pub position: [f32; 2],
}

fn finite(value: f32) -> Result<f32, Error> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::Invalid("non-finite projectile field"))
    }
}

impl ProjectileSync {
    fn validate(self) -> Result<(), Error> {
        if !(1..1137).contains(&self.kind) {
            return Err(Error::Invalid("projectile type"));
        }
        for value in self
            .position
            .into_iter()
            .chain(self.velocity)
            .chain(self.ai.into_iter().flatten())
            .chain(self.knockback)
        {
            finite(value)?;
        }
        Ok(())
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let key = ProjectileKey::read(r)?;
        let position = [r.f32()?, r.f32()?];
        let velocity = [r.f32()?, r.f32()?];
        let kind = r.i16()?;
        let flags = r.byte()?;
        let extra = if flags & 4 != 0 { r.byte()? } else { 0 };
        if flags & 128 != 0 || extra & !1 != 0 {
            return Err(Error::Invalid("projectile flags"));
        }
        let ai0 = (flags & 1 != 0).then(|| r.f32()).transpose()?;
        let ai1 = (flags & 2 != 0).then(|| r.f32()).transpose()?;
        let banner = (flags & 8 != 0).then(|| r.u16()).transpose()?;
        let damage = (flags & 16 != 0).then(|| r.i16()).transpose()?;
        let knockback = (flags & 32 != 0).then(|| r.f32()).transpose()?;
        let original_damage = (flags & 64 != 0).then(|| r.i16()).transpose()?;
        let ai2 = (extra & 1 != 0).then(|| r.f32()).transpose()?;
        let update = Self {
            key,
            position,
            velocity,
            kind,
            ai: [ai0, ai1, ai2],
            banner,
            damage,
            knockback,
            original_damage,
        };
        update.validate()?;
        Ok(update)
    }

    pub(crate) fn write(self, out: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        self.key.write(out)?;
        for value in self.position.into_iter().chain(self.velocity) {
            out.put_f32_le(value);
        }
        out.put_i16_le(self.kind);
        let present = [
            self.ai[0].is_some(),
            self.ai[1].is_some(),
            self.ai[2].is_some(),
            self.banner.is_some(),
            self.damage.is_some(),
            self.knockback.is_some(),
            self.original_damage.is_some(),
        ];
        let flags = present
            .into_iter()
            .enumerate()
            .fold(0, |flags, (bit, set)| flags | (u8::from(set) << bit));
        out.put_u8(flags);
        if self.ai[2].is_some() {
            out.put_u8(1);
        }
        for value in self.ai[..2].iter().flatten() {
            out.put_f32_le(*value);
        }
        if let Some(value) = self.banner {
            out.put_u16_le(value);
        }
        if let Some(value) = self.damage {
            out.put_i16_le(value);
        }
        if let Some(value) = self.knockback {
            out.put_f32_le(value);
        }
        if let Some(value) = self.original_damage {
            out.put_i16_le(value);
        }
        if let Some(value) = self.ai[2] {
            out.put_f32_le(value);
        }
        Ok(())
    }
}

impl ProjectileRemove {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            key: ProjectileKey::read(r)?,
            position: [finite(r.f32()?)?, finite(r.f32()?)?],
        })
    }

    pub(crate) fn write(self, out: &mut BytesMut) -> Result<(), Error> {
        self.key.write(out)?;
        for value in self.position {
            out.put_f32_le(finite(value)?);
        }
        Ok(())
    }
}
