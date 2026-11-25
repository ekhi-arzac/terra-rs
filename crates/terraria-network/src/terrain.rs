use crate::{Error, write_packet};
use bytes::{Bytes, BytesMut};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, RwLock},
};
use terraria_protocol::Error as ProtocolError;
use terraria_protocol::{
    NetworkText, Packet, WorldData,
    section::{SECTION_HEIGHT, SECTION_WIDTH},
};
use terraria_protocol::{TilePatch, section::section_tile};
use terraria_world::{ChestStack, Tile, World};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::Semaphore,
};

pub(super) struct LoadedWorld {
    pub data: WorldData,
    time_rate: u32,
    pub residents: Vec<crate::residents::Resident>,
    join_state: bytes::Bytes,
    source: World,
    workers: Arc<Semaphore>,
    edits: RwLock<WorldEdits>,
}

pub(super) struct SectionSnapshot {
    index: [u16; 2],
    tiles: Vec<([i16; 2], Tile)>,
    chests: Bytes,
    chest_objects: Vec<terraria_protocol::section::SectionObject>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TileBounds {
    start: [i16; 2],
    end: [i16; 2],
}
impl TileBounds {
    fn new(pos: [i16; 2]) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    fn include(&mut self, pos: [i16; 2]) {
        for axis in 0..2 {
            self.start[axis] = self.start[axis].min(pos[axis]);
            self.end[axis] = self.end[axis].max(pos[axis]);
        }
    }
}
struct TileEdit {
    revision: u64,
    tile: Tile,
    footprint: Option<TileBounds>,
}

#[derive(Default)]
struct WorldEdits {
    clock: terraria_world::WorldClock,
    phase_revision: u64,
    tiles: BTreeMap<[u16; 2], BTreeMap<[i16; 2], TileEdit>>,
    revision: u64,
    containers: crate::containers::Containers,
}

/// Owned by the single disk writer. Readers continue using the original world and live edits.
#[derive(Default)]
pub(super) struct Checkpoint {
    pub(super) revision: u64,
    published: Option<World>,
}

impl LoadedWorld {
    pub fn save_changes(
        &self,
        path: &std::path::Path,
        checkpoint: &mut Checkpoint,
    ) -> Result<bool, Error> {
        let edits = self.edits.read().unwrap();
        let revision = edits.revision;
        if revision == checkpoint.revision {
            return Ok(false);
        }
        let blocks = edits
            .tiles
            .values()
            .flat_map(|s| s.iter())
            .filter(|(_, edit)| edit.revision > checkpoint.revision)
            .map(|(&pos, edit)| (pos, edit.tile))
            .collect();
        let containers = edits.containers.changes(checkpoint.revision);
        let clock = edits.clock;
        drop(edits);
        let base = checkpoint.published.as_ref().unwrap_or(&self.source);
        let updated = base.checkpoint(&blocks, &containers, clock)?;
        base.publish_edits(&updated, path)?;
        checkpoint.published = Some(updated);
        checkpoint.revision = revision;
        tracing::info!(world=%path.display(), tiles=blocks.len(),chests=containers.len(),"saved world changes");
        Ok(true)
    }

