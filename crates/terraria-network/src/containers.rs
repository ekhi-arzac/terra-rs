//! The active chest indices remain stable when a world save compacts its chest records.
use crate::terrain::LoadedWorld;
use bytes::BytesMut;
use std::collections::{BTreeMap, BTreeSet};
use terraria_protocol::{Error, Packet, section::SectionObject};
use terraria_world::{Chest, ChestStack, StoredItem, World};

#[derive(Default)]
pub(super) struct Containers {
    entries: BTreeMap<i16, Chest>,
    positions: BTreeMap<[i16; 2], i16>,
    sections: BTreeMap<[u16; 2], BTreeSet<i16>>,
    dirty: BTreeMap<(i32, i32), u64>,
    removals: BTreeMap<([i16; 2], i16), u64>,
}

impl Containers {
    pub fn load(source: &World) -> Self {
        let mut result = Self::default();
        for (id, chest) in source.chests().iter().enumerate() {
            let id = id as i16;
            let (x, y) = chest.position;
            if (0..source.metadata().width as i32).contains(&x)
                && (0..source.metadata().height as i32).contains(&y)
            {
                let pos = [x as i16, y as i16];
                result.positions.entry(pos).or_insert(id);
                result
                    .sections
                    .entry(LoadedWorld::section_at(pos))
                    .or_default()
                    .insert(id);
            }
            result.entries.insert(id, chest.clone());
        }
        result
    }
    pub fn create(&mut self, pos: [i16; 2], rev: u64) -> Option<i16> {
        if self.at(pos).is_some() {
            return None;
        }
        let id = (0..8000).find(|i| !self.entries.contains_key(i))?;
        let position = (i32::from(pos[0]), i32::from(pos[1]));
        self.entries.insert(
            id,
            Chest {
                position,
                name: String::new(),
                capacity: 40,
                items: vec![],
            },
        );
        self.positions.insert(pos, id);
        self.sections
            .entry(LoadedWorld::section_at(pos))
            .or_default()
            .insert(id);
        self.dirty.insert(position, rev);
        Some(id)
    }
    pub fn remove_empty(&mut self, pos: [i16; 2], rev: u64) -> Option<i16> {
        let id = self.at(pos)?;
        if !self.entries.get(&id)?.items.is_empty() {
            return None;
        }
        let chest = self.entries.remove(&id)?;
        self.positions.remove(&pos);
        if let Some(ids) = self.sections.get_mut(&LoadedWorld::section_at(pos)) {
            ids.remove(&id);
        }
        self.dirty.insert(chest.position, rev);
        self.removals.insert((pos, id), rev);
        Some(id)
    }
    pub fn removed_since(
        &self,
        since: u64,
        sent: &BTreeSet<[u16; 2]>,
        out: &mut BytesMut,
    ) -> Result<(), Error> {
        for (&(pos, id), &revision) in &self.removals {
            if revision > since && sent.contains(&LoadedWorld::section_at(pos)) {
                out.extend_from_slice(
                    &Packet::ChestUpdate {
                        action: 1,
                        position: pos,
                        style: 0,
                        chest: id,
                    }
                    .encode()?,
                );
            }
        }
        Ok(())
    }

