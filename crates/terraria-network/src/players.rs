//! The roster owner serializes membership and player updates. Socket writes never run on it.
mod construction;
mod death;
mod teams;
use crate::doors::{self, Door};
use crate::drops::Drops;
use crate::stations::Station;
use crate::terrain::{LoadedWorld, SectionSnapshot, is_ground_cover};
use crate::trees;
use crate::{Character, Error, read_frame, write_packet};
use bytes::{Bytes, BytesMut};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    time::Duration,
};
use terraria_protocol::{
    Equipment, Error as ProtocolError, Movement, NetworkText, Packet, ProjectileRemove,
};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc, time::timeout};

pub(super) enum Outbound {
    Bytes(Bytes),
    Section(SectionSnapshot),
}

pub(super) enum Event {
    Join {
        slot: u8,
        character: Box<Character>,
        sent: BTreeSet<[u16; 2]>,
        revision: u64,
        out: mpsc::Sender<Outbound>,
    },
    Update {
        slot: u8,
        packet: Packet,
    },
    Leave(u8),
    ListPlayers,
}

struct Player {
    character: Box<Character>,
    sections: BTreeSet<[u16; 2]>,
    chest: Option<i16>,
    movement: Option<Movement>,
    projectiles: BTreeMap<u16, ProjectileRemove>,
    dead: bool,
    pvp_death: bool,
    hostile: bool,
    team: u8,
    zones: Option<([u8; 5], u8)>,
    out: Option<mpsc::Sender<Outbound>>,
}
impl Player {
    fn queue(&mut self, bytes: Bytes) {
        self.enqueue(Outbound::Bytes(bytes));
    }

    fn enqueue(&mut self, outgoing: Outbound) {
        if self
            .out
            .as_ref()
            .is_some_and(|out| out.try_send(outgoing).is_err())
        {
            self.out = None;
        }
    }
    fn snapshot(&self, slot: u8) -> Result<Bytes, ProtocolError> {
        let mut bytes = BytesMut::new();
        let mut add = |p: Packet| -> Result<(), ProtocolError> {
            bytes.extend_from_slice(&p.encode()?);
            Ok(())
        };
        add(Packet::PlayerActive {
            player: slot,
            active: true,
        })?;
        add(Packet::PlayerTeam {
            player: slot,
            team: self.team,
        })?;
        add(Packet::PlayerPvp {
            player: slot,
            hostile: self.hostile,
        })?;
        if let Some(chest) = self.chest {
            add(Packet::PlayerChest {
                player: slot,
                chest,
            })?;
        }
        if let Some(a) = &self.character.appearance {
            add(Packet::Appearance(a.clone()))?;
        }
        if let Some((zones, town_npcs)) = self.zones {
            add(Packet::PlayerZones {
                player: slot,
                zones,
                town_npcs,
            })?;
        }
        if let Some(m) = self.movement {
            add(Packet::Movement(m))?;
        }
        if let Some(v) = self.character.life {
            if v.current <= 0 {
                add(Packet::SetDeadAppearance(slot))?;
            }
            add(Packet::Life(v))?;
        }
        if let Some(v) = self.character.mana {
            add(Packet::Mana(v))?;
        }
        add(Packet::Buffs {
            player: slot,
            types: self.character.buffs.clone(),
        })?;
        if let Some((index, hidden_accessories)) = self.character.loadout {
            add(Packet::Loadout {
                player: slot,
                index,
                hidden_accessories,
            })?;
        }
        for index in (0..99).chain(900..990) {
            let equipment = self
                .character
                .equipment
                .get(&index)
                .copied()
                .unwrap_or(Equipment {
                    player: slot,
                    slot: index,
                    stack: 0,
                    prefix: 0,
                    item: 0,
                    flags: 0,
                });
            add(Packet::Equipment(equipment))?;
        }
        Ok(bytes.freeze())
    }
}

