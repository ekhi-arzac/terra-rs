use crate::{Error, write_packet};
use bytes::{Bytes, BytesMut};
use std::{
    collections::BTreeMap,
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
    join_state: bytes::Bytes,
    source: World,
    workers: Arc<Semaphore>,
    edits: RwLock<WorldEdits>,
    chests_by_section: BTreeMap<[u16; 2], Vec<i16>>,
}

pub(super) struct SectionSnapshot {
    index: [u16; 2],
    tiles: Vec<([i16; 2], Tile)>,
    chests: Bytes,
}
type ChangedTiles = BTreeMap<[i16; 2], (u64, Tile)>;

#[derive(Default)]
struct WorldEdits {
    tiles: BTreeMap<[u16; 2], ChangedTiles>,
    revision: u64,
    slots: BTreeMap<(i16, u8), (u64, ChestStack)>,
}

fn chest_item(chest: i16, slot: u8, value: ChestStack) -> Packet {
    Packet::ChestItem {
        chest,
        slot,
        stack: value.stack,
        prefix: value.prefix,
        item: value.item,
    }
}

impl LoadedWorld {
    pub fn save_changes(&self, path: &std::path::Path) -> Result<(), Error> {
        let edits = self.edits.read().unwrap();
        if edits.revision == 0 {
            return Ok(());
        }
        let blocks = edits
            .tiles
            .values()
            .flat_map(|s| s.iter().map(|(&pos, &(_, tile))| (pos, tile)))
            .collect();
        let slots = edits.slots.iter().map(|(&key, &(_, v))| (key, v)).collect();
        drop(edits);
        let updated = self.source.with_edits(&blocks, &slots)?;
        self.source.publish_edits(&updated, path)?;
        tracing::info!(world=%path.display(), tiles=blocks.len(),chest_slots=slots.len(),"saved world changes");
        Ok(())
    }

