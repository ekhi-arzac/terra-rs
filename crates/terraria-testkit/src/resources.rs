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
            name: None,
        },
    )
    .await?;
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
    observe(&mut b, &place).await?;
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
            17 => mined |= Packet::decode(&f)? == mine,
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
    let mut saw_air = false;
    for p in &log {
        if let Packet::TileSection(s) = p {
            let dx = i32::from(cell[0]) - s.origin[0];
            let dy = i32::from(cell[1]) - s.origin[1];
            if (0..i32::from(s.size[0])).contains(&dx) && (0..i32::from(s.size[1])).contains(&dy) {
                let mut index = dy as usize * usize::from(s.size[0]) + dx as usize;
                for run in &s.runs {
                    if index < usize::from(run.count) {
                        saw_air = run.tile.block.is_none();
                        break;
                    }
                    index -= usize::from(run.count);
                }
            }
        }
    }
    if !saw_air || !log.contains(&edit) || !log.iter().any(|p| matches!(p,Packet::DroppedItem(i) if Some(i.slot) == resource && i.item == 3 && i.stack == 1)) {
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
        let saved = &reloaded.chests()[0].items[0];
        assert_eq!((saved.item_type, saved.stack), (3, 17));
        assert_eq!(reloaded.tile(2099, 231).unwrap().block, None);
    }
}