pub(super) struct Roster {
    peers: BTreeMap<u8, Player>,
    drops: Drops,
    world: Option<std::sync::Arc<LoadedWorld>>,
    player_count: std::sync::Arc<std::sync::atomic::AtomicU8>,
}
impl Roster {
    pub fn new(
        world: Option<std::sync::Arc<LoadedWorld>>,
        player_count: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) -> Self {
        Self {
            peers: BTreeMap::new(),
            drops: Drops::default(),
            world,
            player_count,
        }
    }
    fn spawn_resource(
        &mut self,
        slot: u8,
        position: [i16; 2],
        item: i16,
        stack: i16,
    ) -> Result<(), ProtocolError> {
        self.spawn_drop(
            slot,
            terraria_protocol::DroppedItem {
                slot: terraria_protocol::NEW_ITEM,
                position: position.map(|n| f32::from(n) * 16.0),
                velocity: [0.0; 2],
                stack,
                prefix: 0,
                ownership: 1,
                item,
                shimmer: None,
                enemy_delay: None,
            },
        )
    }
    fn spawn_drop(
        &mut self,
        slot: u8,
        item: terraria_protocol::DroppedItem,
    ) -> Result<(), ProtocolError> {
        if let Some(Packet::DroppedItem(i)) = self.drops.apply(slot, item) {
            let mut batch = BytesMut::new();
            batch.extend_from_slice(&Packet::DroppedItem(i).encode()?);
            if let Some(owner) = self.drops.owner(i.slot) {
                batch.extend_from_slice(&owner.encode()?);
            }
            self.send(None, batch.freeze());
            self.refresh_owners()?;
        }
        Ok(())
    }
    fn refresh_owners(&mut self) -> Result<(), ProtocolError> {
        if self.drops.is_empty() {
            return Ok(());
        }
        let spawn = self
            .world
            .as_ref()
            .map_or([0.0; 2], |w| w.data.spawn.map(|n| f32::from(n) * 16.0));
        let positions = self
            .peers
            .iter()
            .filter(|(_, p)| p.out.is_some() && !p.dead)
            .map(|(&id, p)| (id, p.movement.map_or(spawn, |m| m.position)))
            .collect();
        let mut bytes = BytesMut::new();
        let mut requests = BTreeMap::<u8, BytesMut>::new();
        for (recipient, p) in self.drops.refresh(&positions) {
            match recipient {
                Some(slot) => requests
                    .entry(slot)
                    .or_default()
                    .extend_from_slice(&p.encode()?),
                None => bytes.extend_from_slice(&p.encode()?),
            }
        }
        for (slot, request) in requests {
            if let Some(peer) = self.peers.get_mut(&slot) {
                peer.queue(request.freeze());
            }
        }
        if !bytes.is_empty() {
            self.send(None, bytes.freeze());
        }
        Ok(())
    }
    fn send(&mut self, except: Option<u8>, bytes: Bytes) {
        for (&slot, player) in &mut self.peers {
            if Some(slot) != except {
                player.queue(bytes.clone());
            }
        }
    }
    fn send_section_update(&mut self, section: [u16; 2], except: Option<u8>, bytes: Bytes) {
        for (&slot, player) in &mut self.peers {
            if Some(slot) != except && player.sections.contains(&section) {
                player.queue(bytes.clone());
            }
        }
    }
    fn publish_tiles(
        &mut self,
        world: &LoadedWorld,
        changes: &[([i16; 2], terraria_world::Tile)],
    ) -> Result<(), ProtocolError> {
        let patches = world.patches(changes)?;
        world.commit_tiles(changes, None);
        for (section, bytes) in patches {
            self.send_section_update(section, None, bytes);
        }
        Ok(())
    }
    fn publish_footprint(
        &mut self,
        world: &LoadedWorld,
        changes: &[([i16; 2], terraria_world::Tile)],
    ) -> Result<(), ProtocolError> {
        let (bounds, bytes) = world.footprint_patch(changes)?;
        world.commit_tiles(changes, Some(bounds));
        self.send_footprint(changes, bytes);
        Ok(())
    }
    fn send_footprint(&mut self, changes: &[([i16; 2], terraria_world::Tile)], bytes: Bytes) {
        for player in self.peers.values_mut() {
            if changes
                .iter()
                .any(|&(pos, _)| player.sections.contains(&LoadedWorld::section_at(pos)))
            {
                player.queue(bytes.clone());
            }
        }
    }

    fn chest_update(
        &mut self,
        slot: u8,
        action: u8,
        pos: [i16; 2],
        style: i16,
        drop_item: bool,
    ) -> Result<bool, ProtocolError> {
        use crate::containers::{chest_cells, wooden_chest_origin};
        let Some(world) = self.world.clone() else {
            return Ok(false);
        };
        let place = action == 0;
        let origin = match action {
            0 if style == 0 => pos[1].checked_sub(1).map(|y| [pos[0], y]),
            1 => wooden_chest_origin(&world, pos),
            _ => None,
        };
        let Some(origin) = origin else {
            return Ok(false);
        };
        let Some(before) = chest_cells(&world, origin) else {
            return Ok(false);
        };
        let Some(actor) = self.peers.get(&slot) else {
            return Ok(false);
        };
        let id = world.chest_at(origin);
        let valid = before.iter().all(|&(cell, tile)| {
            if !actor.sections.contains(&LoadedWorld::section_at(cell)) {
                return false;
            }
            if place {
                tile.block.is_none_or(is_ground_cover)
                    && !matches!(tile.liquid, Some((terraria_world::Liquid::Lava, _)))
            } else {
                wooden_chest_origin(&world, cell) == Some(origin)
            }
        }) && if place {
            before
                .iter()
                .filter(|(cell, _)| cell[1] == pos[1])
                .all(|(cell, _)| {
                    cell[1]
                        .checked_add(1)
                        .and_then(|y| world.tile([cell[0], y]))
                        .is_some_and(crate::terrain::is_solid_floor)
                })
        } else {
            id.is_some() && !self.peers.values().any(|p| p.chest == id)
        };
        if valid {
            let changes = before
                .iter()
                .map(|&(cell, tile)| {
                    let mut tile = LoadedWorld::with_block(tile, place.then_some(21));
                    if place {
                        tile.frame = Some(((cell[0] - origin[0]) * 18, (cell[1] - origin[1]) * 18));
                    }
                    (cell, tile)
                })
                .collect::<Vec<_>>();
            let (bounds, patch) = world.footprint_patch(&changes)?;
            if let Some(chest) = world.commit_chest(origin, place, &changes, bounds) {
                let mut batch = BytesMut::new();
                batch.extend_from_slice(
                    &Packet::ChestUpdate {
                        action,
                        position: if place { pos } else { origin },
                        style: 0,
                        chest,
                    }
                    .encode()?,
                );
                batch.extend_from_slice(&patch);
                self.send_footprint(&changes, batch.freeze());
                if !place && drop_item {
                    self.spawn_resource(slot, origin, 48, 1)?;
                }
                return Ok(true);
            }
        }
        let mut correction = BytesMut::new();
        if place {
            correction.extend_from_slice(
                &Packet::ChestUpdate {
                    action,
                    position: pos,
                    style,
                    chest: -1,
                }
                .encode()?,
            );
        }
        correction.extend_from_slice(&world.footprint_patch(&before)?.1);
        // A rejected prediction can locally remove an occupied chest. Restore its index and contents.
        let mut overlapping = BTreeSet::new();
        for &(cell, _) in &before {
            if let Some(pos) = wooden_chest_origin(&world, cell)
                && let Some(chest) = world.chest_at(pos)
            {
                overlapping.insert(chest);
            }
        }
        if let Some(chest) = id {
            overlapping.insert(chest);
        }
        for chest in overlapping {
            if let Some(name) = world.chest_name(chest) {
                correction.extend_from_slice(&name.encode()?);
            }
            correction.extend_from_slice(&world.chest_contents(chest)?);
        }
        if let Some(actor) = self.peers.get_mut(&slot) {
            actor.queue(correction.freeze());
        }
        if place && drop_item {
            self.spawn_resource(slot, pos, 48, 1)?;
        }
        Ok(false)
    }

