use super::Roster;
use terraria_protocol::{
    DeathReason, Error, Movement, NetworkText, Packet, PlayerDeath, Spawn, Vitals,
};

fn announcement(reason: &DeathReason, name: &str, world: &str) -> NetworkText {
    if let Some(text) = &reason.custom {
        return NetworkText::Literal(text.clone());
    }
    let key = match reason.other {
        Some(0) => "Fell_1",
        Some(1) => "Drowned_1",
        Some(2) => "Lava_1",
        Some(5) => "Petrified_1",
        Some(6) => "Stabbed",
        Some(7) => "Suffocated_1",
        Some(8) => "Burned_1",
        Some(9) => "Poisoned",
        Some(10) => "Electrocuted_1",
        Some(11) => "TriedToEscape",
        Some(12) => "WasLicked_1",
        Some(13) => "Teleport_1",
        Some(14) => "Teleport_2_Male",
        Some(15) => "Teleport_2_Female",
        Some(16) => "Inferno",
        Some(17) => "DiedInTheDark",
        Some(18) => "Starved_1",
        Some(19) => "Space_1",
        Some(20) => "TeamTank",
        Some(21) => "Underground_1",
        Some(22) => "VampireBurningInDaylight_1",
        Some(23) => "Spored",
        _ => "Slain",
    };
    let mut args = vec![NetworkText::Literal(name.to_owned())];
    if matches!(reason.other, Some(19 | 21 | 22)) {
        args.push(NetworkText::Literal(world.to_owned()));
    }
    NetworkText::Key(format!("DeathText.{key}"), args)
}

impl Roster {
    pub(super) fn player_death(&mut self, death: PlayerDeath) -> Result<(), Error> {
        let slot = death.player;
        let Some(peer) = self.peers.get_mut(&slot) else {
            return Ok(());
        };
        if !peer.dead {
            let name = &peer
                .character
                .appearance
                .as_ref()
                .ok_or(Error::Invalid("missing player appearance"))?
                .name;
            let world = self.world.as_ref().map_or("", |w| w.data.name.as_str());
            let text = announcement(&death.reason, name, world);
            peer.dead = true;
            peer.pvp_death = death.pvp;
            let life = peer.character.life.get_or_insert(Vitals {
                player: slot,
                current: 0,
                maximum: 100,
            });
            life.current = 0;
            let chest = peer.chest.take();
            self.send(
                None,
                Packet::ChatMessage {
                    author: 255,
                    text,
                    color: [225, 25, 25],
                }
                .encode()?,
            );
            if chest.is_some() {
                self.send(
                    None,
                    Packet::PlayerChest {
                        player: slot,
                        chest: -1,
                    }
                    .encode()?,
                );
            }
            self.refresh_owners()?;
        }
        self.send(Some(slot), Packet::PlayerDeath(death).encode()?);
        Ok(())
    }

    pub(super) fn respawn(&mut self, spawn: Spawn) -> Result<(), Error> {
        let Some(peer) = self.peers.get_mut(&spawn.player) else {
            return Ok(());
        };
        peer.team = spawn.team;
        peer.character.spawn = Some(spawn);
        peer.dead = false;
        if let Some(life) = &mut peer.character.life {
            if peer.pvp_death {
                life.current = life.maximum;
            } else if life.current <= 0 {
                life.current = 100.max(life.maximum / 2);
            }
        }
        peer.pvp_death = false;
        if let Some(world) = &self.world {
            let point = if spawn
                .point
                .iter()
                .zip(world.data.size)
                .all(|(&i, n)| (0..n).contains(&i))
            {
                spawn.point
            } else {
                world.data.spawn
            };
            peer.movement = Some(Movement {
                player: spawn.player,
                flags: [64, 16, 0, 0],
                selected: peer.movement.map_or(0, |m| m.selected),
                position: [
                    f32::from(point[0]) * 16. - 2.,
                    f32::from(point[1]) * 16. - 42.,
                ],
                velocity: None,
                mount: None,
                return_points: None,
                camera: None,
            });
        }
        self.send(Some(spawn.player), Packet::Spawn(spawn).encode()?);
        Ok(())
    }
}
