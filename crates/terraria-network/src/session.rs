use std::collections::BTreeMap;
use terraria_protocol::{Appearance, Equipment, GREETING, NetworkText, Packet, Vitals};

const EQUIPMENT_SLOTS: i16 = 990;

#[derive(Debug, PartialEq, Eq)]
pub enum Phase {
    AwaitHello,
    AwaitPassword,
    UploadingCharacter,
    Closed,
}

/// Connection-owned staging state. Only the listener chooses the player identity.
#[derive(Default)]
pub struct Character {
    pub appearance: Option<Appearance>,
    pub life: Option<Vitals>,
    pub mana: Option<Vitals>,
    pub buffs: Vec<u16>,
    pub loadout: Option<(u8, u16)>,
    pub equipment: BTreeMap<i16, Equipment>,
}

pub struct Session {
    pub(crate) phase: Phase,
    slot: u8,
    character: Character,
}

impl Session {
    pub fn new(slot: u8) -> Self {
        Self {
            phase: Phase::AwaitHello,
            slot,
            character: Character::default(),
        }
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
            Packet::Equipment(mut e)
                if (0..EQUIPMENT_SLOTS).contains(&e.slot)
                    && (0..6196).contains(&e.item)
                    && e.stack >= 0 =>
            {
                e.player = self.slot;
                if e.stack == 0 || e.item == 0 {
                    self.character.equipment.remove(&e.slot);
                } else {
                    self.character.equipment.insert(e.slot, e);
                }
            }
            // GetData consumes the UUID without storing it. A direct server has no host token.
            Packet::ClientUuid(_) | Packet::HostToken(_) => {}
            Packet::RequestWorld => {
                return self.close(NetworkText::Literal(
                    "World synchronization is not implemented on this Rust server.".into(),
                ));
            }
            _ => return self.close(NetworkText::key("LegacyMultiplayer.2")),
        }
        None
    }
}
