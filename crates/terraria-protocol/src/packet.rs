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
    TilePatch(Box<crate::TilePatch>),
    DoorToggle {
        action: u8,
        position: [i16; 2],
        facing_right: bool,
    },
    ObjectPlacement {
        position: [i16; 2],
        tile: i16,
        style: i16,
        alternate: u8,
        random: i8,
        facing_right: bool,
    },
    TileChange {
        action: u8,
        position: [i16; 2],
        value: i16,
        style: u8,
    },
    DroppedItem(crate::DroppedItem),
    ItemOwner(crate::ItemOwner),
    ReleaseItem {
        slot: i16,
        force: bool,
    },
    RemoveItem(i16),
    NpcSync(crate::NpcSync),
    NpcBuffs {
        slot: i16,
        buffs: Vec<(u16, u16)>,
    },
    RequestNpcName(i16),
    UseTimeDial {
        player: u8,
        dusk: bool,
    },
    NpcName {
        slot: i16,
        name: String,
        variation: i32,
    },
    NpcHome {
        slot: i16,
        position: [i16; 2],
        state: u8,
    },
    RequestChest([i16; 2]),
    ChestUpdate {
        action: u8,
        position: [i16; 2],
        style: i16,
        chest: i16,
    },
    RequestChestName {
        chest: i16,
        position: [i16; 2],
    },
    ChestName {
        chest: i16,
        position: [i16; 2],
        name: String,
    },
    ChestState {
        chest: i16,
        position: [i16; 2],
        name: Option<String>,
    },
    PlayerChest {
        player: u8,
        chest: i16,
    },
    Spawn(Spawn),
    PlayerDeath(crate::PlayerDeath),
    SetDeadAppearance(u8),
    Movement(crate::Movement),
    PlayerZones {
        player: u8,
        zones: [u8; 5],
        town_npcs: u8,
    },
    PlayerTeam {
        player: u8,
        team: u8,
    },
    PlayerPvp {
        player: u8,
        hostile: bool,
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
    ServerInfo(crate::ServerInfo),
    PlayerPlatform(crate::PlayerPlatform),
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
            Self::TileChange { .. } => 17,
            Self::TilePatch(_) => 20,
            Self::DoorToggle { .. } => 19,
            Self::ObjectPlacement { .. } => 79,
            Self::DroppedItem(_) => 21,
            Self::ItemOwner(_) => 22,
            Self::ReleaseItem { .. } => 39,
            Self::RemoveItem(_) => 151,
            Self::NpcSync(_) => 23,
            Self::NpcBuffs { .. } => 54,
            Self::RequestNpcName(_) | Self::NpcName { .. } => 56,
            Self::UseTimeDial { .. } => 51,
            Self::NpcHome { .. } => 60,
            Self::RequestChest(_) => 31,
            Self::ChestUpdate { .. } => 34,
            Self::RequestChestName { .. } | Self::ChestName { .. } => 69,
            Self::ChestState { .. } => 33,
            Self::PlayerChest { .. } => 80,
            Self::ChestSize { .. } => 155,
            Self::ChestItem { .. } => 32,
            Self::Spawn(_) => 12,
            Self::PlayerDeath(_) => 118,
            Self::SetDeadAppearance(_) => 135,
            Self::Movement(_) => 13,
            Self::PlayerZones { .. } => 36,
            Self::PlayerActive { .. } => 14,
            Self::PlayerTeam { .. } => 45,
            Self::PlayerPvp { .. } => 30,
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
            Self::ServerInfo(_) => 163,
            Self::PlayerPlatform(_) => 164,
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
            79 => Self::ObjectPlacement {
                position: r.i16s()?,
                tile: r.i16()?,
                style: r.i16()?,
                alternate: r.byte()?,
                random: r.byte()? as i8,
                facing_right: r.byte()? != 0,
            },
            19 => Self::DoorToggle {
                action: r.byte()?,
                position: r.i16s()?,
                facing_right: r.byte()? != 0,
            },
            20 if !from_client => Self::TilePatch(Box::new(crate::TilePatch::read(&mut r)?)),
            17 => Self::TileChange {
                action: r.byte()?,
                position: r.i16s()?,
                value: r.i16()?,
                style: r.byte()?,
            },
            21 => Self::DroppedItem(crate::DroppedItem::read(&mut r)?),
            22 => Self::ItemOwner(crate::ItemOwner::read(&mut r)?),
            39 => Self::ReleaseItem {
                slot: r.i16()?,
                force: r.byte()? != 0,
            },
            151 => Self::RemoveItem(r.i16()?),
            31 => Self::RequestChest(r.i16s()?),
            33 => {
                let chest = r.i16()?;
                let position = r.i16s()?;
                let name = match r.byte()? {
                    0 => None,
                    255 => Some(String::new()),
                    length @ 1..=20 => {
                        let name = r.string()?;
                        if name.encode_utf16().count() != usize::from(length) {
                            return Err(Error::Invalid("chest name length"));
                        }
                        Some(name.to_owned())
                    }
                    _ => return Err(Error::Invalid("chest name length")),
                };
                Self::ChestState {
                    chest,
                    position,
                    name,
                }
            }
            34 => Self::ChestUpdate {
                action: r.byte()?,
                position: r.i16s()?,
                style: r.i16()?,
                chest: r.i16()?,
            },
            69 => {
                let chest = r.i16()?;
                let position = r.i16s()?;
                if from_client {
                    // The stock serializer may append its cached name. Requests only look it up.
                    if !r.0.is_empty() {
                        r.string()?;
                    }
                    Self::RequestChestName { chest, position }
                } else {
                    Self::ChestName {
                        chest,
                        position,
                        name: r.string()?.to_owned(),
                    }
                }
            }
            23 if !from_client => Self::NpcSync(crate::NpcSync::read(&mut r)?),
            54 if !from_client => {
                let slot = r.i16()?;
                let mut buffs = Vec::new();
                loop {
                    let kind = r.u16()?;
                    if kind == 0 {
                        break;
                    }
                    if buffs.len() == 20 {
                        return Err(Error::Invalid("NPC buff count"));
                    }
                    buffs.push((kind, r.u16()?));
                }
                Self::NpcBuffs { slot, buffs }
            }
            51 => Self::UseTimeDial {
                player: r.byte()?,
                dusk: match r.byte()? {
                    3 => false,
                    6 => true,
                    _ => return Err(Error::Unknown(51)),
                },
            },
            56 if from_client => Self::RequestNpcName(r.i16()?),
            56 => Self::NpcName {
                slot: r.i16()?,
                name: r.string()?.to_owned(),
                variation: r.i32()?,
            },
            60 => Self::NpcHome {
                slot: r.i16()?,
                position: r.i16s()?,
                state: r.byte()?,
            },
            80 => Self::PlayerChest {
                player: r.byte()?,
                chest: r.i16()?,
            },
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
            118 => Self::PlayerDeath(crate::PlayerDeath::read(&mut r)?),
            135 => Self::SetDeadAppearance(r.byte()?),
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
            30 => Self::PlayerPvp {
                player: r.byte()?,
                hostile: r.byte()? != 0,
            },
            45 => {
                let player = r.byte()?;
                let team = r.byte()?;
                if team > 5 {
                    return Err(Error::Invalid("player team"));
                }
                Self::PlayerTeam { player, team }
            }
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
            163 if !from_client => Self::ServerInfo(crate::ServerInfo::read(&mut r)?),
            164 => Self::PlayerPlatform(crate::PlayerPlatform::read(&mut r)?),
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
            Self::ServerInfo(info) => info.write(&mut out)?,
            Self::PlayerPlatform(platform) => platform.write(&mut out)?,
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
            Self::RequestChestName { chest, position }
            | Self::ChestName {
                chest, position, ..
            } => {
                out.put_i16_le(*chest);
                for c in position {
                    out.put_i16_le(*c);
                }
                if let Self::ChestName { name, .. } = self {
                    put_string(&mut out, name)?;
                }
            }
            Self::ChestUpdate {
                action,
                position,
                style,
                chest,
            } => {
                out.put_u8(*action);
                for c in position {
                    out.put_i16_le(*c);
                }
                out.put_i16_le(*style);
                out.put_i16_le(*chest);
            }
            Self::NpcSync(npc) => npc.write(&mut out)?,
            Self::NpcBuffs { slot, buffs } => {
                if buffs.len() > 20 || buffs.iter().any(|(kind, _)| *kind == 0) {
                    return Err(Error::Invalid("NPC buffs"));
                }
                out.put_i16_le(*slot);
                for &(kind, time) in buffs {
                    out.put_u16_le(kind);
                    out.put_u16_le(time);
                }
                out.put_u16_le(0);
            }
            Self::RequestNpcName(slot) => out.put_i16_le(*slot),
            Self::UseTimeDial { player, dusk } => {
                out.put_u8(*player);
                out.put_u8(if *dusk { 6 } else { 3 });
            }
            Self::NpcName {
                slot,
                name,
                variation,
            } => {
                out.put_i16_le(*slot);
                put_string(&mut out, name)?;
                out.put_i32_le(*variation);
            }
            Self::NpcHome {
                slot,
                position,
                state,
            } => {
                out.put_i16_le(*slot);
                for v in position {
                    out.put_i16_le(*v);
                }
                out.put_u8(*state);
            }
            Self::TileSection(section) => section.write(&mut out)?,
            Self::TilePatch(patch) => patch.write(&mut out)?,
            Self::DoorToggle {
                action,
                position,
                facing_right,
            } => {
                out.put_u8(*action);
                for coord in position {
                    out.put_i16_le(*coord);
                }
                out.put_u8((*facing_right).into());
            }
            Self::ObjectPlacement {
                position,
                tile,
                style,
                alternate,
                random,
                facing_right,
            } => {
                for value in position {
                    out.put_i16_le(*value);
                }
                out.put_i16_le(*tile);
                out.put_i16_le(*style);
                out.put_u8(*alternate);
                out.put_i8(*random);
                out.put_u8((*facing_right).into());
            }
            Self::TileChange {
                action,
                position,
                value,
                style,
            } => {
                out.put_u8(*action);
                for n in position {
                    out.put_i16_le(*n);
                }
                out.put_i16_le(*value);
                out.put_u8(*style);
            }
            Self::DroppedItem(item) => item.write(&mut out),
            Self::ItemOwner(owner) => owner.write(&mut out)?,
            Self::ReleaseItem { slot, force } => {
                out.put_i16_le(*slot);
                out.put_u8((*force).into());
            }
            Self::RemoveItem(slot) => out.put_i16_le(*slot),
            Self::RequestChest(position) => {
                for v in position {
                    out.put_i16_le(*v);
                }
            }
            Self::ChestState {
                chest,
                position,
                name,
            } => {
                out.put_i16_le(*chest);
                for v in position {
                    out.put_i16_le(*v);
                }
                match name {
                    None => out.put_u8(0),
                    Some(s) if s.is_empty() => out.put_u8(255),
                    Some(s) => {
                        let len = s.encode_utf16().count();
                        if len > 20 {
                            return Err(Error::Invalid("chest name length"));
                        }
                        out.put_u8(len as u8);
                        put_string(&mut out, s)?;
                    }
                }
            }
            Self::PlayerChest { player, chest } => {
                out.put_u8(*player);
                out.put_i16_le(*chest);
            }
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
            Self::PlayerTeam { player, team } => {
                if *team > 5 {
                    return Err(Error::Invalid("player team"));
                }
                out.put_u8(*player);
                out.put_u8(*team);
            }
            Self::PlayerPvp { player, hostile } => {
                out.put_u8(*player);
                out.put_u8(u8::from(*hostile));
            }
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
            Self::PlayerDeath(p) => p.write(&mut out)?,
            Self::SetDeadAppearance(player) => out.put_u8(*player),
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
