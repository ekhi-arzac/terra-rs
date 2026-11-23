use crate::{
    Frame, NetworkText,
    wire::{Error, MAX_FRAME, Reader, put_string},
};
use bytes::{BufMut, Bytes, BytesMut};

pub const MAX_BUFFS: usize = 44;

#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub player: u8,
    pub skin: u8,
    pub voice: u8,
    pub pitch: f32,
    pub hair: u8,
    pub name: String,
    pub hair_dye: u8,
    pub hidden_accessories: u16,
    pub hidden_misc: u8,
    /// Hair, skin, eyes, shirt, undershirt, trousers, shoes.
    pub colors: [[u8; 3]; 7],
    pub difficulty_flags: u8,
    pub torch_cart_flags: u8,
    pub permanent_upgrades: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    pub player: u8,
    pub slot: i16,
    pub stack: i16,
    pub prefix: u8,
    pub item: i16,
    pub flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vitals {
    pub player: u8,
    pub current: i16,
    pub maximum: i16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    Hello(String),
    Kick(NetworkText),
    AssignSlot {
        player: u8,
        special: bool,
    },
    Appearance(Appearance),
    Equipment(Equipment),
    RequestWorld,
    Life(Vitals),
    RequestPassword,
    Password(String),
    Mana(Vitals),
    Buffs {
        player: u8,
        types: Vec<u16>,
    },
    ClientUuid(String),
    Loadout {
        player: u8,
        index: u8,
        hidden_accessories: u16,
    },
    HostToken(String),
}

impl Packet {
    pub fn id(&self) -> u8 {
        match self {
            Self::Hello(_) => 1,
            Self::Kick(_) => 2,
            Self::AssignSlot { .. } => 3,
            Self::Appearance(_) => 4,
            Self::Equipment(_) => 5,
            Self::RequestWorld => 6,
            Self::Life(_) => 16,
            Self::RequestPassword => 37,
            Self::Password(_) => 38,
            Self::Mana(_) => 42,
            Self::Buffs { .. } => 50,
            Self::ClientUuid(_) => 68,
            Self::Loadout { .. } => 147,
            Self::HostToken(_) => 161,
        }
    }

    pub fn decode(frame: &Frame) -> Result<Self, Error> {
        let size = frame.payload.len().saturating_add(3);
        if size > MAX_FRAME {
            return Err(Error::Length(size));
        }
        let mut r = Reader(&frame.payload);
        let p = match frame.id {
            1 => Self::Hello(r.string()?.to_owned()),
            2 => Self::Kick(NetworkText::read(&mut r, 0)?),
            3 => Self::AssignSlot {
                player: r.byte()?,
                special: r.byte()? != 0,
            },
            4 => {
                let player = r.byte()?;
                let skin = r.byte()?;
                let voice = r.byte()?;
                let pitch = f32::from_le_bytes(r.take()?);
                let hair = r.byte()?;
                let name = r.string()?.to_owned();
                let hair_dye = r.byte()?;
                let hidden_accessories = r.u16()?;
                let hidden_misc = r.byte()?;
                let mut colors = [[0; 3]; 7];
                for rgb in &mut colors {
                    *rgb = r.take()?;
                }
                Self::Appearance(Appearance {
                    player,
                    skin,
                    voice,
                    pitch,
                    hair,
                    name,
                    hair_dye,
                    hidden_accessories,
                    hidden_misc,
                    colors,
                    difficulty_flags: r.byte()?,
                    torch_cart_flags: r.byte()?,
                    permanent_upgrades: r.byte()?,
                })
            }
            5 => Self::Equipment(Equipment {
                player: r.byte()?,
                slot: r.i16()?,
                stack: r.i16()?,
                prefix: r.byte()?,
                item: r.i16()?,
                flags: r.byte()?,
            }),
            6 => Self::RequestWorld,
            id @ (16 | 42) => {
                let v = Vitals {
                    player: r.byte()?,
                    current: r.i16()?,
                    maximum: r.i16()?,
                };
                if id == 16 {
                    Self::Life(v)
                } else {
                    Self::Mana(v)
                }
            }
            37 => Self::RequestPassword,
            38 => Self::Password(r.string()?.to_owned()),
            50 => {
                let player = r.byte()?;
                let mut types = Vec::new();
                loop {
                    let ty = r.u16()?;
                    if ty == 0 {
                        break;
                    }
                    if types.len() == MAX_BUFFS {
                        return Err(Error::Invalid("buff count"));
                    }
                    types.push(ty);
                }
                Self::Buffs { player, types }
            }
            68 => Self::ClientUuid(r.string()?.to_owned()),
            147 => Self::Loadout {
                player: r.byte()?,
                index: r.byte()?,
                hidden_accessories: r.u16()?,
            },
            161 => Self::HostToken(r.string()?.to_owned()),
            id => return Err(Error::Unknown(id)),
        };
        if !r.0.is_empty() {
            return Err(Error::Trailing(r.0.len()));
        }
        Ok(p)
    }

    /// Encode one complete TCP frame, including its two-byte size.
    pub fn encode(&self) -> Result<Bytes, Error> {
        let mut out = BytesMut::with_capacity(64);
        out.put_u16_le(0);
        out.put_u8(self.id());
        match self {
            Self::Hello(s) | Self::ClientUuid(s) | Self::Password(s) | Self::HostToken(s) => {
                put_string(&mut out, s)?
            }
            Self::Kick(t) => t.write(&mut out, 0)?,
            Self::AssignSlot { player, special } => {
                out.extend_from_slice(&[*player, u8::from(*special)])
            }
            Self::Appearance(a) => {
                out.extend_from_slice(&[a.player, a.skin, a.voice]);
                out.put_f32_le(a.pitch);
                out.put_u8(a.hair);
                put_string(&mut out, &a.name)?;
                out.put_u8(a.hair_dye);
                out.put_u16_le(a.hidden_accessories);
                out.put_u8(a.hidden_misc);
                for color in a.colors {
                    out.extend_from_slice(&color);
                }
                out.extend_from_slice(&[
                    a.difficulty_flags,
                    a.torch_cart_flags,
                    a.permanent_upgrades,
                ]);
            }
            Self::Equipment(e) => {
                out.put_u8(e.player);
                out.put_i16_le(e.slot);
                out.put_i16_le(e.stack);
                out.put_u8(e.prefix);
                out.put_i16_le(e.item);
                out.put_u8(e.flags);
            }
            Self::RequestWorld | Self::RequestPassword => {}
            Self::Life(v) | Self::Mana(v) => {
                out.put_u8(v.player);
                out.put_i16_le(v.current);
                out.put_i16_le(v.maximum);
            }
            Self::Buffs { player, types } => {
                if types.len() > MAX_BUFFS || types.contains(&0) {
                    return Err(Error::Invalid("buff list"));
                }
                out.put_u8(*player);
                for &ty in types {
                    out.put_u16_le(ty);
                }
                out.put_u16_le(0);
            }
            Self::Loadout {
                player,
                index,
                hidden_accessories,
            } => {
                out.extend_from_slice(&[*player, *index]);
                out.put_u16_le(*hidden_accessories);
            }
        }
        let size = u16::try_from(out.len()).map_err(|_| Error::Length(out.len()))?;
        out[..2].copy_from_slice(&size.to_le_bytes());
        Ok(out.freeze())
    }
}