    pub fn at(&self, pos: [i16; 2]) -> Option<i16> {
        self.positions.get(&pos).copied()
    }
    pub fn position(&self, id: i16) -> Option<[i16; 2]> {
        let (x, y) = self.entries.get(&id)?.position;
        Some([x as i16, y as i16])
    }
    pub fn name(&self, id: i16) -> Option<Packet> {
        let chest = self.entries.get(&id)?;
        Some(Packet::ChestName {
            chest: id,
            position: self.position(id)?,
            name: chest.name.clone(),
        })
    }
    pub fn rename(&mut self, id: i16, name: String, rev: u64) -> Option<Packet> {
        let chest = self.entries.get_mut(&id)?;
        chest.name = name;
        self.dirty.insert(chest.position, rev);
        self.name(id)
    }
    pub fn edit_slot(&mut self, id: i16, slot: u8, value: ChestStack, rev: u64) -> bool {
        let Some(chest) = self.entries.get_mut(&id) else {
            return false;
        };
        let slot = u32::from(slot);
        if slot >= chest.capacity {
            return false;
        }
        let item = StoredItem {
            slot,
            stack: value.stack,
            prefix: value.prefix,
            item_type: i32::from(value.item),
        };
        match (
            chest.items.binary_search_by_key(&slot, |i| i.slot),
            value.stack > 0,
        ) {
            (Ok(i), true) => chest.items[i] = item,
            (Ok(i), false) => {
                chest.items.remove(i);
            }
            (Err(i), true) => chest.items.insert(i, item),
            (Err(_), false) => {}
        }
        self.dirty.insert(chest.position, rev);
        true
    }
    pub fn changes(&self, since: u64) -> BTreeMap<(i32, i32), Option<Chest>> {
        self.dirty
            .iter()
            .filter(|(_, rev)| **rev > since)
            .map(|(&(x, y), _)| {
                let chest = self
                    .at([x as i16, y as i16])
                    .and_then(|id| self.entries.get(&id))
                    .cloned();
                ((x, y), chest)
            })
            .collect()
    }
    pub fn write_contents(&self, id: i16, batch: &mut BytesMut) -> Result<(), Error> {
        let chest = self
            .entries
            .get(&id)
            .ok_or(Error::Invalid("unknown chest"))?;
        if chest.capacity > 256 {
            return Err(Error::Invalid("chest capacity"));
        }
        batch.extend_from_slice(
            &Packet::ChestSize {
                index: id,
                capacity: chest.capacity as i16,
            }
            .encode()?,
        );
        let mut items = chest.items.iter().peekable();
        for slot in 0..chest.capacity {
            let item = items.next_if(|i| i.slot == slot);
            batch.extend_from_slice(
                &Packet::ChestItem {
                    chest: id,
                    slot: slot as u8,
                    stack: item.map_or(0, |v| v.stack),
                    prefix: item.map_or(0, |v| v.prefix),
                    item: item
                        .map_or(Ok(0), |v| i16::try_from(v.item_type))
                        .map_err(|_| Error::Invalid("chest item type"))?,
                }
                .encode()?,
            );
        }
        Ok(())
    }
    pub fn section_snapshot(
        &self,
        section: [u16; 2],
        batch: &mut BytesMut,
    ) -> Result<Vec<SectionObject>, Error> {
        let mut objects = Vec::new();
        for &id in self.sections.get(&section).into_iter().flatten() {
            let chest = &self.entries[&id];
            let (x, y) = chest.position;
            objects.push(SectionObject {
                index: id,
                position: [x as i16, y as i16],
                text: chest.name.clone(),
            });
            self.write_contents(id, batch)?;
        }
        objects.sort_by_key(|o| (o.position[1], o.position[0]));
        Ok(objects)
    }
    pub fn catch_up(
        &self,
        since: u64,
        sent: &BTreeSet<[u16; 2]>,
        out: &mut BytesMut,
    ) -> Result<(), Error> {
        for (&(x, y), &rev) in &self.dirty {
            let pos = [x as i16, y as i16];
            if rev > since
                && sent.contains(&LoadedWorld::section_at(pos))
                && let Some(id) = self.at(pos)
            {
                if let Some(name) = self.name(id) {
                    out.extend_from_slice(&name.encode()?);
                }
                self.write_contents(id, out)?;
            }
        }
        Ok(())
    }
}

pub(super) fn wooden_chest_origin(world: &LoadedWorld, hit: [i16; 2]) -> Option<[i16; 2]> {
    let tile = world.tile(hit)?;
    if tile.block != Some(21) {
        return None;
    }
    let (x, y) = tile.frame?;
    if !matches!(x, 0 | 18) || !matches!(y, 0 | 18) {
        return None;
    }
    Some([hit[0].checked_sub(x / 18)?, hit[1].checked_sub(y / 18)?])
}

pub(super) fn chest_cells(world: &LoadedWorld, origin: [i16; 2]) -> Option<crate::stations::Cells> {
    let mut cells = Vec::with_capacity(4);
    for dx in 0..2 {
        for dy in 0..2 {
            let pos = [origin[0].checked_add(dx)?, origin[1].checked_add(dy)?];
            cells.push((pos, world.tile(pos)?));
        }
    }
    Some(cells)
}
