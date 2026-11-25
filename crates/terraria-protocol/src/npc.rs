//! Release-329 SyncNPC fields. Health width follows maximum health on the sender.
use crate::wire::{Error, Reader};
use bytes::{BufMut, BytesMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpcLife {
    Full,
    Byte(i8),
    Short(i16),
    Int(i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NpcSync {
    pub slot: u8,
    pub generation: u8,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub target: u16,
    pub right: bool,
    pub down: bool,
    pub sprite_right: bool,
    pub ai: [Option<f32>; 4],
    pub kind: i16,
    pub scaled_players: Option<u8>,
    pub statue: bool,
    pub difficulty: Option<f32>,
    pub spawning: bool,
    pub shimmer: bool,
    pub life: NpcLife,
    pub release_owner: Option<u8>,
}

// Main.Initialize_TileAndNPCData1 sets these catchable types in release 329.
fn catchable(kind: i16) -> bool {
    matches!(kind, 46 | 55 | 74 | 148..=149 | 297..=300 | 355..=367 | 374 | 377 |
        442..=448 | 484..=487 | 538..=539 | 583..=585 | 592..=593 | 595..=614 |
        616..=617 | 626..=627 | 639..=655 | 661 | 669 | 671..=675 | 677 | 688)
}

impl NpcSync {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let slot = r.byte()?;
        let generation = r.byte()?;
        let position = [r.f32()?, r.f32()?];
        let velocity = [r.f32()?, r.f32()?];
        let target = r.u16()?;
        let flags = r.byte()?;
        let extra = r.byte()?;
        let mut ai = [None; 4];
        for (index, value) in ai.iter_mut().enumerate() {
            if flags & (4 << index) != 0 {
                *value = Some(r.f32()?);
            }
        }
        let kind = r.i16()?;
        let scaled_players = (extra & 1 != 0).then(|| r.byte()).transpose()?;
        let difficulty = (extra & 4 != 0).then(|| r.f32()).transpose()?;
        let life = if flags & 128 != 0 {
            NpcLife::Full
        } else {
            match r.byte()? {
                1 => NpcLife::Byte(r.byte()? as i8),
                2 => NpcLife::Short(r.i16()?),
                4 => NpcLife::Int(r.i32()?),
                _ => return Err(Error::Invalid("NPC life width")),
            }
        };
        let npc = Self {
            slot,
            generation,
            position,
            velocity,
            target,
            kind,
            right: flags & 1 != 0,
            down: flags & 2 != 0,
            sprite_right: flags & 64 != 0,
            ai,
            scaled_players,
            difficulty,
            statue: extra & 2 != 0,
            spawning: extra & 8 != 0,
            shimmer: extra & 16 != 0,
            life,
            release_owner: catchable(kind).then(|| r.byte()).transpose()?,
        };
        npc.validate()?;
        Ok(npc)
    }
    fn validate(&self) -> Result<(), Error> {
        if self.slot >= 200
            || self.release_owner.is_some() != catchable(self.kind)
            || !self
                .position
                .into_iter()
                .chain(self.velocity)
                .chain(self.ai.into_iter().flatten())
                .chain(self.difficulty)
                .all(f32::is_finite)
        {
            return Err(Error::Invalid("NPC sync fields"));
        }
        Ok(())
    }
    pub(crate) fn write(&self, w: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        w.put_u8(self.slot);
        w.put_u8(self.generation);
        for v in self.position.into_iter().chain(self.velocity) {
            w.put_f32_le(v);
        }
        w.put_u16_le(self.target);
        let mut flags = u8::from(self.right)
            | u8::from(self.down) << 1
            | u8::from(self.sprite_right) << 6
            | u8::from(self.life == NpcLife::Full) << 7;
        for (index, value) in self.ai.iter().enumerate() {
            if value.is_some() {
                flags |= 4 << index;
            }
        }
        w.put_u8(flags);
        w.put_u8(
            u8::from(self.scaled_players.is_some())
                | u8::from(self.statue) << 1
                | u8::from(self.difficulty.is_some()) << 2
                | u8::from(self.spawning) << 3
                | u8::from(self.shimmer) << 4,
        );
        for v in self.ai.into_iter().flatten() {
            w.put_f32_le(v);
        }
        w.put_i16_le(self.kind);
        if let Some(v) = self.scaled_players {
            w.put_u8(v);
        }
        if let Some(v) = self.difficulty {
            w.put_f32_le(v);
        }
        match self.life {
            NpcLife::Full => {}
            NpcLife::Byte(v) => {
                w.put_u8(1);
                w.put_i8(v);
            }
            NpcLife::Short(v) => {
                w.put_u8(2);
                w.put_i16_le(v);
            }
            NpcLife::Int(v) => {
                w.put_u8(4);
                w.put_i32_le(v);
            }
        }
        if let Some(v) = self.release_owner {
            w.put_u8(v);
        }
        Ok(())
    }
}
