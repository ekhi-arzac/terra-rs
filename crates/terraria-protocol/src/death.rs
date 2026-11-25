//! The stock player-death report carries optional attribution before its fixed fields.
use crate::wire::{Error, Reader, put_string};
use bytes::{BufMut, BytesMut};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeathReason {
    pub player: Option<i16>,
    pub npc: Option<i16>,
    pub projectile: Option<i16>,
    pub other: Option<u8>,
    pub projectile_type: Option<i16>,
    pub item: Option<i16>,
    pub prefix: Option<u8>,
    pub custom: Option<String>,
}

impl DeathReason {
    fn validate(&self) -> Result<(), Error> {
        if self.player.is_some_and(|v| !(0..255).contains(&v))
            || self.npc.is_some_and(|v| !(0..200).contains(&v))
            || self.projectile.is_some_and(|v| !(0..1000).contains(&v))
            || self
                .projectile_type
                .is_some_and(|v| !(0..1137).contains(&v))
            || self.item.is_some_and(|v| !(0..6196).contains(&v))
            || self.prefix.is_some_and(|v| v >= 98)
        {
            return Err(Error::Invalid("death reason index"));
        }
        Ok(())
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let flags = r.byte()?;
        let reason = Self {
            player: (flags & 1 != 0).then(|| r.i16()).transpose()?,
            npc: (flags & 2 != 0).then(|| r.i16()).transpose()?,
            projectile: (flags & 4 != 0).then(|| r.i16()).transpose()?,
            other: (flags & 8 != 0).then(|| r.byte()).transpose()?,
            projectile_type: (flags & 16 != 0).then(|| r.i16()).transpose()?,
            item: (flags & 32 != 0).then(|| r.i16()).transpose()?,
            prefix: (flags & 64 != 0).then(|| r.byte()).transpose()?,
            custom: (flags & 128 != 0)
                .then(|| r.string().map(str::to_owned))
                .transpose()?,
        };
        reason.validate()?;
        Ok(reason)
    }

    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        let fields = [
            self.player.is_some(),
            self.npc.is_some(),
            self.projectile.is_some(),
            self.other.is_some(),
            self.projectile_type.is_some(),
            self.item.is_some(),
            self.prefix.is_some(),
            self.custom.is_some(),
        ];
        w.put_u8(
            fields
                .into_iter()
                .enumerate()
                .fold(0, |mask, (bit, present)| mask | u8::from(present) << bit),
        );
        for value in [self.player, self.npc, self.projectile]
            .into_iter()
            .flatten()
        {
            w.put_i16_le(value);
        }
        if let Some(value) = self.other {
            w.put_u8(value);
        }
        for value in [self.projectile_type, self.item].into_iter().flatten() {
            w.put_i16_le(value);
        }
        if let Some(value) = self.prefix {
            w.put_u8(value);
        }
        if let Some(value) = &self.custom {
            put_string(w, value)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerDeath {
    pub player: u8,
    pub reason: DeathReason,
    pub damage: i16,
    pub direction: i8,
    pub pvp: bool,
}

impl PlayerDeath {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let player = r.byte()?;
        let reason = DeathReason::read(r)?;
        let damage = r.i16()?;
        let direction = r.byte()?;
        if direction > 2 {
            return Err(Error::Invalid("player death direction"));
        }
        Ok(Self {
            player,
            reason,
            damage,
            direction: direction as i8 - 1,
            pvp: r.byte()? & 1 != 0,
        })
    }
    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        if !(-1..=1).contains(&self.direction) {
            return Err(Error::Invalid("player death direction"));
        }
        w.put_u8(self.player);
        self.reason.write(w)?;
        w.put_i16_le(self.damage);
        w.put_u8((self.direction + 1) as u8);
        w.put_u8(u8::from(self.pvp));
        Ok(())
    }
}
