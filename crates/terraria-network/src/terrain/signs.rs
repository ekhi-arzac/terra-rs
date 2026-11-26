//! Session-local sign slots, shared text and section snapshots.
use super::{LoadedWorld, WorldEdits};
use bytes::BytesMut;
use std::collections::{BTreeMap, BTreeSet};
use terraria_protocol::{Error, Packet, section::SectionObject};
use terraria_world::{Sign, World};

const MAX_SIGNS: i16 = 32000;
// Bound edits replicated into the compressed section as well as the direct response.
const MAX_TEXT_BYTES: usize = 4096;

struct LiveSign {
    object: SectionObject,
    revision: u64,
}

impl LiveSign {
    fn packet(&self, player: u8, flags: u8) -> Packet {
        Packet::SignText {
            index: self.object.index,
            position: self.object.position,
            text: self.object.text.clone(),
            player,
            flags,
        }
    }
}

#[derive(Default)]
pub(super) struct Signs {
    sections: BTreeMap<[u16; 2], Vec<LiveSign>>,
    count: i16,
}

impl Signs {
    pub(super) fn load(world: &World) -> Self {
        let mut signs = Self::default();
        for sign in world.signs() {
            let index = signs.count;
            signs.count += 1;
            if let (Ok(x), Ok(y)) = (
                i16::try_from(sign.position.0),
                i16::try_from(sign.position.1),
            ) && (0..world.metadata().width as i16).contains(&x)
                && (0..world.metadata().height as i16).contains(&y)
            {
                signs
                    .sections
                    .entry(LoadedWorld::section_at([x, y]))
                    .or_default()
                    .push(LiveSign {
                        object: SectionObject {
                            index,
                            position: [x, y],
                            text: sign.text.clone(),
                        },
                        revision: 0,
                    });
            }
        }
        signs
    }

    fn get_or_create(&mut self, position: [i16; 2], revision: u64) -> Option<&LiveSign> {
        let section = self
            .sections
            .entry(LoadedWorld::section_at(position))
            .or_default();
        if let Some(index) = section
            .iter()
            .position(|sign| sign.object.position == position)
        {
            return Some(&section[index]);
        }
        if self.count == MAX_SIGNS {
            return None;
        }
        section.push(LiveSign {
            object: SectionObject {
                index: self.count,
                position,
                text: String::new(),
            },
            revision,
        });
        self.count += 1;
        section.last()
    }

    pub(super) fn snapshot(&self, index: [u16; 2]) -> Vec<SectionObject> {
        let mut objects: Vec<_> = self
            .sections
            .get(&index)
            .into_iter()
            .flatten()
            .map(|sign| sign.object.clone())
            .collect();
        objects.sort_by_key(|sign| (sign.position[1], sign.position[0]));
        objects
    }

    pub(super) fn catch_up(
        &self,
        since: u64,
        sent: &BTreeSet<[u16; 2]>,
        out: &mut BytesMut,
    ) -> Result<(), Error> {
        for sign in sent
            .iter()
            .filter_map(|index| self.sections.get(index))
            .flatten()
            .filter(|sign| sign.revision > since)
        {
            out.extend_from_slice(&sign.packet(255, 1).encode()?);
        }
        Ok(())
    }

    pub(super) fn changes(&self, since: u64) -> BTreeMap<i16, Sign> {
        self.sections
            .values()
            .flatten()
            .filter(|sign| sign.revision > since)
            .map(|sign| {
                let [x, y] = sign.object.position.map(i32::from);
                (
                    sign.object.index,
                    Sign {
                        position: (x, y),
                        text: sign.object.text.clone(),
                    },
                )
            })
            .collect()
    }
}

impl LoadedWorld {
    fn sign_anchor(&self, [x, y]: [i16; 2], edits: &WorldEdits) -> Option<[i16; 2]> {
        let tile = self.tile_at([x, y], edits)?;
        let kind = tile.block?;
        if !matches!(kind, 55 | 85 | 425 | 573) {
            return None;
        }
        let (frame_x, frame_y) = tile.frame?;
        let anchor = [
            x.checked_sub((frame_x / 18) % 2)?,
            y.checked_sub(frame_y / 18)?,
        ];
        let top = self.tile_at(anchor, edits)?;
        (top.block == Some(kind) && top.frame.is_some_and(|(fx, fy)| fx % 36 == 0 && fy == 0))
            .then_some(anchor)
    }

    pub(crate) fn open_sign(&self, clicked: [i16; 2], slot: u8) -> Option<Packet> {
        let mut edits = self.edits.write().unwrap();
        let position = self.sign_anchor(clicked, &edits)?;
        let revision = edits.revision + 1;
        let sign = edits.signs.get_or_create(position, revision)?;
        let created = sign.revision == revision;
        let packet = sign.packet(slot, 0);
        if created {
            edits.revision = revision;
            edits.content_revision = revision;
        }
        Some(packet)
    }

    pub(crate) fn edit_sign(
        &self,
        position: [i16; 2],
        index: i16,
        text: String,
        slot: u8,
    ) -> Option<(Packet, bool)> {
        let mut edits = self.edits.write().unwrap();
        if self.sign_anchor(position, &edits)? != position {
            return None;
        }
        let revision = edits.revision + 1;
        let sign = edits
            .signs
            .sections
            .get_mut(&Self::section_at(position))?
            .iter_mut()
            .find(|sign| sign.object.position == position)?;
        if sign.object.index != index || text.len() > MAX_TEXT_BYTES {
            return Some((sign.packet(slot, 1), false));
        }
        if sign.object.text == text {
            return None;
        }
        sign.object.text = text;
        sign.revision = revision;
        let packet = sign.packet(slot, 0);
        edits.revision = revision;
        edits.content_revision = revision;
        Some((packet, true))
    }
}