    pub fn new(source: World) -> Result<Self, Error> {
        let state = source.join_state()?;
        let data = state.info;
        let time_rate = source.saved_time_rate()?;
        let clock = terraria_world::WorldClock::from_world(source.metadata().time, &data);
        let mut join_state = bytes::BytesMut::new();
        let residents = crate::residents::Resident::load(&source)?;
        for resident in &residents {
            join_state.extend_from_slice(&Packet::NpcSync(resident.sync).encode()?);
            join_state.extend_from_slice(
                &Packet::NpcBuffs {
                    slot: i16::from(resident.sync.slot),
                    buffs: Vec::new(),
                }
                .encode()?,
            );
        }
        for p in [
            Packet::BannerState(Box::new(state.banners)),
            Packet::TowerShields(state.shields),
            Packet::CavernMonsters(state.cavern_monsters),
        ] {
            join_state.extend_from_slice(&p.encode()?);
        }
        let workers = Arc::new(Semaphore::new(
            std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(4),
        ));
        let containers = crate::containers::Containers::load(&source);
        Ok(Self {
            data,
            time_rate,
            residents,
            join_state: join_state.freeze(),
            edits: RwLock::new(WorldEdits {
                containers,
                clock,
                ..Default::default()
            }),
            source,
            workers,
        })
    }
    pub fn current_time(&self, info: &mut WorldData) -> u64 {
        let edits = self.edits.read().unwrap();
        edits.clock.apply(info);
        edits.revision
    }
    pub fn advance_time(&self) -> Option<Packet> {
        let mut edits = self.edits.write().unwrap();
        let previous = edits.clock;
        let transition = edits.clock.tick(self.time_rate);
        if edits.clock == previous {
            return None;
        }
        edits.revision += 1;
        if !transition {
            return None;
        }
        Some(self.clock_update(&mut edits))
    }
    pub fn activate_dial(&self, dusk: bool) -> Option<Packet> {
        let mut edits = self.edits.write().unwrap();
        if !edits.clock.activate_dial(dusk) {
            return None;
        }
        edits.revision += 1;
        Some(self.clock_update(&mut edits))
    }
    fn clock_update(&self, edits: &mut WorldEdits) -> Packet {
        edits.phase_revision = edits.revision;
        let mut data = self.data.clone();
        edits.clock.apply(&mut data);
        Packet::WorldData(Box::new(data))
    }
    pub fn section_at([x, y]: [i16; 2]) -> [u16; 2] {
        [x as u16 / SECTION_WIDTH, y as u16 / SECTION_HEIGHT]
    }
    pub fn tile(&self, pos: [i16; 2]) -> Option<Tile> {
        self.tile_at(pos, &self.edits.read().unwrap())
    }

    fn tile_at(&self, pos: [i16; 2], edits: &WorldEdits) -> Option<Tile> {
        if let Some(edit) = edits
            .tiles
            .get(&Self::section_at(pos))
            .and_then(|s| s.get(&pos))
        {
            return Some(edit.tile);
        }
        self.source
            .tile(u32::try_from(pos[0]).ok()?, u32::try_from(pos[1]).ok()?)
            .ok()
    }

    pub fn with_block(mut tile: Tile, block: Option<u16>) -> Tile {
        tile.block = block;
        tile.frame = None;
        tile.block_paint = 0;
        tile.flags[1] &= !0x70; // slope and half-brick
        tile.flags[2] &= !4; // inactive block
        tile.flags[3] &= !0x0a; // block invisibility and fullbright
        tile
    }
    pub fn commit_tiles(&self, changes: &[([i16; 2], Tile)], bounds: Option<TileBounds>) {
        let mut edits = self.edits.write().unwrap();
        Self::record_tiles(&mut edits, changes, bounds);
    }
    pub fn commit_chest(
        &self,
        origin: [i16; 2],
        place: bool,
        changes: &[([i16; 2], Tile)],
        bounds: TileBounds,
    ) -> Option<i16> {
        let mut edits = self.edits.write().unwrap();
        let rev = edits.revision + 1;
        let id = if place {
            edits.containers.create(origin, rev)
        } else {
            edits.containers.remove_empty(origin, rev)
        }?;
        Self::record_tiles(&mut edits, changes, Some(bounds));
        Some(id)
    }
    fn record_tiles(
        edits: &mut WorldEdits,
        changes: &[([i16; 2], Tile)],
        bounds: Option<TileBounds>,
    ) {
        edits.revision += 1;
        let revision = edits.revision;
        for &(pos, tile) in changes {
            let section = edits.tiles.entry(Self::section_at(pos)).or_default();
            // Wall changes can preserve a recently placed station's block and frame.
            let footprint = bounds.or_else(|| {
                let old = section.get(&pos)?;
                if old.tile.block == tile.block && old.tile.frame == tile.frame {
                    old.footprint
                } else {
                    None
                }
            });
            section.insert(
                pos,
                TileEdit {
                    revision,
                    tile,
                    footprint,
                },
            );
        }
    }

