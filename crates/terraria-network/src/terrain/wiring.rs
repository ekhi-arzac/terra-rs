//! Resolve a bounded pulse through ordinary wires from a one-tile switch.
use super::{LoadedWorld, WorldEdits, is_solid_floor};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use terraria_world::Tile;

pub(crate) const WIRE_MASKS: [(usize, u8); 4] = [(1, 2), (1, 4), (1, 8), (2, 32)];
pub(crate) const ACTUATOR: u8 = 2;
pub(crate) const INACTIVE: u8 = 4;
pub(crate) const SWITCH: u16 = 136;
const MAX_WIRE_VISITS: usize = 4096;

impl LoadedWorld {
    pub(crate) fn actuator_change(&self, position: [i16; 2]) -> Option<Tile> {
        let edits = self.edits.read().unwrap();
        let tile = self.tile_at(position, &edits)?;
        self.pulse_actuator(position, tile, &edits)
    }

    pub(crate) fn switch_changes(&self, origin: [i16; 2]) -> Option<Vec<([i16; 2], Tile)>> {
        let edits = self.edits.read().unwrap();
        let mut switch = self.tile_at(origin, &edits)?;
        let (frame_x, frame_y) = switch.frame?;
        if switch.block != Some(SWITCH) || switch.flags[2] & INACTIVE != 0 {
            return None;
        }
        switch.frame = Some((frame_x, if frame_y == 0 { 18 } else { 0 }));
        let mut changes = BTreeMap::from([(origin, switch)]);
        let mut remaining = MAX_WIRE_VISITS;
        for (byte, mask) in WIRE_MASKS {
            if switch.flags[byte] & mask == 0 {
                continue;
            }
            let mut visited = BTreeSet::from([origin]);
            let mut queue = VecDeque::from([(origin, switch)]);
            while let Some((position, original)) = queue.pop_front() {
                remaining = remaining.checked_sub(1)?;
                if position != origin {
                    let tile = changes.get(&position).copied().unwrap_or(original);
                    if let Some(updated) = self.pulse_actuator(position, tile, &edits) {
                        if updated == original {
                            changes.remove(&position);
                        } else {
                            changes.insert(position, updated);
                        }
                    }
                }
                let [x, y] = position;
                for next in [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]] {
                    if !visited.insert(next) {
                        continue;
                    }
                    if let Some(tile) = self.tile_at(next, &edits)
                        && tile.flags[byte] & mask != 0
                        && !matches!(tile.block, Some(424 | 445))
                    {
                        queue.push_back((next, tile));
                    }
                }
            }
        }
        Some(changes.into_iter().collect())
    }

    fn pulse_actuator(&self, [x, y]: [i16; 2], mut tile: Tile, edits: &WorldEdits) -> Option<Tile> {
        if tile.flags[2] & ACTUATOR == 0 || tile.frame.is_some() {
            return None;
        }
        let mut flat = tile;
        flat.flags[1] &= !0x70;
        flat.flags[2] &= !INACTIVE;
        if !is_solid_floor(flat) {
            return None;
        }
        if tile.flags[2] & INACTIVE == 0 {
            let above = self.tile_at([x, y - 1], edits)?;
            if above.frame.is_some() || matches!(above.block, Some(5 | 12 | 26 | 72 | 235 | 237)) {
                return None;
            }
        }
        tile.flags[2] ^= INACTIVE;
        Some(tile)
    }
}
