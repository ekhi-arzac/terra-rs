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

impl Equipment {
    pub fn valid(&self) -> bool {
        (0..990).contains(&self.slot) && (0..6196).contains(&self.item) && self.stack >= 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vitals {
    pub player: u8,
    pub current: i16,
    pub maximum: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spawn {
    pub player: u8,
    pub point: [i16; 2],
    pub respawn_timer: i32,
    pub deaths: [i16; 2],
    pub team: u8,
    pub context: u8,
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
    WorldData(Box<crate::WorldData>),
    RequestSections {
        spawn: [i32; 2],
        team: u8,
    },
    Status {
        count: i32,
        text: NetworkText,
        flags: u8,
    },
    TileSection(Box<crate::section::TileSection>),
    ChestSize {
        index: i16,
        capacity: i16,
    },
    ChestItem {
        chest: i16,
        slot: u8,
        stack: i16,
        prefix: u8,
        item: i16,
    },
    Spawn(Spawn),
    Movement(crate::Movement),
    PlayerZones {
        player: u8,
        zones: [u8; 5],
        town_npcs: u8,
    },
    PlayerActive {
        player: u8,
        active: bool,
    },
    ChatCommand {
        command: String,
        text: String,
    },
    ChatMessage {
        author: u8,
        text: NetworkText,
        color: [u8; 3],
    },
    BannerState(Box<crate::BannerState>),
    TowerShields([u16; 4]),
    CavernMonsters([u16; 6]),
    Ping,
    SpawnAllowed,
    RequestSection([u16; 2]),
    FinishedConnecting,
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
            Self::WorldData(_) => 7,
            Self::RequestSections { .. } => 8,
            Self::Status { .. } => 9,
            Self::TileSection(_) => 10,
            Self::ChestSize { .. } => 155,
            Self::ChestItem { .. } => 32,
            Self::Spawn(_) => 12,
            Self::Movement(_) => 13,
            Self::PlayerZones { .. } => 36,
            Self::PlayerActive { .. } => 14,
            Self::ChatCommand { .. } | Self::ChatMessage { .. } => 82,
            Self::BannerState(_) => 82,
            Self::TowerShields(_) => 101,
            Self::CavernMonsters(_) => 136,
            Self::Ping => 154,
            Self::SpawnAllowed => 49,
            Self::RequestSection(_) => 159,
            Self::FinishedConnecting => 129,
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

    /// Decode a server-to-client frame. Text module 1 has direction-dependent payloads.
    pub fn decode(frame: &Frame) -> Result<Self, Error> {
        Self::decode_direction(frame, false)
    }
    pub fn decode_client(frame: &Frame) -> Result<Self, Error> {
        Self::decode_direction(frame, true)
    }
    fn decode_direction(frame: &Frame, from_client: bool) -> Result<Self, Error> {
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
            7 => Self::WorldData(Box::new(crate::WorldData::read(&mut r)?)),
            8 => Self::RequestSections {
                spawn: r.i32s()?,
                team: r.byte()?,
            },
            9 => Self::Status {
                count: r.i32()?,
                text: NetworkText::read(&mut r, 0)?,
                flags: r.byte()?,
            },
            10 => {
                let data = r.bytes(r.0.len())?;
                Self::TileSection(Box::new(crate::section::TileSection::read(data)?))
            }
            155 => Self::ChestSize {
                index: r.i16()?,
                capacity: r.i16()?,
            },
            32 => Self::ChestItem {
                chest: r.i16()?,
                slot: r.byte()?,
                stack: r.i16()?,
                prefix: r.byte()?,
                item: r.i16()?,
            },
            12 => Self::Spawn(Spawn {
                player: r.byte()?,
                point: r.i16s()?,
                respawn_timer: r.i32()?,
                deaths: r.i16s()?,
                team: r.byte()?,
                context: r.byte()?,
            }),
            36 => Self::PlayerZones {
                player: r.byte()?,
                zones: r.take()?,
                town_npcs: r.byte()?,
            },
            13 => Self::Movement(crate::Movement::read(&mut r)?),
            14 => Self::PlayerActive {
                player: r.byte()?,
                active: r.byte()? != 0,
            },
            82 => match (r.u16()?, from_client) {
                (1, true) => Self::ChatCommand {
                    command: r.string()?.to_owned(),
                    text: r.string()?.to_owned(),
                },
                (1, false) => Self::ChatMessage {
                    author: r.byte()?,
                    text: NetworkText::read(&mut r, 0)?,
                    color: r.take()?,
                },
                (11, false) => Self::BannerState(Box::new(crate::BannerState::read(&mut r)?)),
                _ => return Err(Error::Invalid("unsupported net module")),
            },
            101 => Self::TowerShields(r.i16s::<4>()?.map(|v| v as u16)),
            136 => Self::CavernMonsters(r.i16s::<6>()?.map(|v| v as u16)),
            154 => Self::Ping,
            49 => Self::SpawnAllowed,
            159 => Self::RequestSection([r.u16()?, r.u16()?]),
            129 => Self::FinishedConnecting,
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
            Self::Status { count, text, flags } => {
                out.put_i32_le(*count);
                text.write(&mut out, 0)?;
                out.put_u8(*flags);
            }
            Self::TileSection(section) => section.write(&mut out)?,
            Self::ChestSize { index, capacity } => {
                out.put_i16_le(*index);
                out.put_i16_le(*capacity);
            }
            Self::ChestItem {
                chest,
                slot,
                stack,
                prefix,
                item,
            } => {
                out.put_i16_le(*chest);
                out.put_u8(*slot);
                out.put_i16_le(*stack);
                out.put_u8(*prefix);
                out.put_i16_le(*item);
            }
            Self::WorldData(data) => data.write(&mut out)?,
            Self::RequestSections { spawn, team } => {
                for v in spawn {
                    out.put_i32_le(*v);
                }
                out.put_u8(*team);
            }
            Self::BannerState(s) => {
                out.put_u16_le(11);
                s.write(&mut out);
            }
            Self::TowerShields(a) => {
                for &n in a {
                    out.put_u16_le(n);
                }
            }
            Self::CavernMonsters(a) => {
                for &n in a {
                    out.put_u16_le(n);
                }
            }
            Self::PlayerZones {
                player,
                zones,
                town_npcs,
            } => {
                out.put_u8(*player);
                out.extend_from_slice(zones);
                out.put_u8(*town_npcs);
            }
            Self::Ping => {}
            Self::Movement(m) => m.write(&mut out)?,
            Self::PlayerActive { player, active } => {
                out.extend_from_slice(&[*player, u8::from(*active)])
            }
            Self::ChatCommand { command, text } => {
                out.put_u16_le(1);
                put_string(&mut out, command)?;
                put_string(&mut out, text)?;
            }
            Self::ChatMessage {
                author,
                text,
                color,
            } => {
                out.put_u16_le(1);
                out.put_u8(*author);
                text.write(&mut out, 0)?;
                out.extend_from_slice(color);
            }
            Self::RequestSection(index) => {
                for n in index {
                    out.put_u16_le(*n)
                }
            }
            Self::Spawn(s) => {
                out.put_u8(s.player);
                for v in s.point {
                    out.put_i16_le(v)
                }
                out.put_i32_le(s.respawn_timer);
                for v in s.deaths {
                    out.put_i16_le(v)
                }
                out.extend_from_slice(&[s.team, s.context]);
            }
            Self::RequestWorld
            | Self::RequestPassword
            | Self::SpawnAllowed
            | Self::FinishedConnecting => {}
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