    pub fn tile_patch(pos: [i16; 2], tile: Tile) -> Result<Bytes, ProtocolError> {
        Packet::TilePatch(Box::new(TilePatch {
            origin: pos,
            size: [1, 1],
            reason: 0,
            tiles: vec![section_tile(tile)],
        }))
        .encode()
    }

    /// Fill the small rectangle around each part of the edit using the proposed tiles.
    pub fn patches(
        &self,
        changes: &[([i16; 2], Tile)],
    ) -> Result<Vec<([u16; 2], Bytes)>, ProtocolError> {
        let changed: BTreeMap<_, _> = changes.iter().copied().collect();
        let mut rectangles = BTreeMap::new();
        for &pos in changed.keys() {
            rectangles
                .entry(Self::section_at(pos))
                .or_insert_with(|| TileBounds::new(pos))
                .include(pos);
        }
        let edits = self.edits.read().unwrap();
        rectangles
            .into_iter()
            .map(|(section, bounds)| {
                Ok((section, self.encode_rectangle(bounds, &edits, &changed)?))
            })
            .collect()
    }

    /// Multi-tile objects need one patch even when their footprint straddles two sections.
    pub fn footprint_patch(
        &self,
        changes: &[([i16; 2], Tile)],
    ) -> Result<(TileBounds, Bytes), ProtocolError> {
        let changed: BTreeMap<_, _> = changes.iter().copied().collect();
        let mut bounds = TileBounds::new(
            *changed
                .first_key_value()
                .ok_or(ProtocolError::Invalid("empty footprint"))?
                .0,
        );
        for &pos in changed.keys() {
            bounds.include(pos);
        }
        Ok((
            bounds,
            self.encode_rectangle(bounds, &self.edits.read().unwrap(), &changed)?,
        ))
    }

    fn encode_rectangle(
        &self,
        bounds: TileBounds,
        edits: &WorldEdits,
        changed: &BTreeMap<[i16; 2], Tile>,
    ) -> Result<Bytes, ProtocolError> {
        let [x, y] = bounds.start;
        let [right, bottom] = bounds.end;
        let size = [
            i32::from(right) - i32::from(x) + 1,
            i32::from(bottom) - i32::from(y) + 1,
        ]
        .map(u8::try_from);
        let [Ok(width), Ok(height)] = size else {
            return Err(ProtocolError::Invalid("terrain patch dimensions"));
        };
        let mut tiles = Vec::with_capacity(usize::from(width) * usize::from(height));
        for column in x..=right {
            for row in y..=bottom {
                let pos = [column, row];
                let tile = changed
                    .get(&pos)
                    .copied()
                    .or_else(|| self.tile_at(pos, edits))
                    .ok_or(ProtocolError::Invalid("terrain patch coordinate"))?;
                tiles.push(section_tile(tile));
            }
        }
        Packet::TilePatch(Box::new(TilePatch {
            origin: bounds.start,
            size: [width, height],
            reason: 0,
            tiles,
        }))
        .encode()
    }