    pub fn new(source: World) -> Result<Self, Error> {
        let state = source.join_state()?;
        let data = state.info;
        let mut join_state = bytes::BytesMut::new();
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
        let mut chests_by_section = BTreeMap::<_, Vec<i16>>::new();
        for (index, chest) in source.chests().iter().enumerate() {
            let (x, y) = chest.position;
            if (0..i32::from(data.size[0])).contains(&x)
                && (0..i32::from(data.size[1])).contains(&y)
            {
                chests_by_section
                    .entry(Self::section_at([x as i16, y as i16]))
                    .or_default()
                    .push(index as i16);
            }
        }
        Ok(Self {
            data,
            chests_by_section,
            join_state: join_state.freeze(),
            edits: RwLock::default(),
            source,
            workers,
        })
    }
    pub fn section_at([x, y]: [i16; 2]) -> [u16; 2] {
        [x as u16 / SECTION_WIDTH, y as u16 / SECTION_HEIGHT]
    }
    pub fn tile(&self, pos: [i16; 2]) -> Option<Tile> {
        if let Some((_, tile)) = self
            .edits
            .read()
            .unwrap()
            .tiles
            .get(&Self::section_at(pos))
            .and_then(|s| s.get(&pos))
        {
            return Some(*tile);
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
    pub fn commit_tiles(&self, changes: &[([i16; 2], Tile)]) {
        let mut edits = self.edits.write().unwrap();
        edits.revision += 1;
        let rev = edits.revision;
        for &(pos, tile) in changes {
            edits
                .tiles
                .entry(Self::section_at(pos))
                .or_default()
                .insert(pos, (rev, tile));
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
        let mut rectangles = BTreeMap::<_, ([i16; 2], [i16; 2])>::new();
        for &(pos, _) in changes {
            let (start, end) = rectangles
                .entry(Self::section_at(pos))
                .or_insert((pos, pos));
            for axis in 0..2 {
                start[axis] = start[axis].min(pos[axis]);
                end[axis] = end[axis].max(pos[axis]);
            }
        }
        let edits = self.edits.read().unwrap();
        rectangles
            .into_iter()
            .map(|(index, (origin, end))| {
                let size = [
                    (end[0] - origin[0] + 1) as u8,
                    (end[1] - origin[1] + 1) as u8,
                ];
                let mut tiles = Vec::with_capacity(usize::from(size[0]) * usize::from(size[1]));
                let edited = edits.tiles.get(&index);
                for x in origin[0]..=end[0] {
                    for y in origin[1]..=end[1] {
                        let tile = changed
                            .get(&[x, y])
                            .copied()
                            .or_else(|| edited.and_then(|s| s.get(&[x, y])).map(|&(_, tile)| tile))
                            .or_else(|| self.source.tile(x as u32, y as u32).ok())
                            .ok_or(ProtocolError::Invalid("committed terrain coordinate"))?;
                        tiles.push(section_tile(tile));
                    }
                }
                let bytes = Packet::TilePatch(Box::new(TilePatch {
                    origin,
                    size,
                    reason: 0,
                    tiles,
                }))
                .encode()?;
                Ok((index, bytes))
            })
            .collect()
    }

    pub fn chest_section(&self, chest: i16) -> Option<[u16; 2]> {
        let (x, y) = self.source.chests().get(chest as usize)?.position;
        Some(Self::section_at([x as i16, y as i16]))
    }

    pub fn chest_at(&self, position: [i16; 2]) -> Option<i16> {
        self.source
            .chests()
            .iter()
            .position(|c| c.position == (position[0].into(), position[1].into()))
            .map(|i| i as i16)
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
        for index in sent {
            if let Some(changes) = edits.tiles.get(index) {
                for (&pos, &(revision, tile)) in changes {
                    if revision > since {
                        out.extend_from_slice(&Self::tile_patch(pos, tile)?);
                    }
                }
            }
        }
        for (&(index, slot), (rev, value)) in &edits.slots {
            let (x, y) = self.source.chests()[index as usize].position;
            if *rev > since && sent.contains(&[x as u16 / SECTION_WIDTH, y as u16 / SECTION_HEIGHT])
            {
                out.extend_from_slice(&chest_item(index, slot, *value).encode()?);
            }
        }
        Ok(())
    }
    pub fn edit_chest(&self, index: i16, slot: u8, value: ChestStack) -> bool {
        if !self
            .source
            .chests()
            .get(index as usize)
            .is_some_and(|c| u32::from(slot) < c.capacity)
        {
            return false;
        }
        let mut edits = self.edits.write().unwrap();
        edits.revision += 1;
        let rev = edits.revision;
        edits.slots.insert((index, slot), (rev, value));
        true
    }
    pub fn chest_contents(&self, index: i16) -> Result<BytesMut, ProtocolError> {
        let edits = self.edits.read().unwrap();
        let mut batch = BytesMut::new();
        self.write_chest(index, &edits, &mut batch)?;
        Ok(batch)
    }

    fn write_chest(
        &self,
        index: i16,
        edits: &WorldEdits,
        batch: &mut BytesMut,
    ) -> Result<(), ProtocolError> {
        let chest = &self.source.chests()[index as usize];
        if chest.capacity > 256 {
            return Err(ProtocolError::Invalid("chest capacity"));
        }
        batch.extend_from_slice(
            &Packet::ChestSize {
                index,
                capacity: chest.capacity as i16,
            }
            .encode()?,
        );
        let mut items = chest.items.iter().peekable();
        for slot in 0..chest.capacity {
            let item = items.next_if(|v| v.slot == slot);
            if let Some((_, value)) = edits.slots.get(&(index, slot as u8)) {
                batch.extend_from_slice(&chest_item(index, slot as u8, *value).encode()?);
            } else {
                batch.extend_from_slice(
                    &Packet::ChestItem {
                        chest: index,
                        slot: slot as u8,
                        item: item
                            .map_or(Ok(0), |v| i16::try_from(v.item_type))
                            .map_err(|_| ProtocolError::Invalid("chest item type"))?,
                        stack: item.map_or(0, |v| v.stack),
                        prefix: item.map_or(0, |v| v.prefix),
                    }
                    .encode()?,
                );
            }
        }
        Ok(())
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
            .map(|(&pos, &(_, tile))| (pos, tile))
            .collect();
        let mut chests = BytesMut::new();
        for &chest in self.chests_by_section.get(&index).into_iter().flatten() {
            self.write_chest(chest, &edits, &mut chests)?;
        }
        Ok(SectionSnapshot {
            index,
            tiles,
            chests: chests.freeze(),
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
        } = snapshot;
        let frame = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let section = source.source.section_with_edits(index, &tiles)?;
            Ok::<_, Error>(Packet::TileSection(Box::new(section)).encode()?)
        })
        .await??;
        out.write_all(&frame).await?;
        out.write_all(&chests).await?;
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
