//! Mutating probe for a disposable copy of a world with a known chest.
use super::{
    Result,
    peers::{join, observe},
};
use std::net::SocketAddr;
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{DroppedItem, Movement, NEW_ITEM, Packet};
use tokio::net::TcpStream;

async fn next(stream: &mut TcpStream, id: u8) -> Result<Packet> {
    loop {
        let f = read_frame(stream).await?.ok_or("EOF during item probe")?;
        if f.id == 2 {
            return Err(format!("kick: {:?}", Packet::decode(&f)?).into());
        }
        if f.id == id {
            return Ok(Packet::decode(&f)?);
        }
    }
}
fn visible_tile(log: &[Packet], [x, y]: [i16; 2]) -> Option<terraria_world::Tile> {
    log.iter().rev().find_map(|p| {
        if let Packet::TilePatch(patch) = p {
            let dx = i32::from(x) - i32::from(patch.origin[0]);
            let dy = i32::from(y) - i32::from(patch.origin[1]);
            if (0..i32::from(patch.size[0])).contains(&dx)
                && (0..i32::from(patch.size[1])).contains(&dy)
            {
                return Some(patch.tiles[dx as usize * usize::from(patch.size[1]) + dy as usize]);
            }
        }
        let Packet::TileSection(s) = p else {
            return None;
        };
        let dx = i32::from(x) - s.origin[0];
        let dy = i32::from(y) - s.origin[1];
        if !(0..i32::from(s.size[0])).contains(&dx) || !(0..i32::from(s.size[1])).contains(&dy) {
            return None;
        }
        let mut remaining = dy as u32 * u32::from(s.size[0]) + dx as u32;
        for run in &s.runs {
            if remaining < u32::from(run.count) {
                return Some(run.tile);
            }
            remaining -= u32::from(run.count);
        }
        None
    })
}

fn matches_edit(frame: &terraria_protocol::Frame, request: &Packet) -> Result<bool> {
    let Packet::TileChange {
        position,
        action,
        value,
        ..
    } = request
    else {
        return Ok(false);
    };
    match frame.id {
        17 => Ok(Packet::decode(frame)? == *request),
        20 => Ok(
            visible_tile(&[Packet::decode(frame)?], *position).is_some_and(|tile| match action {
                0 => tile.block.is_none(),
                1 => tile.block == Some(*value as u16),
                2 => tile.wall == 0,
                3 => tile.wall == *value as u16,
                _ => false,
            }),
        ),
        _ => Ok(false),
    }
}

async fn observe_edit(stream: &mut TcpStream, edit: &Packet) -> Result<()> {
    loop {
        let frame = read_frame(stream)
            .await?
            .ok_or("EOF waiting for terrain edit")?;
        if frame.id == 2 {
            return Err("kicked while waiting for terrain".into());
        }
        if matches_edit(&frame, edit)? {
            return Ok(());
        }
    }
}