    pub fn chest_section(&self, chest: i16) -> Option<[u16; 2]> {
        self.edits
            .read()
            .unwrap()
            .containers
            .position(chest)
            .map(Self::section_at)
    }
    pub fn chest_at(&self, pos: [i16; 2]) -> Option<i16> {
        self.edits.read().unwrap().containers.at(pos)
    }
    pub fn chest_name(&self, id: i16) -> Option<Packet> {
        self.edits.read().unwrap().containers.name(id)
    }
    pub fn rename_chest(&self, id: i16, name: String) -> Option<Packet> {
        let mut edits = self.edits.write().unwrap();
        let rev = edits.revision + 1;
        let packet = edits.containers.rename(id, name, rev)?;
        edits.revision = rev;
        Some(packet)
    }
    pub fn revision(&self) -> u64 {
        self.edits.read().unwrap().revision
    }
    pub fn catch_up(
        &self,
        since: u64,
        sent: &std::collections::BTreeSet<[u16; 2]>,
        out: &mut BytesMut,
    ) -> Result<(), ProtocolError> {
        let edits = self.edits.read().unwrap();
        if since == edits.revision {
            return Ok(());
        }
        if edits.phase_revision > since {
            let mut data = self.data.clone();
            edits.clock.apply(&mut data);
            out.extend_from_slice(&Packet::WorldData(Box::new(data)).encode()?);
        }
        edits.containers.removed_since(since, sent, out)?;
        let mut footprints = BTreeSet::new();
        for index in sent {
            if let Some(changes) = edits.tiles.get(index) {
                for (&pos, edit) in changes {
                    if edit.revision > since {
                        if let Some(bounds) = edit.footprint {
                            footprints.insert(bounds);
                        } else {
                            out.extend_from_slice(&Self::tile_patch(pos, edit.tile)?);
                        }
                    }
                }
            }
        }
        for bounds in footprints {
            out.extend_from_slice(&self.encode_rectangle(bounds, &edits, &BTreeMap::new())?);
        }
        edits.containers.catch_up(since, sent, out)
    }
    pub fn edit_chest(&self, id: i16, slot: u8, value: ChestStack) -> bool {
        let mut edits = self.edits.write().unwrap();
        let rev = edits.revision + 1;
        if !edits.containers.edit_slot(id, slot, value, rev) {
            return false;
        }
        edits.revision = rev;
        true
    }
    pub fn chest_contents(&self, id: i16) -> Result<BytesMut, ProtocolError> {
        let mut batch = BytesMut::new();
        self.edits
            .read()
            .unwrap()
            .containers
            .write_contents(id, &mut batch)?;
        Ok(batch)
    }
    pub fn contains(&self, index: [u16; 2]) -> bool {
        u32::from(index[0]) * u32::from(SECTION_WIDTH) < self.source.metadata().width
            && u32::from(index[1]) * u32::from(SECTION_HEIGHT) < self.source.metadata().height
    }
    pub fn section_snapshot(&self, index: [u16; 2]) -> Result<SectionSnapshot, ProtocolError> {
        let edits = self.edits.read().unwrap();
        let tiles = edits
            .tiles
            .get(&index)
            .into_iter()
            .flatten()
            .map(|(&pos, edit)| (pos, edit.tile))
            .collect();
        let mut chests = BytesMut::new();
        let chest_objects = edits.containers.section_snapshot(index, &mut chests)?;
        Ok(SectionSnapshot {
            index,
            tiles,
            chests: chests.freeze(),
            chest_objects,
        })
    }

    pub async fn send_section<W: AsyncWrite + Unpin>(
        self: &Arc<Self>,
        out: &mut W,
        snapshot: SectionSnapshot,
    ) -> Result<(), Error> {
        let permit = Arc::clone(&self.workers)
            .acquire_owned()
            .await
            .map_err(io::Error::other)?;
        let source = Arc::clone(self);
        let SectionSnapshot {
            index,
            tiles,
            chests,
            chest_objects,
        } = snapshot;
        let frame = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut section = source.source.section_with_edits(index, &tiles)?;
            section.chests = chest_objects;
            Ok::<_, Error>(Packet::TileSection(Box::new(section)).encode()?)
        })
        .await??;
        out.write_all(&frame).await?;
        out.write_all(&chests).await?;
        for resident in &self.residents {
            let tile = resident.sync.position.map(|p| (p / 16.) as i16);
            if Self::section_at(tile) == index {
                write_packet(out, &Packet::NpcSync(resident.sync)).await?;
            }
        }
        Ok(())
    }
    pub async fn transfer<W: AsyncWrite + Unpin>(
        self: &Arc<Self>,
        out: &mut W,
        spawn: [i32; 2],
        team: u8,
    ) -> Result<std::collections::BTreeSet<[u16; 2]>, Error> {
        let sections = initial_sections(&self.data, spawn, team);
        write_packet(
            out,
            &Packet::Status {
                count: sections.len() as i32,
                text: NetworkText::key("LegacyInterface.44"),
                flags: 0,
            },
        )
        .await?;
        for &index in &sections {
            self.send_section(out, self.section_snapshot(index)?)
                .await?;
        }
        out.write_all(&self.join_state).await?;
        write_packet(out, &Packet::SpawnAllowed).await?;
        Ok(sections.into_iter().collect())
    }
}