    fn place_station(
        &mut self,
        slot: u8,
        anchor: [i16; 2],
        station: Station,
        style: i16,
        alternate: u8,
    ) -> Result<(), ProtocolError> {
        let Some(world) = self.world.clone() else {
            return Ok(());
        };
        let Some(peer) = self.peers.get_mut(&slot) else {
            return Ok(());
        };
        let Some(cells) = station.footprint(&world, anchor) else {
            return Ok(());
        };
        let subscribed = |pos| peer.sections.contains(&LoadedWorld::section_at(pos));
        if alternate == 0
            && cells.iter().all(|&(pos, _)| subscribed(pos))
            && let Some(edits) = station.placement(&world, style, &cells)
        {
            self.publish_footprint(&world, &edits)?;
        } else {
            // Reverse local prediction over the whole footprint, even when only one cell conflicts.
            peer.queue(world.footprint_patch(&cells)?.1);
        }
        Ok(())
    }

    fn place_door(
        &mut self,
        slot: u8,
        anchor: [i16; 2],
        style: i16,
        origin_row: u8,
    ) -> Result<(), ProtocolError> {
        let Some(world) = self.world.clone() else {
            return Ok(());
        };
        let Some(peer) = self.peers.get_mut(&slot) else {
            return Ok(());
        };
        let Some(door) = Door::placement(anchor, style, origin_row) else {
            return Ok(());
        };
        let Some(cells) = door.footprint(&world) else {
            return Ok(());
        };
        if cells
            .iter()
            .all(|&(pos, _)| peer.sections.contains(&LoadedWorld::section_at(pos)))
            && let Some(edits) = door.place(&world, &cells)
        {
            self.publish_footprint(&world, &edits)?;
        } else {
            peer.queue(world.footprint_patch(&cells)?.1);
        }
        Ok(())
    }

    fn toggle_door(
        &mut self,
        slot: u8,
        action: u8,
        pos: [i16; 2],
        right: bool,
    ) -> Result<(), ProtocolError> {
        if action > 1 {
            return Ok(());
        } // Trapdoors and tall gates have different layouts.
        let Some(world) = self.world.clone() else {
            return Ok(());
        };
        let Some(tile) = world.tile(pos) else {
            return Ok(());
        };
        let Some(door) = Door::locate(&world, pos, tile) else {
            if let Some(peer) = self.peers.get_mut(&slot) {
                peer.queue(LoadedWorld::tile_patch(pos, tile)?);
            }
            return Ok(());
        };
        let blocked = action == 1
            && self.peers.values().any(|player| {
                !player.dead
                    && player
                        .movement
                        .is_some_and(|m| door.overlaps_hinge(m.position))
            });
        if let Some((tiles, accepted)) = door.toggle(&world, action == 0, right, blocked) {
            if accepted {
                self.publish_footprint(&world, &tiles)?;
            } else if let Some(peer) = self.peers.get_mut(&slot) {
                peer.queue(world.footprint_patch(&tiles)?.1);
            }
        }
        Ok(())
    }