pub(super) async fn run(
    addr: SocketAddr,
    password: Option<&str>,
    position: [i16; 2],
) -> Result<()> {
    let (mut a, first, _, _) = join(addr, password).await?;
    let (mut b, second, _, _) = join(addr, password).await?;
    let point = position.map(|v| f32::from(v) * 16.0);
    for (socket, player) in [(&mut a, first), (&mut b, second)] {
        write_packet(
            socket,
            &Packet::Movement(Movement {
                player,
                flags: [0; 4],
                selected: 0,
                position: point,
                velocity: None,
                mount: None,
                return_points: None,
                camera: None,
            }),
        )
        .await?;
    }
    let requested = DroppedItem {
        slot: NEW_ITEM,
        position: point,
        velocity: [0.; 2],
        stack: 7,
        prefix: 0,
        ownership: 1,
        item: 3,
        shimmer: None,
        enemy_delay: None,
    };
    write_packet(&mut a, &Packet::DroppedItem(requested)).await?;
    let assigned = loop {
        if let Packet::DroppedItem(i) = next(&mut b, 21).await?
            && i.item == requested.item
            && i.stack == requested.stack
        {
            break i;
        }
    };
    if !(0..NEW_ITEM).contains(&assigned.slot) {
        return Err("drop has no server slot".into());
    }
    observe(&mut a, &Packet::DroppedItem(assigned)).await?;
    loop {
        if let Packet::ItemOwner(o) = next(&mut b, 22).await?
            && o.slot == assigned.slot
            && o.player == first
        {
            break;
        }
    }
    // Another player cannot take the reserved item. TCP ordering ensures the server receives
    // the removal before the ping, while the owner's subsequent update proves it still exists.
    write_packet(&mut b, &Packet::RemoveItem(assigned.slot)).await?;
    write_packet(&mut b, &Packet::Ping).await?;
    observe(&mut b, &Packet::Ping).await?;
    let partial = Packet::DroppedItem(DroppedItem {
        stack: 4,
        ..assigned
    });
    write_packet(&mut a, &partial).await?;
    observe(&mut b, &partial).await?;
    write_packet(&mut a, &Packet::RemoveItem(assigned.slot)).await?;
    observe(&mut b, &Packet::RemoveItem(assigned.slot)).await?;
    println!("Observed one allocated stack, reserved pickup, partial pickup and removal.");
    write_packet(&mut a, &Packet::RequestChest(position)).await?;
    let opened = next(&mut a, 33).await?;
    let Packet::ChestState {
        chest,
        position: actual,
        ..
    } = opened
    else {
        unreachable!()
    };
    if chest < 0 || actual != position {
        return Err("unexpected chest opened".into());
    }
    observe(
        &mut b,
        &Packet::PlayerChest {
            player: first,
            chest,
        },
    )
    .await?;
    write_packet(&mut b, &Packet::RequestChest(position)).await?;
    write_packet(&mut b, &Packet::Ping).await?;
    loop {
        let frame = read_frame(&mut b)
            .await?
            .ok_or("EOF checking chest ownership")?;
        if frame.id == 33 {
            return Err("two players opened the same chest".into());
        }
        if frame.id == 154 {
            break;
        }
    }
    let edit = Packet::ChestItem {
        chest,
        slot: 0,
        stack: 17,
        prefix: 0,
        item: 3,
    };
    write_packet(&mut a, &edit).await?;
    observe(&mut b, &edit).await?;
    write_packet(
        &mut a,
        &Packet::ChestState {
            chest: -1,
            position: [0; 2],
            name: Some("Bois 🪵".into()),
        },
    )
    .await?;
    let label = Packet::ChestName {
        chest,
        position,
        name: "Bois 🪵".into(),
    };
    observe(&mut b, &label).await?;
    observe(
        &mut b,
        &Packet::PlayerChest {
            player: first,
            chest: -1,
        },
    )
    .await?;
    write_packet(&mut b, &Packet::RequestChest(position)).await?;
    observe(&mut b, &edit).await?;
    observe(&mut b, &opened).await?;
    write_packet(
        &mut b,
        &Packet::RequestChestName {
            chest: -1,
            position,
        },
    )
    .await?;
    observe(&mut b, &label).await?;
    println!(
        "Observed chest open, slot edit, close and the same contents when the second player opened it."
    );
    // The compact oracle fixture has an empty cell next to the chest.
    let cell = [position[0] + 4, position[1] - 1];
    let place = Packet::TileChange {
        action: 1,
        position: cell,
        value: 1,
        style: 0,
    };
    write_packet(&mut a, &place).await?;
    observe_edit(&mut b, &place).await?;
    let mine = Packet::TileChange {
        action: 0,
        position: cell,
        value: 0,
        style: 0,
    };
    write_packet(&mut a, &mine).await?;
    // The reference emits the drop before the terrain message. Read both in either order.
    let mut mined = false;
    let mut resource = None;
    while !mined || resource.is_none() {
        let f = read_frame(&mut b).await?.ok_or("EOF mining stone")?;
        match f.id {
            17 | 20 => mined |= matches_edit(&f, &mine)?,
            21 => {
                if let Packet::DroppedItem(i) = Packet::decode(&f)?
                    && i.item == 3
                    && i.stack == 1
                {
                    resource = Some(i.slot);
                }
            }
            _ => {}
        }
    }
    drop(b);
    observe(
        &mut a,
        &Packet::PlayerActive {
            player: second,
            active: false,
        },
    )
    .await?;
    let mut log = Vec::new();
    let (_rejoined, _, _, _) = super::peers::join_capturing(addr, password, &mut log).await?;
    let named=log.iter().any(|p|matches!(p,Packet::TileSection(s) if s.chests.iter().any(|c| c.index==chest && c.position==position && c.text=="Bois 🪵")));
    if !named {
        return Err("rejoined chest has the wrong name".into());
    }
    if visible_tile(&log,cell).is_none_or(|tile| tile.block.is_some()) || !log.contains(&edit) || !log.iter().any(|p| matches!(p,Packet::DroppedItem(i) if Some(i.slot) == resource && i.item == 3 && i.stack == 1)) {
        return Err("rejoin did not reproduce mined cell, dropped stone and chest contents".into());
    }
    println!("Observed placement, mining, a stone drop, and the tile, chest and item on rejoin.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn items_and_chest_contents_synchronize_over_tcp() {
        let w = terraria_world::World::parse(
            include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec(),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = tokio::sync::oneshot::channel();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("probe.wld");
        w.save(&file).unwrap();
        let task = tokio::spawn(terraria_network::serve_with_save(
            listener,
            2.try_into().unwrap(),
            String::new(),
            Some(w),
            Some(file.clone()),
            async {
                let _ = end.await;
            },
        ));
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run(addr, None, [2095, 232]),
        )
        .await
        .unwrap()
        .unwrap();
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        let reloaded = terraria_world::World::load(file).unwrap();
        assert_eq!(reloaded.chests()[0].name, "Bois 🪵");
        let saved = &reloaded.chests()[0].items[0];
        assert_eq!((saved.item_type, saved.stack), (3, 17));
        assert_eq!(reloaded.tile(2099, 231).unwrap().block, None);
    }
}

