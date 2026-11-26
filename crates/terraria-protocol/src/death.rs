//! Player hurt and death reports share optional attribution and an impact direction.
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

fn read_direction(r: &mut Reader<'_>) -> Result<i8, Error> {
    match r.byte()? {
        value @ 0..=2 => Ok(value as i8 - 1),
        _ => Err(Error::Invalid("impact direction")),
    }
}

fn write_direction(direction: i8, w: &mut BytesMut) -> Result<(), Error> {
    if !(-1..=1).contains(&direction) {
        return Err(Error::Invalid("impact direction"));
    }
    w.put_u8((direction + 1) as u8);
    Ok(())
}

impl PlayerDeath {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let player = r.byte()?;
        let reason = DeathReason::read(r)?;
        let damage = r.i16()?;
        let direction = read_direction(r)?;
        Ok(Self {
            player,
            reason,
            damage,
            direction,
            pvp: r.byte()? & 1 != 0,
        })
    }
    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        w.put_u8(self.player);
        self.reason.write(w)?;
        w.put_i16_le(self.damage);
        write_direction(self.direction, w)?;
        w.put_u8(u8::from(self.pvp));
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerHurt {
    pub player: u8,
    pub reason: DeathReason,
    pub damage: i16,
    pub direction: i8,
    pub critical: bool,
    pub pvp: bool,
    pub dodgeable: bool,
    pub immunity_slot: i8,
}

impl PlayerHurt {
    fn validate(&self) -> Result<(), Error> {
        if self.player == 255 || self.damage <= 0 || !(-1..=5).contains(&self.immunity_slot) {
            return Err(Error::Invalid("player hurt fields"));
        }
        Ok(())
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let player = r.byte()?;
        let reason = DeathReason::read(r)?;
        let damage = r.i16()?;
        let direction = read_direction(r)?;
        let flags = r.byte()?;
        let hurt = Self {
            player,
            reason,
            damage,
            direction,
            critical: flags & 1 != 0,
            pvp: flags & 2 != 0,
            dodgeable: flags & 4 != 0,
            immunity_slot: r.byte()? as i8,
        };
        hurt.validate()?;
        Ok(hurt)
    }

    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        w.put_u8(self.player);
        self.reason.write(w)?;
        w.put_i16_le(self.damage);
        write_direction(self.direction, w)?;
        let flags =
            u8::from(self.critical) | u8::from(self.pvp) << 1 | u8::from(self.dodgeable) << 2;
        w.put_u8(flags);
        w.put_i8(self.immunity_slot);
        Ok(())
    }
}
