//! World item slots live for the lifetime of the simulation, independently of player slots.
use bytes::{Bytes, BytesMut};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use terraria_protocol::{DroppedItem, Error, ItemOwner, NEW_ITEM, Packet};

const UNOWNED: u8 = 255;
const GRAB_DELAY: u32 = 100;
fn ticks_left(end: Instant, now: Instant) -> u32 {
    (end.saturating_duration_since(now).as_secs_f64() * 60.0).ceil() as u32
}
fn deadline(now: Instant, ticks: u32) -> Instant {
    now + Duration::from_secs_f64(f64::from(ticks) / 60.0)
}

struct Entry {
    item: DroppedItem,
    player: u8,
    reserved_until: Instant,
    delayed_player: u8,
    delayed_until: Instant,
    created: Instant,
    release_requested: bool,
}
impl Entry {
    fn owner(&self, now: Instant) -> Packet {
        Packet::ItemOwner(ItemOwner {
            slot: self.item.slot,
            player: self.player,
            reservation: ticks_left(self.reserved_until, now),
            delayed_player: self.delayed_player,
            delay: ticks_left(self.delayed_until, now),
            position: self.item.position,
        })
    }
}
#[derive(Default)]
pub(super) struct Drops {
    active: BTreeMap<i16, Entry>,
    retired: BTreeMap<i16, Instant>,
}
impl Drops {
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
    pub fn snapshot(&self, batch: &mut BytesMut) -> Result<(), Error> {
        let now = Instant::now();
        for e in self.active.values() {
            batch.extend_from_slice(&Packet::DroppedItem(e.item).encode()?);
            batch.extend_from_slice(&e.owner(now).encode()?);
        }
        Ok(())
    }
    pub fn correction(&self, slot: i16) -> Result<Bytes, Error> {
        let Some(entry) = self.active.get(&slot) else {
            return Packet::RemoveItem(slot).encode();
        };
        let mut bytes = BytesMut::new();
        bytes.extend_from_slice(&Packet::DroppedItem(entry.item).encode()?);
        bytes.extend_from_slice(&entry.owner(Instant::now()).encode()?);
        Ok(bytes.freeze())
    }

    pub fn apply(&mut self, player: u8, mut item: DroppedItem) -> Option<Packet> {
        let now = Instant::now();
        if item.slot == NEW_ITEM {
            if item.item == 0 || item.stack == 0 {
                return None;
            }
            self.retired.retain(|_, until| *until > now);
            item.slot = (0..NEW_ITEM)
                .find(|s| !self.active.contains_key(s) && !self.retired.contains_key(s))
                .or_else(|| {
                    self.active
                        .values()
                        .min_by_key(|e| e.created)
                        .map(|e| e.item.slot)
                })?;
            let mode = item.ownership;
            item.ownership = 0;
            self.active.insert(
                item.slot,
                Entry {
                    item,
                    player: if mode == 1 { player } else { UNOWNED },
                    reserved_until: deadline(now, if mode == 1 { GRAB_DELAY } else { 0 }),
                    delayed_player: if mode == 2 { player } else { UNOWNED },
                    delayed_until: deadline(now, if mode >= 2 { GRAB_DELAY } else { 0 }),
                    created: now,
                    release_requested: false,
                },
            );
        } else {
            let e = self.active.get_mut(&item.slot)?;
            if e.player != player {
                return None;
            }
            if item.item == 0 || item.stack == 0 {
                return self.remove(player, item.slot);
            }
            item.ownership = 0;
            e.item = item;
        }
        Some(Packet::DroppedItem(item))
    }
    pub fn remove(&mut self, player: u8, slot: i16) -> Option<Packet> {
        if self.active.get(&slot)?.player != player {
            return None;
        }
        self.active.remove(&slot);
        self.retired.insert(slot, deadline(Instant::now(), 15));
        Some(Packet::RemoveItem(slot))
    }
    pub fn release(&mut self, player: u8, slot: i16, force: bool) -> Option<Packet> {
        let e = self.active.get_mut(&slot)?;
        if e.player != player {
            return None;
        }
        let now = Instant::now();
        e.player = UNOWNED;
        e.reserved_until = now;
        e.release_requested = false;
        if force {
            e.delayed_until = deadline(now, 15);
            e.delayed_player = player;
        }
        Some(e.owner(now))
    }
    pub fn owner(&self, slot: i16) -> Option<Packet> {
        Some(self.active.get(&slot)?.owner(Instant::now()))
    }
    pub fn refresh(&mut self, players: &BTreeMap<u8, [f32; 2]>) -> Vec<(Option<u8>, Packet)> {
        let now = Instant::now();
        let mut changes = Vec::new();
        for e in self.active.values_mut() {
            if players.contains_key(&e.player) && now < e.reserved_until {
                continue;
            }
            let delayed = now < e.delayed_until;
            let [x, y] = e.item.position;
            let nearest = players
                .iter()
                .filter(|(id, _)| {
                    !delayed || (e.delayed_player != UNOWNED && **id != e.delayed_player)
                })
                .map(|(&id, p)| (id, (p[0] - x).abs() + (p[1] - y).abs()))
                .filter(|(_, d)| *d < 1920.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(UNOWNED, |(id, _)| id);
            if nearest != e.player {
                if players.contains_key(&e.player) {
                    if !e.release_requested {
                        changes.push((
                            Some(e.player),
                            Packet::ReleaseItem {
                                slot: e.item.slot,
                                force: false,
                            },
                        ));
                        e.release_requested = true;
                    }
                    continue;
                }
                e.release_requested = false;
                e.player = nearest;
                e.reserved_until = deadline(now, 15);
                changes.push((None, e.owner(now)));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_waits_for_owner_and_only_one_player_can_remove_the_stack() {
        let mut drops = Drops::default();
        let request = DroppedItem {
            slot: NEW_ITEM,
            position: [0.; 2],
            velocity: [0.; 2],
            item: 2,
            stack: 9,
            prefix: 0,
            ownership: 0,
            shimmer: None,
            enemy_delay: None,
        };
        let Some(Packet::DroppedItem(i)) = drops.apply(0, request) else {
            panic!("missing stack")
        };
        let players = BTreeMap::from([(0, [0.; 2]), (1, [40.; 2])]);
        assert!(matches!(
            drops.refresh(&players).as_slice(),
            [(None, Packet::ItemOwner(ItemOwner { player: 0, .. }))]
        ));
        let moved = BTreeMap::from([(0, [200.; 2]), (1, [0.; 2])]);
        drops.active.get_mut(&i.slot).unwrap().reserved_until = Instant::now();
        assert_eq!(
            drops.refresh(&moved),
            vec![(
                Some(0),
                Packet::ReleaseItem {
                    slot: i.slot,
                    force: false
                }
            )]
        );
        assert!(drops.refresh(&moved).is_empty());
        assert!(drops.remove(1, i.slot).is_none());
        assert!(drops.release(0, i.slot, false).is_some());
        assert!(matches!(
            drops.refresh(&moved).as_slice(),
            [(None, Packet::ItemOwner(ItemOwner { player: 1, .. }))]
        ));
        assert!(drops.remove(0, i.slot).is_none());
        assert_eq!(drops.remove(1, i.slot), Some(Packet::RemoveItem(i.slot)));
        assert!(drops.remove(1, i.slot).is_none());
        assert!(
            matches!(drops.apply(0,request),Some(Packet::DroppedItem(next)) if next.slot != i.slot)
        );
    }
}