#[cfg(test)]
mod tree_tests {
    use super::super::peers::join_capturing;
    use super::*;
    use std::{collections::BTreeMap, time::Duration};
    use terraria_world::{Tile, World};
    use tokio::{net::TcpListener, sync::oneshot, time::timeout};

    const GRASS: [i16; 2] = [2101, 233];
    const SOD: [i16; 2] = [2102, 234];
    const CUT: [i16; 2] = [2100, 232];
    const HARVESTED: [[i16; 2]; 4] = [CUT, [2100, 231], [2099, 231], [2100, 230]];

    fn block(ty: u16, frame: Option<(i16, i16)>) -> Tile {
        Tile {
            block: Some(ty),
            frame,
            ..Tile::default()
        }
    }
    fn visible_block(log: &[Packet], pos: [i16; 2]) -> Option<Option<u16>> {
        visible_tile(log, pos).map(|tile| tile.block)
    }

    async fn wait_for_wood(stream: &mut TcpStream) -> Result<Packet> {
        let mut patches = 0;
        loop {
            let frame = read_frame(stream).await?.ok_or("EOF waiting for wood")?;
            match frame.id {
                20 => {
                    let patch = Packet::decode(&frame)?;
                    for cell in HARVESTED {
                        assert_eq!(
                            visible_block(std::slice::from_ref(&patch), cell),
                            Some(None)
                        );
                    }
                    patches += 1;
                }
                21 => {
                    assert_eq!(patches, 1);
                    return Ok(Packet::decode(&frame)?);
                }
                10 | 17 => return Err("tree harvest sent a redundant terrain update".into()),
                _ => {}
            }
        }
    }

