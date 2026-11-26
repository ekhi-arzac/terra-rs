//! Shared hammer shapes and mechanical tile flags carried by terrain updates.
use super::{BytesMut, Roster};
use crate::terrain::{
    LoadedWorld, is_solid_floor,
    wiring::{ACTUATOR, SWITCH, WIRE_MASKS},
};
use std::collections::BTreeMap;
use terraria_protocol::{Error, Packet};
use terraria_world::Tile;

pub(super) fn handles(action: u8) -> bool {
    matches!(action, 5..=14 | 16 | 17 | 19 | 23)
}

fn hammer(
    world: &LoadedWorld,
    [x, y]: [i16; 2],
    mut tile: Tile,
    action: u8,
    value: i16,
) -> Option<Tile> {
    const SHAPE: u8 = 0x70;
    const HALF: u8 = 0x10;

    // Reuse the terrain support table and leave framed platforms to their own framing path.
    let mut flat = tile;
    flat.flags[1] &= !SHAPE;
    if tile.block == Some(19) || !is_solid_floor(flat) || tile.wall == 350 {
        return None;
    }
    let above = y
        .checked_sub(1)
        .and_then(|above_y| world.tile([x, above_y]));
    if above.and_then(|tile| tile.block).is_some_and(|block| {
        matches!(
            block,
            21 | 26 | 77 | 88 | 235 | 237 | 441 | 467 | 468 | 470 | 475 | 488 | 597
        )
    }) {
        return None;
    }
    let current = tile.flags[1] & SHAPE;
    let next = match action {
        7 if current <= HALF => current ^ HALF,
        14 if (0..=4).contains(&value) => {
            if value == 0 {
                0
            } else {
                ((value as u8) + 1) << 4
            }
        }
        // The stock client sends zero to clear a slope and toggle the half-block.
        23 if value == 0 => {
            if current == HALF {
                0
            } else {
                HALF
            }
        }
        _ => return None,
    };
    tile.flags[1] = (tile.flags[1] & !SHAPE) | next;
    Some(tile)
}

fn mechanical(mut tile: Tile, action: u8) -> (Tile, Option<i16>) {
    let ((byte, mask), place, item) = match action {
        5 | 6 => (WIRE_MASKS[0], action == 5, 530),
        8 | 9 => ((2, ACTUATOR), action == 8, 849),
        10 | 11 => (WIRE_MASKS[1], action == 10, 530),
        12 | 13 => (WIRE_MASKS[2], action == 12, 530),
        16 | 17 => (WIRE_MASKS[3], action == 16, 530),
        _ => return (tile, None),
    };
    let was_set = tile.flags[byte] & mask != 0;
    if place {
        tile.flags[byte] |= mask;
    } else {
        tile.flags[byte] &= !mask;
    }
    (tile, (was_set && !place).then_some(item))
}

impl Roster {
    pub(super) fn hit_switch(&mut self, slot: u8, position: [i16; 2]) -> Result<(), Error> {
        let Some(world) = self.world.clone() else {
            return Ok(());
        };
        let Some(changes) = world.switch_changes(position) else {
            if let Some(peer) = self.peers.get_mut(&slot)
                && let Some(tile) = world.tile(position)
                && tile.block == Some(SWITCH)
            {
                peer.queue(LoadedWorld::tile_patch(position, tile)?);
            }
            return Ok(());
        };
        // Group committed snapshots by subscription, but keep distant actuators in separate frames.
        let mut batches = BTreeMap::<_, BytesMut>::new();
        for &(pos, tile) in &changes {
            batches
                .entry(LoadedWorld::section_at(pos))
                .or_default()
                .extend_from_slice(&LoadedWorld::tile_patch(pos, tile)?);
        }
        let click = Packet::HitSwitch(position).encode()?;
        world.commit_tiles(&changes, None);
        // Clients can play the stock switch event, then apply the authoritative frame and state.
        self.send_section_update(LoadedWorld::section_at(position), Some(slot), click);
        for (section, batch) in batches {
            self.send_section_update(section, None, batch.freeze());
        }
        Ok(())
    }

    pub(super) fn construction_edit(
        &mut self,
        slot: u8,
        action: u8,
        position: [i16; 2],
        value: i16,
    ) -> Result<(), Error> {
        let Some(world) = self.world.clone() else {
            return Ok(());
        };
        let Some(before) = world.tile(position) else {
            return Ok(());
        };
        let (updated, item) = match action {
            7 | 14 | 23 => (
                hammer(&world, position, before, action, value).unwrap_or(before),
                None,
            ),
            19 => (world.actuator_change(position).unwrap_or(before), None),
            _ => mechanical(before, action),
        };
        if updated != before {
            self.publish_tiles(&world, &[(position, updated)])?;
            if let Some(item) = item {
                self.spawn_resource(slot, position, item, 1)?;
            }
        } else if let Some(peer) = self.peers.get_mut(&slot) {
            peer.queue(LoadedWorld::tile_patch(position, before)?);
        }
        Ok(())
    }
}
