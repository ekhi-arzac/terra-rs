use std::collections::BTreeMap;
use terraria_protocol::{Appearance, Equipment, GREETING, NetworkText, Packet, Vitals};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    AwaitHello,
    AwaitPassword,
    UploadingCharacter,
    AwaitSections,
    SendingSections,
    AwaitSpawn,
    Active,
    Closed,
}

/// Connection-owned staging state. Only the listener chooses the player identity.
#[derive(Default)]
pub struct Character {
    pub spawn: Option<terraria_protocol::Spawn>,
    pub appearance: Option<Appearance>,
    pub life: Option<Vitals>,
    pub mana: Option<Vitals>,
    pub buffs: Vec<u16>,
    pub loadout: Option<(u8, u16)>,
    pub equipment: BTreeMap<i16, Equipment>,
}

pub struct Session<'w> {
    world: Option<&'w terraria_protocol::WorldData>,
    pub(crate) phase: Phase,
    slot: u8,
    journey_mode: bool,
    character: Character,
}

impl<'w> Session<'w> {
    pub fn new(slot: u8, journey_mode: bool) -> Self {
        Self {
            world: None,
            phase: Phase::AwaitHello,
            slot,
            journey_mode,
            character: Character::default(),
        }
    }
    pub fn for_world(slot: u8, world: &'w terraria_protocol::WorldData) -> Self {
        Self {
            world: Some(world),
            ..Self::new(slot, world.game_mode == 3)
        }
    }
    pub(crate) fn into_character(self) -> Character {
        self.character
    }
    pub fn phase(&self) -> &Phase {
        &self.phase
    }
    pub fn character(&self) -> &Character {
        &self.character
    }

    fn admit(&mut self) -> Option<Packet> {
        self.phase = Phase::UploadingCharacter;
        Some(Packet::AssignSlot {
            player: self.slot,
            special: false,
        })
    }
    fn close(&mut self, reason: NetworkText) -> Option<Packet> {
        self.phase = Phase::Closed;
        Some(Packet::Kick(reason))
    }

    pub fn handle(&mut self, packet: Packet, password: &str) -> Option<Packet> {
        match (&self.phase, packet) {
            (Phase::Closed, _) => None,
            (Phase::AwaitHello, Packet::Hello(v)) => {
                if v != GREETING {
                    return self.close(NetworkText::key("LegacyMultiplayer.4"));
                }
                if password.is_empty() {
                    return self.admit();
                }
                self.phase = Phase::AwaitPassword;
                Some(Packet::RequestPassword)
            }
            (Phase::AwaitPassword, Packet::Password(p)) if p == password => self.admit(),
            (Phase::AwaitPassword, _) => self.close(NetworkText::key("LegacyMultiplayer.1")),
            (Phase::UploadingCharacter | Phase::AwaitSections, Packet::RequestWorld) => {
                if let Some(world) = self.world {
                    self.phase = Phase::AwaitSections;
                    Some(Packet::WorldData(Box::new(world.clone())))
                } else {
                    self.close(NetworkText::Literal(
                        "No world is configured on this server.".into(),
                    ))
                }
            }
            (Phase::AwaitSections, Packet::RequestSections { .. }) => {
                self.phase = Phase::SendingSections;
                self.world
                    .map(|data| Packet::WorldData(Box::new(data.clone())))
            }
            (Phase::AwaitSpawn, Packet::Spawn(mut spawn))
                if self.character.appearance.is_some() && spawn.team <= 5 =>
            {
                spawn.player = self.slot;
                self.character.spawn = Some(spawn);
                self.phase = Phase::Active;
                Some(Packet::FinishedConnecting)
            }
            (Phase::UploadingCharacter, packet) => self.upload(packet),
            _ => self.close(NetworkText::key("LegacyMultiplayer.2")),
        }
    }

    fn upload(&mut self, packet: Packet) -> Option<Packet> {
        match packet {
            Packet::Appearance(mut a) => {
                a.player = self.slot;
                a.name = a.name.trim().to_owned();
                if a.name.is_empty() {
                    return self.close(NetworkText::key("Net.EmptyName"));
                }
                if a.name.encode_utf16().count() > 20 {
                    return self.close(NetworkText::key("Net.NameTooLong"));
                }
                let journey_character = a.difficulty_flags & 8 != 0;
                if journey_character != self.journey_mode {
                    let key = if journey_character {
                        "Net.PlayerIsCreativeAndWorldIsNotCreative"
                    } else {
                        "Net.PlayerIsNotCreativeAndWorldIsCreative"
                    };
                    return self.close(NetworkText::key(key));
                }
                a.skin = a.skin.min(11);
                a.voice = a.voice.clamp(1, 4);
                a.pitch = if a.pitch.is_nan() {
                    0.0
                } else {
                    a.pitch.clamp(-1.0, 1.0)
                };
                if a.hair >= 228 {
                    a.hair = 0
                }
                self.character.appearance = Some(a);
            }
            Packet::Life(mut v) => {
                v.player = self.slot;
                v.maximum = v.maximum.max(20);
                self.character.life = Some(v);
            }
            Packet::Mana(mut v) => {
                v.player = self.slot;
                self.character.mana = Some(v);
            }
            Packet::Buffs { types, .. } => self.character.buffs = types,
            Packet::Loadout {
                index,
                hidden_accessories,
                ..
            } if index < 3 => {
                self.character.loadout = Some((index, hidden_accessories));
            }
            Packet::Equipment(mut e) if e.valid() => {
                e.player = self.slot;
                if e.stack == 0 || e.item == 0 {
                    self.character.equipment.remove(&e.slot);
                } else {
                    self.character.equipment.insert(e.slot, e);
                }
            }
            // GetData consumes the UUID without storing it. A direct server has no host token.
            Packet::ClientUuid(_) | Packet::HostToken(_) => {}
            _ => return self.close(NetworkText::key("LegacyMultiplayer.2")),
        }
        None
    }
}