    async fn assert_no_terrain_update(stream: &mut TcpStream) -> Result<()> {
        write_packet(stream, &Packet::Ping).await?;
        loop {
            let frame = read_frame(stream).await?.ok_or("EOF waiting for ping")?;
            match frame.id {
                154 => return Ok(()),
                10 | 17 | 20 | 21 => return Err(format!("redundant update {}", frame.id).into()),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn tree_cut_syncs_wood_and_removes_only_the_detached_tiles() {
        let world =
            World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
        let world = world
            .with_edits(
                &BTreeMap::from([
                    ([2100, 234], block(60, None)), // Jungle grass under the uncut stump.
                    ([2100, 233], block(5, Some((0, 22)))),
                    (CUT, block(5, Some((0, 22)))),
                    ([2100, 231], block(5, Some((44, 0)))),
                    ([2099, 231], block(5, Some((66, 0)))),
                    ([2100, 230], block(5, Some((22, 198)))),
                    ([2101, 232], block(5, Some((0, 0)))), // Adjacent trunk belongs to another tree.
                    (GRASS, block(73, Some((0, 0)))),
                    ([2101, 234], block(2, None)),
                    (SOD, block(2, None)),
                ]),
                &BTreeMap::new(),
            )
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("trees.wld");
        world.save(&file).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = oneshot::channel();
        let task = tokio::spawn(terraria_network::serve_with_save(
            listener,
            3.try_into().unwrap(),
            String::new(),
            Some(world),
            Some(file.clone()),
            async {
                let _ = end.await;
            },
        ));
        timeout(Duration::from_secs(10), async {
            let (mut a, first, _, _) = join(addr, None).await?;
            let (mut b, _, _, _) = join(addr, None).await?;
            let request = Packet::TileChange {
                action: 0,
                position: CUT,
                value: 0,
                style: 0,
            };
            write_packet(
                &mut a,
                &Packet::TileChange {
                    action: 0,
                    position: CUT,
                    value: 1,
                    style: 0,
                },
            )
            .await?;
            write_packet(&mut a, &request).await?;
            let a_drop = wait_for_wood(&mut a).await?;
            let b_drop = wait_for_wood(&mut b).await?;
            assert_eq!(a_drop, b_drop);
            let Packet::DroppedItem(wood) = a_drop else {
                return Err("missing jungle wood".into());
            };
            assert_eq!((wood.item, wood.stack), (620, 4));
            match next(&mut a, 22).await? {
                Packet::ItemOwner(owner) => {
                    assert_eq!((owner.slot, owner.player), (wood.slot, first))
                }
                _ => unreachable!(),
            }
            write_packet(&mut a, &Packet::RemoveItem(wood.slot)).await?;
            observe(&mut b, &Packet::RemoveItem(wood.slot)).await?;

            // Retrying a cut against air must neither drop items nor reframe grass.
            write_packet(&mut b, &request).await?;
            assert_no_terrain_update(&mut b).await?;

            let trim_grass = Packet::TileChange {
                action: 0,
                position: GRASS,
                value: 0,
                style: 0,
            };
            write_packet(&mut a, &trim_grass).await?;
            observe_edit(&mut b, &trim_grass).await?;
            observe_edit(&mut a, &trim_grass).await?;
            assert_no_terrain_update(&mut a).await?;

            // An initial hit strips sod once. Repeating the hit against dirt stays quiet.
            let strip_sod = Packet::TileChange {
                action: 0,
                position: SOD,
                value: 1,
                style: 0,
            };
            write_packet(&mut a, &strip_sod).await?;
            for socket in [&mut a, &mut b] {
                let patch = next(socket, 20).await?;
                assert_eq!(visible_block(&[patch], SOD), Some(Some(0)));
            }
            write_packet(&mut a, &strip_sod).await?;
            assert_no_terrain_update(&mut a).await?;
            assert_no_terrain_update(&mut b).await?;

            // A genuine correction uses a tile patch and leaves the surrounding section intact.
            write_packet(
                &mut a,
                &Packet::TileChange {
                    action: 1,
                    position: [2101, 232],
                    value: 1,
                    style: 0,
                },
            )
            .await?;
            let patch = next(&mut a, 20).await?;
            assert_eq!(visible_block(&[patch], [2101, 232]), Some(Some(5)));

            let mut log = Vec::new();
            let (_rejoined, _, _, _) = join_capturing(addr, None, &mut log).await?;
            for position in HARVESTED {
                assert_eq!(visible_block(&log, position), Some(None));
            }
            assert_eq!(visible_block(&log, GRASS), Some(None));
            assert_eq!(visible_block(&log, SOD), Some(Some(0)));
            assert_eq!(visible_block(&log, [2101, 234]), Some(Some(2)));
            for position in [[2100, 233], [2101, 232]] {
                assert_eq!(visible_block(&log, position), Some(Some(5)));
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await
        .unwrap()
        .unwrap();
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        let saved = World::load(file).unwrap();
        for [x, y] in HARVESTED {
            assert_eq!(saved.tile(x as u32, y as u32).unwrap().block, None);
        }
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;
    use std::{collections::BTreeMap, time::Duration};
    use terraria_world::{Tile, World};
    use tokio::{net::TcpListener, sync::oneshot, time::timeout};

    #[tokio::test]
    async fn section_snapshots_precede_live_edits_and_contested_wall_drops_once() {
        let cell = [4003, 1051];
        let section = [20, 7];
        let origin = [4000, 1050];
        let source =
            World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap();
        let world = source
            .with_edits(&BTreeMap::from([(cell, Tile::default())]), &BTreeMap::new())
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shared.wld");
        world.save(&file).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = oneshot::channel();
        let task = tokio::spawn(terraria_network::serve_with_save(
            listener,
            2.try_into().unwrap(),
            String::new(),
            Some(world),
            Some(file.clone()),
            async {
                let _ = end.await;
            },
        ));
        timeout(Duration::from_secs(10), async {
            let (mut a, first, _, _) = join(addr, None).await?;
            let (mut b, second, _, _) = join(addr, None).await?;
            let request = Packet::RequestSection(section);
            let edit = |action, value| Packet::TileChange {
                action,
                position: cell,
                value,
                style: 0,
            };

            // The same socket queues the section and a newer edit back to back.
            write_packet(&mut a, &request).await?;
            write_packet(&mut a, &edit(1, 1)).await?;
            let snapshot = next(&mut a, 10).await?;
            assert!(matches!(&snapshot,Packet::TileSection(s) if s.origin==origin));
            assert_eq!(visible_tile(&[snapshot], cell).unwrap().block, None);
            observe_edit(&mut a, &edit(1, 1)).await?;

            // The unsubscribed socket gets the placed block in its snapshot. The later
            // wall edit arrives on both sockets and leaves the foreground block intact.
            write_packet(&mut b, &request).await?;
            write_packet(&mut b, &edit(3, 4)).await?;
            let snapshot = next(&mut b, 10).await?;
            let tile = visible_tile(&[snapshot], cell).unwrap();
            assert_eq!((tile.block, tile.wall), (Some(1), 0));
            for socket in [&mut a, &mut b] {
                let patch = next(socket, 20).await?;
                let tile = visible_tile(&[patch], cell).unwrap();
                assert_eq!((tile.block, tile.wall), (Some(1), 4));
            }

            let break_wall = edit(2, 0);
            tokio::try_join!(
                write_packet(&mut a, &break_wall),
                write_packet(&mut b, &break_wall)
            )?;
            observe_edit(&mut a, &break_wall).await?;
            observe_edit(&mut b, &break_wall).await?;
            let drop = next(&mut a, 21).await?;
            assert!(matches!(&drop,Packet::DroppedItem(item) if (item.item,item.stack)==(93,1)));
            observe(&mut b, &drop).await?;
            let Packet::ItemOwner(owner) = next(&mut a, 22).await? else {
                unreachable!()
            };
            let (winner, other) = if owner.player == first {
                (&mut a, &mut b)
            } else {
                assert_eq!(owner.player, second);
                (&mut b, &mut a)
            };
            let removal = Packet::RemoveItem(owner.slot);
            write_packet(other, &removal).await?;
            observe(other, &drop).await?; // Recreate the stack the non-owner predicted removing.
            write_packet(winner, &removal).await?;
            observe(other, &removal).await?;

            for socket in [winner, other] {
                write_packet(socket, &Packet::Ping).await?;
                loop {
                    let frame = read_frame(socket)
                        .await?
                        .ok_or("EOF after contested wall break")?;
                    assert!(
                        !matches!(frame.id, 17 | 20 | 21),
                        "duplicate terrain or drop"
                    );
                    if frame.id == 154 {
                        break;
                    }
                }
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await
        .unwrap()
        .unwrap();
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        let saved = World::load(file)
            .unwrap()
            .tile(cell[0] as u32, cell[1] as u32)
            .unwrap();
        assert_eq!((saved.block, saved.wall), (Some(1), 0));
    }
}

#[cfg(test)]
mod furniture_tests {
    use super::*;
    use std::{collections::BTreeMap, time::Duration};
    use terraria_world::{Tile, World};
    use tokio::{net::TcpListener, sync::oneshot, time::timeout};

    fn place(pos: [i16; 2], tile: i16, style: i16) -> Packet {
        Packet::ObjectPlacement {
            position: pos,
            tile,
            style,
            alternate: 0,
            random: -1,
            facing_right: true,
        }
    }

    fn break_at(position: [i16; 2]) -> Packet {
        Packet::TileChange {
            action: 0,
            position,
            value: 0,
            style: 0,
        }
    }

    async fn expect_tile(
        socket: &mut TcpStream,
        pos: [i16; 2],
        block: Option<u16>,
        frame: Option<(i16, i16)>,
    ) -> Result<()> {
        let patch = next(socket, 20).await?;
        let tile = visible_tile(&[patch], pos).ok_or("tile missing from station patch")?;
        assert_eq!((tile.block, tile.frame), (block, frame));
        Ok(())
    }

    async fn expect_bench(socket: &mut TcpStream, present: bool) -> Result<()> {
        let patch = next(socket, 20).await?;
        for (x, frame_x) in [(2199, 0), (2200, 18)] {
            let tile = visible_tile(std::slice::from_ref(&patch), [x, 233])
                .ok_or("bench was split across patches")?;
            assert_eq!(
                (tile.block, tile.frame),
                (present.then_some(18), present.then_some((frame_x, 0)))
            );
        }
        Ok(())
    }

    async fn expect_drop(socket: &mut TcpStream, item: i16) -> Result<()> {
        let drop = next(socket, 21).await?;
        assert!(matches!(drop,Packet::DroppedItem(d) if (d.item,d.stack)==(item,1)));
        Ok(())
    }

    #[tokio::test]
    async fn stations_place_and_break_atomically_and_survive_a_save() {
        let mut fixture = BTreeMap::new();
        for x in (2100..2118).chain(2199..2201) {
            for y in 228..235 {
                fixture.insert(
                    [x, y],
                    Tile {
                        block: (y == 234).then_some(1),
                        ..Tile::default()
                    },
                );
            }
        }
        fixture.insert(
            [2199, 233],
            Tile {
                block: Some(73),
                frame: Some((0, 0)),
                ..Tile::default()
            },
        );
        let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())
            .unwrap()
            .with_edits(&fixture, &BTreeMap::new())
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crafting.wld");
        world.save(&path).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve_with_save(
            listener,
            3.try_into().unwrap(),
            String::new(),
            Some(world),
            Some(path.clone()),
            async {
                let _ = end.await;
            },
        ));
        timeout(Duration::from_secs(10), async {
            let (mut a, _, _, _) = join(addr, None).await?;
            let (mut b, _, _, _) = join(addr, None).await?;

            // A floating bench prediction is corrected across the entire empty footprint.
            write_packet(&mut a, &place([2102, 229], 18, 0)).await?;
            let rejection = next(&mut a, 20).await?;
            for pos in [[2102, 229], [2103, 229]] {
                assert_eq!(
                    visible_tile(std::slice::from_ref(&rejection), pos)
                        .unwrap()
                        .block,
                    None
                );
            }

            // A work bench replaces grass and crosses the x=2200 section boundary.
            write_packet(&mut a, &place([2199, 233], 18, 0)).await?;
            for socket in [&mut a, &mut b] {
                expect_bench(socket, true).await?;
            }
            write_packet(&mut b, &place([2199, 233], 18, 2)).await?;
            expect_bench(&mut b, true).await?;
            let break_bench = break_at([2200, 233]);
            tokio::try_join!(
                write_packet(&mut a, &break_bench),
                write_packet(&mut b, &break_bench)
            )?;
            for socket in [&mut a, &mut b] {
                expect_bench(socket, false).await?;
                expect_drop(socket, 36).await?;
                write_packet(socket, &Packet::Ping).await?;
                loop {
                    let f = read_frame(socket)
                        .await?
                        .ok_or("EOF after station harvest")?;
                    assert_ne!(f.id, 21, "a second station drop was emitted");
                    if f.id == 154 {
                        break;
                    }
                }
            }

            // The fixed furnace has a center-bottom anchor and two rows of frames.
            write_packet(&mut a, &place([2106, 233], 17, 0)).await?;
            for socket in [&mut a, &mut b] {
                let patch = next(socket, 20).await?;
                for x in 0..3 {
                    for y in 0..2 {
                        let tile = visible_tile(std::slice::from_ref(&patch), [2105 + x, 232 + y])
                            .unwrap();
                        assert_eq!((tile.block, tile.frame), (Some(17), Some((x * 18, y * 18))));
                    }
                }
            }
            write_packet(&mut b, &break_at([2106, 234])).await?;
            for socket in [&mut a, &mut b] {
                let patch = next(socket, 20).await?;
                assert_eq!(
                    visible_tile(std::slice::from_ref(&patch), [2105, 232])
                        .unwrap()
                        .block,
                    None
                );
                assert_eq!(visible_tile(&[patch], [2106, 234]).unwrap().block, None);
                expect_drop(socket, 3).await?;
                expect_drop(socket, 33).await?;
            }

            // Legacy placement selects the lead anvil style in packet 17.
            write_packet(
                &mut a,
                &Packet::TileChange {
                    action: 1,
                    position: [2110, 233],
                    value: 16,
                    style: 1,
                },
            )
            .await?;
            for socket in [&mut a, &mut b] {
                expect_tile(socket, [2111, 233], Some(16), Some((54, 0))).await?;
            }
            let mut log = Vec::new();
            let (_c, _, _, _) = super::super::peers::join_capturing(addr, None, &mut log).await?;
            assert_eq!(
                visible_tile(&log, [2110, 233]).unwrap().frame,
                Some((36, 0))
            );
            assert_eq!(visible_tile(&log, [2200, 233]).unwrap().block, None);
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await
        .unwrap()
        .unwrap();
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
        let world = World::load(path).unwrap();
        assert_eq!(world.tile(2111, 233).unwrap().frame, Some((54, 0)));
        assert_eq!(world.tile(2106, 233).unwrap().block, None);
        assert_eq!(world.tile(2199, 233).unwrap().block, None);
    }
    #[tokio::test]
    async fn doors_share_both_columns_and_save_the_open_direction() {
        let mut cells = BTreeMap::new();
        for x in 2196..=2204 {
            for y in 229..=233 {
                cells.insert(
                    [x, y],
                    Tile {
                        block: ((y == 229 || y == 233) && (x == 2199 || x == 2203)).then_some(1),
                        wall: 4,
                        ..Tile::default()
                    },
                );
            }
        }
        let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())
            .unwrap()
            .with_edits(&cells, &BTreeMap::new())
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doors.wld");
        world.save(&path).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve_with_save(
            listener,
            3.try_into().unwrap(),
            String::new(),
            Some(world),
            Some(path.clone()),
            async {
                let _ = end.await;
            },
        ));
        timeout(Duration::from_secs(10), async {
            let (mut a, _, _, _) = join(addr, None).await?;
            let (mut b, second, _, _) = join(addr, None).await?;
            let toggle = |action, x, right| Packet::DoorToggle {
                action,
                position: [x, 231],
                facing_right: right,
            };
            let move_b = |x| {
                Packet::Movement(Movement {
                    player: second,
                    flags: [0; 4],
                    selected: 0,
                    position: [x, 230.0 * 16.0],
                    velocity: None,
                    mount: None,
                    return_points: None,
                    camera: None,
                })
            };
            write_packet(&mut a, &place([2199, 230], 10, 2)).await?;
            for socket in [&mut a, &mut b] {
                expect_tile(socket, [2199, 232], Some(10), Some((0, 144))).await?;
            }
            write_packet(&mut a, &toggle(0, 2199, true)).await?;
            for socket in [&mut a, &mut b] {
                let patch = next(socket, 20).await?;
                for row in 0..3 {
                    for dx in 0..2 {
                        let tile =
                            visible_tile(std::slice::from_ref(&patch), [2199 + dx, 230 + row])
                                .unwrap();
                        assert_eq!(
                            (tile.block, tile.frame, tile.wall),
                            (Some(11), Some((dx * 18, 108 + row * 18)), 4)
                        );
                    }
                }
            }
            let at_hinge = move_b(2199.0 * 16.0);
            write_packet(&mut b, &at_hinge).await?;
            observe(&mut a, &at_hinge).await?;
            write_packet(&mut a, &toggle(1, 2200, false)).await?;
            expect_tile(&mut a, [2200, 230], Some(11), Some((18, 108))).await?;
            let clear = move_b(2195.0 * 16.0);
            write_packet(&mut b, &clear).await?;
            observe(&mut a, &clear).await?;
            write_packet(&mut a, &toggle(1, 2200, false)).await?;
            for socket in [&mut a, &mut b] {
                expect_tile(socket, [2200, 230], None, None).await?;
            }
            write_packet(&mut b, &toggle(0, 2199, false)).await?;
            for socket in [&mut a, &mut b] {
                expect_tile(socket, [2198, 232], Some(11), Some((36, 144))).await?;
            }
            write_packet(&mut a, &place([2203, 230], 10, 0)).await?;
            for socket in [&mut a, &mut b] {
                expect_tile(socket, [2203, 230], Some(10), Some((0, 0))).await?;
            }
            let floor = break_at([2203, 233]);
            tokio::try_join!(write_packet(&mut a, &floor), write_packet(&mut b, &floor))?;
            for socket in [&mut a, &mut b] {
                expect_tile(socket, [2203, 230], None, None).await?;
                expect_drop(socket, 3).await?;
                expect_drop(socket, 25).await?;
                write_packet(socket, &Packet::Ping).await?;
                loop {
                    let frame = read_frame(socket).await?.ok_or("EOF after breaking door")?;
                    assert_ne!(frame.id, 21, "duplicate door or floor drop");
                    if frame.id == 154 {
                        break;
                    }
                }
            }
            let mut log = Vec::new();
            let (_c, _, _, _) = super::super::peers::join_capturing(addr, None, &mut log).await?;
            assert_eq!(
                visible_tile(&log, [2198, 231]).unwrap().frame,
                Some((36, 126))
            );
            assert_eq!(
                visible_tile(&log, [2199, 231]).unwrap().frame,
                Some((54, 126))
            );
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await
        .unwrap()
        .unwrap();
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
        let saved = World::load(path).unwrap();
        assert_eq!(saved.tile(2198, 231).unwrap().frame, Some((36, 126)));
        assert_eq!(saved.tile(2199, 231).unwrap().frame, Some((54, 126)));
        assert_eq!(saved.tile(2203, 231).unwrap().block, None);
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;
    use std::{num::NonZeroU32, time::Duration};
    use terraria_network::{Administration, ConsoleCommand};
    use terraria_world::World;
    use tokio::{
        net::TcpListener,
        sync::mpsc,
        time::{sleep, timeout},
    };

    async fn check_online_save(automatic: bool) -> Result<()> {
        let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())?;
        let revision = world.metadata().revision;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("online.wld");
        world.save(&path)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (send, commands) = mpsc::channel(4);
        let server = tokio::spawn(terraria_network::serve_managed(
            listener,
            2.try_into()?,
            String::new(),
            Some(world),
            Some(path.clone()),
            Administration {
                autosave_seconds: if automatic { NonZeroU32::new(1) } else { None },
                commands,
            },
            std::future::pending(),
        ));
        let (mut socket, _, _, _) = join(addr, None).await?;
        let place = |x, value| Packet::TileChange {
            action: 1,
            position: [x, 231],
            value,
            style: 0,
        };
        write_packet(&mut socket, &place(2099, 1)).await?;
        observe_edit(&mut socket, &place(2099, 1)).await?;
        if !automatic {
            send.send(ConsoleCommand::Save).await?;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let image = World::load(&path)?;
                if image.metadata().revision == revision + 1 {
                    assert_eq!(image.tile(2099, 231)?.block, Some(1));
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        })
        .await??;
        let (mut b, _, _, _) = join(addr, None).await?;
        write_packet(&mut b, &place(2100, 0)).await?;
        observe_edit(&mut socket, &place(2100, 0)).await?;
        send.send(ConsoleCommand::Exit).await?;
        server.await??;
        let final_world = World::load(path)?;
        assert_eq!(final_world.metadata().revision, revision + 2);
        assert_eq!(final_world.tile(2099, 231)?.block, Some(1));
        assert_eq!(final_world.tile(2100, 231)?.block, Some(0));
        let backup = World::load(dir.path().join("online.wld.terra-rs.bak"))?;
        assert_eq!(backup.metadata().revision, revision + 1);
        assert_eq!(backup.tile(2100, 231)?.block, None);
        Ok(())
    }

    #[tokio::test]
    async fn manual_save_keeps_players_online_and_exit_flushes_later_edits() {
        timeout(Duration::from_secs(10), check_online_save(false))
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn periodic_save_keeps_players_online_and_exit_flushes_later_edits() {
        timeout(Duration::from_secs(10), check_online_save(true))
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;
    use std::time::Duration;
    use terraria_network::{Administration, ConsoleCommand};
    use terraria_world::World;
    use tokio::{
        net::TcpListener,
        sync::mpsc,
        time::{sleep, timeout},
    };

    fn chest(action: u8, position: [i16; 2], id: i16) -> Packet {
        Packet::ChestUpdate {
            action,
            position,
            style: 0,
            chest: id,
        }
    }
    fn stack(id: i16, n: i16) -> Packet {
        Packet::ChestItem {
            chest: id,
            slot: 5,
            stack: n,
            prefix: 0,
            item: 3,
        }
    }

    #[tokio::test]
    async fn chest_slots_remain_stable_across_removal_compaction_and_reuse() {
        timeout(Duration::from_secs(10), run())
            .await
            .unwrap()
            .unwrap();
    }
    async fn run() -> Result<()> {
        let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())?;
        let initial_revision = world.metadata().revision;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("containers.wld");
        world.save(&path)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (admin, commands) = mpsc::channel(4);
        let task = tokio::spawn(terraria_network::serve_managed(
            listener,
            3.try_into()?,
            String::new(),
            Some(world),
            Some(path.clone()),
            Administration {
                autosave_seconds: None,
                commands,
            },
            std::future::pending(),
        ));
        let (mut a, _, _, _) = join(addr, None).await?;
        let (mut b, _, _, _) = join(addr, None).await?;
        for x in [2102, 2103, 2199, 2200] {
            let floor = Packet::TileChange {
                action: 1,
                position: [x, 234],
                value: 1,
                style: 0,
            };
            write_packet(&mut a, &floor).await?;
            observe_edit(&mut b, &floor).await?;
        }
        for (x, id) in [(2102, 1), (2199, 2)] {
            write_packet(&mut a, &chest(0, [x, 233], 1234)).await?;
            for socket in [&mut a, &mut b] {
                observe(socket, &chest(0, [x, 233], id)).await?;
                let patch = next(socket, 20).await?;
                for dx in 0..2 {
                    for dy in 0..2 {
                        let cell =
                            visible_tile(std::slice::from_ref(&patch), [x + dx, 232 + dy]).unwrap();
                        assert_eq!(
                            (cell.block, cell.frame),
                            (Some(21), Some((dx * 18, dy * 18)))
                        );
                    }
                }
            }
        }
        write_packet(&mut a, &Packet::RequestChest([2199, 232])).await?;
        assert!(matches!(
            next(&mut a, 155).await?,
            Packet::ChestSize {
                index: 2,
                capacity: 40
            }
        ));
        next(&mut a, 33).await?;
        write_packet(&mut a, &stack(2, 17)).await?;
        observe(&mut b, &stack(2, 17)).await?;
        write_packet(
            &mut a,
            &Packet::ChestState {
                chest: -1,
                position: [0; 2],
                name: Some("Ore".into()),
            },
        )
        .await?;
        observe(
            &mut b,
            &Packet::ChestName {
                chest: 2,
                position: [2199, 232],
                name: "Ore".into(),
            },
        )
        .await?;
        // Mining any part of a full chest or either of its supports leaves its contents intact.
        write_packet(&mut b, &chest(1, [2200, 233], 1)).await?;
        observe(&mut b, &stack(2, 17)).await?;
        write_packet(
            &mut b,
            &Packet::TileChange {
                action: 0,
                position: [2199, 234],
                value: 0,
                style: 0,
            },
        )
        .await?;
        let correction = next(&mut b, 20).await?;
        assert_eq!(
            visible_tile(&[correction], [2199, 234]).unwrap().block,
            Some(1)
        );
        let cut = chest(1, [2103, 233], 2);
        tokio::try_join!(write_packet(&mut a, &cut), write_packet(&mut b, &cut))?;
        for socket in [&mut a, &mut b] {
            observe(socket, &chest(1, [2102, 232], 1)).await?;
            let drop = next(socket, 21).await?;
            assert!(matches!(drop,Packet::DroppedItem(i) if (i.item,i.stack)==(48,1)));
            write_packet(socket, &Packet::Ping).await?;
            loop {
                let frame = read_frame(socket)
                    .await?
                    .ok_or("EOF after contested chest break")?;
                assert_ne!(frame.id, 21, "duplicate chest drop");
                if frame.id == 154 {
                    break;
                }
            }
        }
        admin.send(ConsoleCommand::Save).await?;
        loop {
            let image = World::load(&path)?;
            if image.metadata().revision > initial_revision {
                assert_eq!(image.chests().len(), 2);
                assert_eq!(image.chests()[1].name, "Ore");
                assert_eq!(image.chests()[1].items[0].stack, 17);
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        // The disk writer assigned the surviving new chest file index 1. Online it is still index 2.
        write_packet(&mut b, &Packet::RequestChest([2199, 232])).await?;
        observe(&mut b, &stack(2, 17)).await?;
        next(&mut b, 33).await?;
        write_packet(&mut b, &stack(2, 31)).await?;
        observe(&mut a, &stack(2, 31)).await?;
        write_packet(&mut a, &chest(0, [2102, 233], 0)).await?;
        observe(&mut b, &chest(0, [2102, 233], 1)).await?;
        let mut log = Vec::new();
        let (_c, _, _, _) = super::super::peers::join_capturing(addr, None, &mut log).await?;
        assert!(log.contains(&stack(2, 31)));
        assert!(log.iter().any(|p|matches!(p,Packet::TileSection(s) if s.chests.iter().any(|c|c.index==2 && c.text=="Ore"))));
        admin.send(ConsoleCommand::Exit).await?;
        task.await??;
        let saved = World::load(&path)?;
        assert_eq!(saved.chests().len(), 3);
        let ore = saved
            .chests()
            .iter()
            .find(|c| c.position == (2199, 232))
            .unwrap();
        assert_eq!((&*ore.name, ore.items[0].stack), ("Ore", 31));
        assert_eq!(saved.tile(2200, 233)?.frame, Some((18, 18)));
        assert!(
            saved
                .chests()
                .iter()
                .any(|c| c.position == (2102, 232) && c.items.is_empty())
        );
        Ok(())
    }
}