    pub async fn run(mut self, mut input: mpsc::Receiver<Event>) {
        let period = Duration::from_nanos(16_666_667);
        let mut simulation = tokio::time::interval(period);
        let mut ownership = tokio::time::interval(Duration::from_millis(100));
        ownership.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let result = tokio::select! {
                event = input.recv() => match event {
                    Some(e) => {
                        let idle = self.peers.is_empty();
                        let result = self.handle(e);
                        if idle && !self.peers.is_empty() {
                            simulation.reset();
                        }
                        result
                    },
                    None => break
                },
                _ = ownership.tick(), if !self.drops.is_empty() => self.refresh_owners(),
                _ = simulation.tick(), if !self.peers.is_empty() => self.simulate_tick(),
            };
            if let Err(error) = result {
                tracing::debug!(%error,"could not serialize player update");
            }
        }
    }
    fn simulate_tick(&mut self) -> Result<(), ProtocolError> {
        if let Some(world) = &self.world
            && let Some(update) = world.advance_time()
        {
            self.send(None, update.encode()?);
        }
        Ok(())
    }
    fn handle(&mut self, event: Event) -> Result<(), ProtocolError> {
        match event {
            Event::Join {
                slot,
                character,
                sent,
                revision,
                out,
            } => {
                let mut player = Player {
                    team: character.spawn.map_or(0, |spawn| spawn.team),
                    dead: character.life.is_some_and(|v| v.current <= 0)
                        || character.spawn.is_some_and(|s| s.respawn_timer > 0),
                    pvp_death: false,
                    hostile: false,
                    character,
                    sections: sent,
                    chest: None,
                    movement: None,
                    projectiles: BTreeMap::new(),
                    zones: None,
                    out: Some(out),
                };
                let mut roster = BytesMut::new();
                for (&id, peer) in &self.peers {
                    roster.extend_from_slice(&peer.snapshot(id)?);
                }
                if let Some(world) = &self.world {
                    world.catch_up(revision, &player.sections, &mut roster)?;
                }
                self.drops.snapshot(&mut roster)?;
                if let Some(world) = &self.world {
                    for resident in &world.residents {
                        roster.extend_from_slice(&resident.home.encode()?);
                    }
                }
                roster.extend_from_slice(&Packet::FinishedConnecting.encode()?);
                player.queue(roster.freeze());
                if player.out.is_some() {
                    self.send(None, player.snapshot(slot)?);
                    if let Some(spawn) = player.character.spawn {
                        self.send(None, Packet::Spawn(spawn).encode()?);
                    }
                    self.peers.insert(slot, player);
                    self.player_count
                        .store(self.peers.len() as u8, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Event::ListPlayers => {
                let names: Vec<_> = self
                    .peers
                    .values()
                    .filter_map(|peer| peer.character.appearance.as_ref().map(|a| a.name.as_str()))
                    .collect();
                tracing::info!(players = ?names, count = names.len(), "connected players");
            }
            Event::Leave(slot) => {
                if let Some(player) = self.peers.remove(&slot) {
                    self.player_count
                        .store(self.peers.len() as u8, std::sync::atomic::Ordering::Relaxed);
                    if player.chest.is_some() {
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
                    let mut removals = BytesMut::new();
                    for projectile in player.projectiles.into_values() {
                        removals.extend_from_slice(&Packet::ProjectileRemove(projectile).encode()?);
                    }
                    if !removals.is_empty() {
                        self.send(None, removals.freeze());
                    }
                    self.send(
                        None,
                        Packet::PlayerActive {
                            player: slot,
                            active: false,
                        }
                        .encode()?,
                    );
                }
            }
            Event::Update { slot, packet } => {
                let Some(peer) = self.peers.get_mut(&slot) else {
                    return Ok(());
                };
                if let Packet::TileChange { position, .. }
                | Packet::ObjectPlacement { position, .. }
                | Packet::DoorToggle { position, .. }
                | Packet::ChestUpdate { position, .. }
                | Packet::RequestChestName { position, .. }
                | Packet::RequestSign(position)
                | Packet::HitSwitch(position)
                | Packet::SignText { position, .. }
                | Packet::LiquidUpdate(terraria_protocol::LiquidUpdate { position, .. })
                | Packet::RequestChest(position) = &packet
                    && !peer.sections.contains(&LoadedWorld::section_at(*position))
                {
                    return Ok(());
                }
                match packet {
                    Packet::HitSwitch(position) => self.hit_switch(slot, position)?,
                    Packet::LiquidUpdate(change) => {
                        if let Some(reply) =
                            self.world.as_ref().and_then(|w| w.update_liquid(change))
                        {
                            let excluded = (change.amount == 0).then_some(slot);
                            self.send_section_update(
                                LoadedWorld::section_at(change.position),
                                excluded,
                                reply.encode()?,
                            );
                        }
                    }
                    Packet::RequestSign(position) => {
                        if let Some(reply) = self
                            .world
                            .as_ref()
                            .and_then(|w| w.open_sign(position, slot))
                        {
                            peer.queue(reply.encode()?);
                        }
                    }
                    Packet::SignText {
                        index,
                        position,
                        text,
                        ..
                    } => {
                        if let Some((reply, changed)) = self
                            .world
                            .as_ref()
                            .and_then(|w| w.edit_sign(position, index, text, slot))
                        {
                            let bytes = reply.encode()?;
                            if changed {
                                self.send_section_update(
                                    LoadedWorld::section_at(position),
                                    Some(slot),
                                    bytes,
                                );
                            } else {
                                peer.queue(bytes);
                            }
                        }
                    }
                    Packet::ProjectileSync(projectile)
                        if projectile.key.spawner == slot && matches!(projectile.kind, 1 | 14) =>
                    {
                        let bytes = Packet::ProjectileSync(projectile).encode()?;
                        let entry = ProjectileRemove {
                            key: projectile.key,
                            position: projectile.position,
                        };
                        if let Some(old) = peer.projectiles.insert(projectile.key.index, entry)
                            && old.key != projectile.key
                        {
                            self.send(Some(slot), Packet::ProjectileRemove(old).encode()?);
                        }
                        self.send(Some(slot), bytes);
                    }
                    Packet::ProjectileRemove(projectile) if projectile.key.spawner == slot => {
                        if peer
                            .projectiles
                            .get(&projectile.key.index)
                            .is_some_and(|active| active.key == projectile.key)
                        {
                            peer.projectiles.remove(&projectile.key.index);
                        }
                        self.send(Some(slot), Packet::ProjectileRemove(projectile).encode()?);
                    }
                    Packet::ProjectileSync(_) | Packet::ProjectileRemove(_) => {}
                    Packet::PlayerHurt(mut hurt) => {
                        let hostile = peer.hostile;
                        if let Some(target) = self.peers.get(&hurt.player)
                            && (hurt.player == slot || (hostile && target.hostile))
                        {
                            if hurt.player != slot {
                                hurt.reason.player = Some(i16::from(slot));
                                hurt.pvp = true;
                            }
                            self.send(Some(slot), Packet::PlayerHurt(hurt).encode()?);
                        }
                    }
                    Packet::PlayerDeath(death) => self.player_death(death)?,
                    Packet::Spawn(spawn) => self.respawn(spawn)?,
                    Packet::PlayerTeam { team, .. } => self.change_team(slot, team)?,
                    Packet::PlayerPvp { hostile, .. } => self.change_pvp(slot, hostile)?,
                    Packet::ChatCommand { command, text } if command == "Party" => {
                        self.party_chat(slot, text)?;
                    }
                    Packet::UseTimeDial { dusk, .. } => {
                        if let Some(update) =
                            self.world.as_ref().and_then(|w| w.activate_dial(dusk))
                        {
                            self.send(None, update.encode()?);
                        }
                    }
                    Packet::RequestSection(index) => {
                        if let Some(world) = &self.world
                            && world.contains(index)
                            && !peer.sections.contains(&index)
                        {
                            peer.enqueue(Outbound::Section(world.section_snapshot(index)?));
                            if peer.out.is_some() {
                                peer.sections.insert(index);
                            }
                        }
                    }
                    Packet::ChestUpdate {
                        action,
                        position,
                        style,
                        ..
                    } => {
                        self.chest_update(slot, action, position, style, true)?;
                    }
                    Packet::RequestNpcName(index) => {
                        if let Some(resident) = self.world.as_ref().and_then(|world| {
                            world
                                .residents
                                .iter()
                                .find(|r| i16::from(r.sync.slot) == index)
                        }) {
                            peer.queue(resident.name.encode()?);
                        }
                    }
                    Packet::DoorToggle {
                        action,
                        position,
                        facing_right,
                    } => {
                        self.toggle_door(slot, action, position, facing_right)?;
                    }
                    Packet::ObjectPlacement {
                        position,
                        tile,
                        style,
                        alternate,
                        ..
                    } => {
                        if tile == doors::CLOSED as i16 {
                            self.place_door(slot, position, style, alternate)?;
                        } else if let Some(station) =
                            u16::try_from(tile).ok().and_then(Station::for_block)
                        {
                            self.place_station(slot, position, station, style, alternate)?;
                        }
                    }
                    Packet::TileChange {
                        action,
                        position,
                        value,
                        ..
                    } if construction::handles(action) => {
                        self.construction_edit(slot, action, position, value)?;
                    }
                    Packet::TileChange {
                        action,
                        position,
                        value,
                        style,
                    } => {
                        let Some(world) = self.world.clone() else {
                            return Ok(());
                        };
                        if !matches!(action, 0 | 1 | 2 | 3 | 4 | 20) {
                            return Ok(());
                        }
                        let Some(tile) = world.tile(position) else {
                            return Ok(());
                        };
                        if action == 1
                            && let Some(station) =
                                u16::try_from(value).ok().and_then(Station::for_block)
                        {
                            return self.place_station(
                                slot,
                                position,
                                station,
                                i16::from(style),
                                0,
                            );
                        }
                        if matches!(action, 2 | 3) {
                            let wall = match action {
                                2 if value == 1 || tile.wall == 0 => return Ok(()),
                                2 if wall_item(tile.wall).is_some() => Some(0),
                                3 if tile.wall == 0
                                    && u16::try_from(value).ok().and_then(wall_item).is_some() =>
                                {
                                    Some(value as u16)
                                }
                                _ => None,
                            };
                            if let Some(wall) = wall {
                                let updated = terraria_world::Tile {
                                    wall,
                                    wall_paint: 0,
                                    flags: [
                                        tile.flags[0],
                                        tile.flags[1],
                                        tile.flags[2],
                                        tile.flags[3] & !0x14,
                                    ],
                                    ..tile
                                };
                                self.publish_tiles(&world, &[(position, updated)])?;
                                if action == 2
                                    && let Some(item) = wall_item(tile.wall)
                                {
                                    self.spawn_resource(slot, position, item, 1)?;
                                }
                            } else {
                                peer.queue(LoadedWorld::tile_patch(position, tile)?);
                            }
                            return Ok(());
                        }
                        let breaking = action != 1;
                        if breaking && value == 1 {
                            // Incomplete hits on grass-covered blocks can still strip the sod.
                            // Relay state changes only, so harmless hits do not reframe foliage.
                            if action == 20 {
                                peer.queue(tile_break_result(position, tile.block.is_some())?);
                            } else if let Some(soil) = tile.block.and_then(soil_under_grass) {
                                let updated = terraria_world::Tile {
                                    block: Some(soil),
                                    ..tile
                                };
                                self.publish_tiles(&world, &[(position, updated)])?;
                            }
                            return Ok(());
                        }
                        if breaking && tile.block.is_none() && action != 20 {
                            // Redundant cuts against air have no state to repair.
                            return Ok(());
                        }
                        if breaking
                            && crate::containers::wooden_chest_origin(&world, position).is_some()
                        {
                            let removed = self.chest_update(slot, 1, position, 0, action != 4)?;
                            if action == 20
                                && let Some(peer) = self.peers.get_mut(&slot)
                            {
                                peer.queue(tile_break_result(position, !removed)?);
                            }
                            return Ok(());
                        }
                        if breaking
                            && let Some(y) = position[1].checked_sub(2)
                            && [Some(position[0]), position[0].checked_sub(1)]
                                .into_iter()
                                .flatten()
                                .any(|x| world.chest_at([x, y]).is_some())
                        {
                            if action == 20 {
                                peer.queue(tile_break_result(position, true)?);
                            }
                            peer.queue(LoadedWorld::tile_patch(position, tile)?);
                            return Ok(());
                        }
                        if breaking
                            && let Some((changes, item)) = Door::locate(&world, position, tile)
                                .and_then(|door| door.removal(&world))
                        {
                            if action == 20 {
                                peer.queue(tile_break_result(position, false)?);
                            }
                            self.publish_footprint(&world, &changes)?;
                            if action != 4 {
                                self.spawn_resource(slot, position, item, 1)?;
                            }
                            return Ok(());
                        }
                        if breaking
                            && let Some((changes, item)) = Station::removal(&world, position, tile)
                        {
                            if action == 20 {
                                peer.queue(tile_break_result(position, false)?);
                            }
                            self.publish_footprint(&world, &changes)?;
                            if action != 4 {
                                self.spawn_resource(slot, position, item, 1)?;
                            }
                            return Ok(());
                        }
                        if breaking && trees::is_tree(tile) {
                            let harvest = trees::harvest(&world, position, tile);
                            let changes: Vec<_> = harvest
                                .tiles
                                .iter()
                                .map(|&(pos, before)| (pos, LoadedWorld::with_block(before, None)))
                                .collect();
                            if action == 20 {
                                peer.queue(tile_break_result(position, false)?);
                            }
                            self.publish_tiles(&world, &changes)?;
                            if action != 4 {
                                let mut remaining = harvest.tiles.len();
                                while remaining > 0 {
                                    let stack = remaining.min(9_999);
                                    self.spawn_resource(
                                        slot,
                                        position,
                                        harvest.wood,
                                        stack as i16,
                                    )?;
                                    remaining -= stack;
                                }
                            }
                            return Ok(());
                        }
                        let change = match action {
                            1 if tile.block.is_none()
                                && u16::try_from(value).ok().and_then(resource).is_some() =>
                            {
                                Some(Some(value as u16))
                            }
                            0 | 4 | 20
                                if tile.block.is_some_and(|block| {
                                    resource(block).is_some() || is_ground_cover(block)
                                }) =>
                            {
                                Some(None)
                            }
                            _ => None,
                        };
                        if let Some(block) = change {
                            if action == 20 {
                                peer.queue(tile_break_result(position, false)?);
                            }
                            let mut edits = vec![(position, LoadedWorld::with_block(tile, block))];
                            let mut attached_items = Vec::new();
                            if block.is_none() {
                                if let Some((mut cells, item)) =
                                    Station::on_support(&world, position)
                                {
                                    edits.append(&mut cells);
                                    attached_items.push(item);
                                }
                                for dy in [-1, 1] {
                                    let Some(y) = position[1].checked_add(dy) else {
                                        continue;
                                    };
                                    let next = [position[0], y];
                                    if let Some(tile) = world.tile(next)
                                        && let Some(door) = Door::locate(&world, next, tile)
                                        && door.supported_by(position)
                                        && let Some((mut cells, item)) = door.removal(&world)
                                    {
                                        edits.append(&mut cells);
                                        attached_items.push(item);
                                    }
                                }
                            }
                            if attached_items.is_empty() {
                                self.publish_tiles(&world, &edits)?;
                            } else {
                                self.publish_footprint(&world, &edits)?;
                            }
                            if block.is_none() && action != 4 {
                                for item in tile
                                    .block
                                    .and_then(resource)
                                    .into_iter()
                                    .chain(attached_items)
                                {
                                    self.spawn_resource(slot, position, item, 1)?;
                                }
                            }
                        } else if action == 20 {
                            peer.queue(tile_break_result(position, tile.block.is_some())?);
                        } else {
                            peer.queue(LoadedWorld::tile_patch(position, tile)?);
                        }
                    }
                    Packet::RequestChest(position) => {
                        let Some(world) = &self.world else {
                            return Ok(());
                        };
                        let Some(chest) = world.chest_at(position) else {
                            return Ok(());
                        };
                        if self
                            .peers
                            .iter()
                            .any(|(&id, p)| id != slot && p.chest == Some(chest))
                        {
                            return Ok(());
                        }
                        let mut batch = world.chest_contents(chest)?;
                        batch.extend_from_slice(
                            &Packet::ChestState {
                                chest,
                                position,
                                name: None,
                            }
                            .encode()?,
                        );
                        let peer = self.peers.get_mut(&slot).unwrap();
                        peer.queue(batch.freeze());
                        if peer.out.is_none() {
                            return Ok(());
                        }
                        peer.chest = Some(chest);
                        self.send(
                            Some(slot),
                            Packet::PlayerChest {
                                player: slot,
                                chest,
                            }
                            .encode()?,
                        );
                    }
                    Packet::RequestChestName { chest, position } => {
                        if let Some(world) = &self.world
                            && let Some(index) = world.chest_at(position)
                            && (chest == -1 || chest == index)
                            && let Some(packet) = world.chest_name(index)
                        {
                            peer.queue(packet.encode()?);
                        }
                    }
                    Packet::ChestState { chest, name, .. } => {
                        let previous = peer.chest;
                        let closed = chest < 0 && peer.chest.take().is_some();
                        if let Some(name) = name
                            && let Some(index) = previous
                            && let Some(world) = &self.world
                            && let Some(section) = world.chest_section(index)
                            && let Some(packet) = world.rename_chest(index, name)
                        {
                            self.send_section_update(section, Some(slot), packet.encode()?);
                        }
                        if closed {
                            self.send(
                                Some(slot),
                                Packet::PlayerChest {
                                    player: slot,
                                    chest: -1,
                                }
                                .encode()?,
                            );
                        }
                    }
                    Packet::ChestItem {
                        chest,
                        slot: index,
                        stack,
                        prefix,
                        item,
                    } => {
                        if peer.chest == Some(chest)
                            && let Some(world) = &self.world
                            && let Some(section) = world.chest_section(chest)
                            && world.edit_chest(
                                chest,
                                index,
                                terraria_world::ChestStack {
                                    stack,
                                    prefix,
                                    item,
                                },
                            )
                        {
                            self.send_section_update(section, Some(slot), packet.encode()?);
                        }
                    }
                    Packet::DroppedItem(item) => {
                        if item.slot == terraria_protocol::NEW_ITEM {
                            self.spawn_drop(slot, item)?;
                        } else if let Some(p) = self.drops.apply(slot, item) {
                            self.send(Some(slot), p.encode()?);
                        } else {
                            peer.queue(self.drops.correction(item.slot)?);
                        }
                    }
                    Packet::RemoveItem(index) => {
                        if let Some(p) = self.drops.remove(slot, index) {
                            self.send(Some(slot), p.encode()?);
                        } else {
                            peer.queue(self.drops.correction(index)?);
                        }
                    }
                    Packet::ReleaseItem { slot: index, force } => {
                        if let Some(p) = self.drops.release(slot, index, force) {
                            self.send(None, p.encode()?);
                            self.refresh_owners()?;
                        }
                    }
                    Packet::Ping => peer.queue(Packet::Ping.encode()?),
                    Packet::ChatCommand { text, .. } => {
                        let flags = peer
                            .character
                            .appearance
                            .as_ref()
                            .map_or(0, |a| a.difficulty_flags);
                        let color = if flags & 8 != 0 {
                            [255; 3]
                        } else if flags & 2 != 0 {
                            [255, 38, 25]
                        } else if flags & 1 != 0 {
                            [255, 153, 0]
                        } else {
                            [255; 3]
                        };
                        self.send(
                            None,
                            Packet::ChatMessage {
                                author: slot,
                                text: NetworkText::Literal(text),
                                color,
                            }
                            .encode()?,
                        );
                    }
                    p => {
                        match &p {
                            Packet::PlayerZones {
                                zones, town_npcs, ..
                            } => peer.zones = Some((*zones, *town_npcs)),
                            Packet::Movement(m) => peer.movement = Some(*m),
                            Packet::Life(v) => {
                                peer.character.life = Some(*v);
                                peer.dead = v.current <= 0;
                            }
                            Packet::Mana(v) => peer.character.mana = Some(*v),
                            Packet::Buffs { types, .. } => peer.character.buffs.clone_from(types),
                            Packet::Equipment(e) => {
                                if e.stack == 0 || e.item == 0 {
                                    peer.character.equipment.remove(&e.slot);
                                } else {
                                    peer.character.equipment.insert(e.slot, *e);
                                }
                                if (99..700).contains(&e.slot) {
                                    return Ok(());
                                }
                            }
                            Packet::Loadout {
                                index,
                                hidden_accessories,
                                ..
                            } => peer.character.loadout = Some((*index, *hidden_accessories)),
                            _ => return Err(ProtocolError::Invalid("active update")),
                        }
                        self.send(Some(slot), p.encode()?);
                    }
                }
            }
        }
        Ok(())
    }
}

fn tile_break_result(position: [i16; 2], remains: bool) -> Result<Bytes, ProtocolError> {
    Packet::TileChange {
        action: 20,
        position,
        value: i16::from(remains),
        style: 0,
    }
    .encode()
}

fn soil_under_grass(block: u16) -> Option<u16> {
    match block {
        2 | 23 | 109 | 199 | 477 | 492 => Some(0),
        60 | 70 | 661 | 662 => Some(59),
        633 => Some(57),
        _ => None,
    }
}

fn wall_item(wall: u16) -> Option<i16> {
    match wall {
        1 => Some(26),
        4 => Some(93),
        5 => Some(130),
        6 => Some(132),
        16 => Some(30),
        21 => Some(392),
        34 => Some(608),
        41 => Some(622),
        42 => Some(623),
        43 => Some(624),
        85 => Some(927),
        147 => Some(2433),
        149 => Some(2505),
        151 => Some(2506),
        316 => Some(5216),
        _ => None,
    }
}

// Direct single-block drops from release 326 WorldGen.KillTile_GetItemDrops.
fn resource(block: u16) -> Option<i16> {
    Some(match block {
        0 | 2 => 2,
        1 => 3,
        6..=9 => block as i16 + 5,
        30 => 9,
        40 => 133,
        59 => 176,
        161 => 664,
        166..=169 => block as i16 + 533,
        _ => return None,
    })
}

fn update(frame: &terraria_protocol::Frame, slot: u8) -> Result<Packet, ProtocolError> {
    if !matches!(
        frame.id,
        5 | 12
            | 13
            | 16
            | 17
            | 19
            | 21
            | 27
            | 29
            | 30
            | 31
            | 32
            | 33
            | 34
            | 36
            | 39
            | 42
            | 45
            | 46
            | 47
            | 48
            | 50
            | 51
            | 56
            | 59
            | 69
            | 79
            | 82
            | 117
            | 118
            | 147
            | 151
            | 154
            | 159
    ) {
        return Err(ProtocolError::Unknown(frame.id));
    }
    let mut packet = Packet::decode_client(frame)?;
    match &mut packet {
        Packet::PlayerZones { player, .. }
        | Packet::UseTimeDial { player, .. }
        | Packet::PlayerPvp { player, .. }
        | Packet::PlayerTeam { player, .. } => *player = slot,
        Packet::TileChange { .. }
        | Packet::ObjectPlacement { .. }
        | Packet::DoorToggle { .. }
        | Packet::ProjectileSync(_)
        | Packet::ProjectileRemove(_)
        | Packet::RequestSection(_) => {}
        Packet::DroppedItem(i)
            if (0..=terraria_protocol::NEW_ITEM).contains(&i.slot)
                && (0..6196).contains(&i.item)
                && i.stack >= 0 => {}
        Packet::ReleaseItem { slot: index, .. } | Packet::RemoveItem(index)
            if (0..terraria_protocol::NEW_ITEM).contains(index) => {}
        Packet::ChestUpdate { .. }
        | Packet::RequestChestName { .. }
        | Packet::RequestNpcName(_)
        | Packet::RequestChest(_)
        | Packet::RequestSign(_)
        | Packet::HitSwitch(_)
        | Packet::SignText { .. }
        | Packet::LiquidUpdate(_)
        | Packet::ChestState { .. } => {}
        Packet::ChestItem {
            stack,
            item,
            prefix,
            ..
        } if *stack >= 0 && (0..6196).contains(item) => {
            if *stack == 0 || *item == 0 {
                *stack = 0;
                *item = 0;
                *prefix = 0;
            }
        }
        Packet::Spawn(s) if s.team <= 5 => s.player = slot,
        Packet::PlayerHurt(_) => {}
        Packet::PlayerDeath(d) => d.player = slot,
        Packet::Ping => {}
        Packet::Movement(m) => m.player = slot,
        Packet::Life(v) => {
            v.player = slot;
            v.maximum = v.maximum.max(20);
        }
        Packet::Mana(v) => v.player = slot,
        Packet::Buffs { player, .. } => *player = slot,
        Packet::Equipment(e) if e.valid() => e.player = slot,
        Packet::Loadout { player, index, .. } if *index < 3 => *player = slot,
        Packet::ChatCommand { command, text }
            if matches!(command.as_str(), "Say" | "Party") && text.len() <= 65520 => {}
        _ => return Err(ProtocolError::Invalid("active player update")),
    }
    Ok(packet)
}

pub(super) async fn play(
    stream: &mut TcpStream,
    slot: u8,
    character: Character,
    events: &mpsc::Sender<Event>,
    world: &std::sync::Arc<LoadedWorld>,
    sent: BTreeSet<[u16; 2]>,
    revision: u64,
) -> Result<(), Error> {
    let (sender, mut pending) = mpsc::channel::<Outbound>(64);
    events
        .send(Event::Join {
            slot,
            character: Box::new(character),
            out: sender,
            sent,
            revision,
        })
        .await
        .map_err(|_| io::Error::other("roster closed"))?;
    let (mut r, mut w) = stream.split();
    let read = async {
        let mut reported_ids = [false; 166];
        let mut reported_modules = std::collections::BTreeSet::new();
        while let Some(frame) = read_frame(&mut r).await? {
            if frame.id == 82
                && let Some(header) = frame.payload.first_chunk::<2>()
            {
                let module = u16::from_le_bytes(*header);
                if module != 1 {
                    if reported_modules.len() < 32 && reported_modules.insert(module) {
                        tracing::info!(slot, module, "ignoring unimplemented active net module");
                    }
                    continue;
                }
            }
            let packet = match update(&frame, slot) {
                Ok(p) => p,
                Err(ProtocolError::Unknown(id)) if (1..=165).contains(&id) => {
                    if !std::mem::replace(&mut reported_ids[usize::from(id)], true) {
                        tracing::info!(
                            slot,
                            packet_id = id,
                            "unimplemented active packet discarded"
                        );
                    }
                    continue;
                }
                Err(e) => {
                    tracing::warn!(slot,packet_id=frame.id,error=%e,"active packet rejected");
                    return Err(e.into());
                }
            };
            events
                .send(Event::Update { slot, packet })
                .await
                .map_err(|_| io::Error::other("roster closed"))?;
        }
        Ok::<_, Error>(())
    };
    let write = async {
        while let Some(message) = pending.recv().await {
            let write = async {
                match message {
                    Outbound::Bytes(bytes) => w.write_all(&bytes).await?,
                    Outbound::Section(snapshot) => world.send_section(&mut w, snapshot).await?,
                }
                Ok::<_, Error>(())
            };
            timeout(Duration::from_secs(10), write)
                .await
                .map_err(io::Error::other)??;
        }
        Ok(())
    };
    let result = tokio::select! {r=read=>r,r=write=>r};
    if matches!(result, Err(Error::Protocol(_))) {
        let _ = timeout(
            Duration::from_secs(1),
            write_packet(
                stream,
                &Packet::Kick(NetworkText::key("LegacyMultiplayer.2")),
            ),
        )
        .await;
    }
    result
}
