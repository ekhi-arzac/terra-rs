//! Saved town residents as they appear before the first simulation tick.
use terraria_protocol::{NpcLife, NpcSync, Packet};
use terraria_world::World;

pub(super) struct Resident {
    pub sync: NpcSync,
    pub name: Packet,
    pub home: Packet,
}

impl Resident {
    pub fn load(world: &World) -> Result<Vec<Self>, terraria_world::Error> {
        let assigned = world.assigned_town_rooms()?;
        let mut residents = Vec::new();
        for (index, npc) in world.npcs().iter().enumerate() {
            let Some(town) = &npc.town else { continue };
            // Types verified from a populated town on the 1.4.6.1 server.
            if !matches!(
                npc.kind,
                17..=20 | 22 | 37..=38 | 54 | 107..=108 | 124 | 142 | 160 |
                178 | 207..=209 | 227..=229 | 353 | 369 | 441 | 550 | 588 |
                633 | 637..=638 | 656 | 663
            ) {
                continue;
            }
            let slot = index as i16;
            let sync = NpcSync {
                slot: index as u8,
                generation: 0,
                position: npc.position,
                velocity: [0.; 2],
                target: 255,
                right: false,
                down: true,
                sprite_right: false,
                ai: [None; 4],
                kind: npc.kind as i16,
                scaled_players: None,
                statue: false,
                difficulty: None,
                spawning: false,
                shimmer: false,
                life: NpcLife::Full,
                release_owner: None,
            };
            let name = Packet::NpcName {
                slot,
                name: town.name.clone(),
                variation: town.variation.unwrap_or(0),
            };
            let state = if town.homeless {
                1
            } else if assigned.contains(&npc.kind) {
                2
            } else {
                0
            };
            let home = Packet::NpcHome {
                slot,
                position: town.home.map(|c| c as i16),
                state,
            };
            residents.push(Self { sync, name, home });
        }
        Ok(residents)
    }
}