// Ordinary single-cell grass and background plants.
pub(super) fn is_solid_floor(tile: Tile) -> bool {
    // Common solid terrain, brick and platforms from the 1.4.6 tile tables.
    tile.flags[1] & 0x70 == 0
        && tile.flags[2] & 4 == 0
        && matches!(
            tile.block,
            Some(
                0..=2
                | 6..=9
                | 19
                | 23
                | 25
                | 30
                | 38..=41
                | 43..=47
                | 53
                | 56
                | 57
                | 59
                | 60
                | 70
                | 75
                | 76
                | 109
                | 112
                | 116..=119
                | 147
                | 161
                | 163
                | 164
                | 166..=169
                | 199
                | 633
                | 661
                | 662,
            )
        )
}

pub(super) fn is_ground_cover(block: u16) -> bool {
    matches!(
        block,
        3 | 24 | 61 | 71 | 73 | 74 | 110 | 113 | 201 | 530 | 637
    )
}

fn initial_sections(data: &WorldData, personal: [i32; 2], team: u8) -> Vec<[u16; 2]> {
    let [width, height] = data.size.map(i32::from);
    let step_x = i32::from(SECTION_WIDTH);
    let step_y = i32::from(SECTION_HEIGHT);
    let mut sections = Vec::new();
    let mut add = |[x, y]: [i32; 2], additional: i32| {
        let x = x / step_x - 2;
        let y = y / step_y - 1;
        for sx in x.max(0)..(x + 5 + additional).min((width + step_x - 1) / step_x) {
            for sy in y.max(0)..(y + 3 + additional).min((height + step_y - 1) / step_y) {
                let index = [sx as u16, sy as u16];
                if !sections.contains(&index) {
                    sections.push(index);
                }
            }
        }
    };
    add(data.spawn.map(i32::from), 0);
    if (10..=width - 10).contains(&personal[0]) && (10..=height - 10).contains(&personal[1]) {
        add(personal, 1);
    }
    if team != 0
        && data.progression[9] & 32 != 0
        && let Some(point) = data.extra_spawns.get(usize::from(team))
    {
        add(point.map(i32::from), 1);
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catch_up_keeps_a_station_in_one_patch_across_sections() {
        let source =
            World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
        let world = LoadedWorld::new(source).unwrap();
        let left = [2199, 233];
        let right = [2200, 233];
        let a = Tile {
            block: Some(18),
            frame: Some((0, 0)),
            ..Tile::default()
        };
        let b = Tile {
            frame: Some((18, 0)),
            ..a
        };
        let changes = [(left, a), (right, b)];
        let (footprint, _) = world.footprint_patch(&changes).unwrap();
        world.commit_tiles(&changes, Some(footprint));
        world.commit_tiles(&[(left, Tile { wall: 4, ..a })], None);
        let mut output = BytesMut::new();
        world
            .catch_up(
                0,
                &BTreeSet::from([LoadedWorld::section_at(left)]),
                &mut output,
            )
            .unwrap();
        let frame = terraria_protocol::Frame::split(&mut output)
            .unwrap()
            .unwrap();
        let Packet::TilePatch(patch) = Packet::decode(&frame).unwrap() else {
            panic!("expected station patch")
        };
        assert_eq!((patch.origin, patch.size), (left, [2, 1]));
        assert_eq!(patch.tiles, vec![Tile { wall: 4, ..a }, b]);
        assert!(output.is_empty());
    }

    #[test]
    fn spawn_rectangles_clip_at_world_edges_and_deduplicate_personal_and_team_points() {
        let world =
            World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
        let mut data = world.world_data().unwrap();
        data.spawn = [0, 0];
        let expected: Vec<_> = (0..3).flat_map(|x| (0..2).map(move |y| [x, y])).collect();
        assert_eq!(initial_sections(&data, [i32::MAX, i32::MIN], 255), expected);
        data.progression[9] |= 32;
        data.extra_spawns = vec![[0, 0], [410, 465]];
        let team = initial_sections(&data, [-1, -1], 1);
        assert_eq!(team, initial_sections(&data, [410, 465], 1));
        assert_eq!(team.len(), 30);
        assert_eq!(team.last(), Some(&[5, 5]));
    }
}
