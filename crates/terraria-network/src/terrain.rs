use crate::{Error, write_packet};
use std::{io, sync::Arc};
use terraria_protocol::{
    NetworkText, Packet, WorldData,
    section::{SECTION_HEIGHT, SECTION_WIDTH},
};
use terraria_world::World;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::Semaphore,
};

pub(super) struct LoadedWorld {
    pub data: WorldData,
    join_state: bytes::Bytes,
    source: World,
    workers: Arc<Semaphore>,
}

impl LoadedWorld {
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
        Ok(Self {
            data,
            join_state: join_state.freeze(),
            source,
            workers,
        })
    }
    pub fn contains(&self, index: [u16; 2]) -> bool {
        u32::from(index[0]) * u32::from(SECTION_WIDTH) < self.source.metadata().width
            && u32::from(index[1]) * u32::from(SECTION_HEIGHT) < self.source.metadata().height
    }
    pub async fn send_section<W: AsyncWrite + Unpin>(
        self: &Arc<Self>,
        out: &mut W,
        index: [u16; 2],
    ) -> Result<(), Error> {
        let permit = Arc::clone(&self.workers)
            .acquire_owned()
            .await
            .map_err(io::Error::other)?;
        let source = Arc::clone(self);
        let (frame, mut indices) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let section = source.source.section(index)?;
            let indices: Vec<_> = section.chests.iter().map(|c| c.index).collect();
            Ok::<_, Error>((Packet::TileSection(Box::new(section)).encode()?, indices))
        })
        .await??;
        out.write_all(&frame).await?;
        indices.sort_unstable();
        for index in indices {
            let mut batch = Vec::new();
            let chest = &self.source.chests()[index as usize];
            if chest.capacity > 256 {
                return Err(
                    terraria_world::Error::Invalid("chest slots exceed protocol capacity").into(),
                );
            }
            batch.extend(
                Packet::ChestSize {
                    index,
                    capacity: chest.capacity as i16,
                }
                .encode()?,
            );
            let mut items = chest.items.iter().peekable();
            for slot in 0..chest.capacity {
                let item = items.next_if(|item| item.slot == slot);
                let item_id = item
                    .map_or(Ok(0), |v| i16::try_from(v.item_type))
                    .map_err(|_| {
                        terraria_world::Error::Invalid("chest item ID exceeds protocol range")
                    })?;
                batch.extend(
                    Packet::ChestItem {
                        chest: index,
                        slot: slot as u8,
                        stack: item.map_or(0, |v| v.stack),
                        prefix: item.map_or(0, |v| v.prefix),
                        item: item_id,
                    }
                    .encode()?,
                );
            }
            out.write_all(&batch).await?;
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
            self.send_section(out, index).await?;
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
