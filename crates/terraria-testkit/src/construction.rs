//! Reference checks for hammer changes, local wire edits and their dropped items.
use super::{
    Result,
    entry::visible_tile,
    peers::{join, join_capturing},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::Path,
};
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{Packet, Vitals};
use terraria_world::{Chest, Tile, World};
use tokio::net::TcpStream;

const HALF: [i16; 2] = [2199, 230];
const SLOPE: [i16; 2] = [2200, 230];
const WIRING: [i16; 2] = [2201, 230];
const SUPPORT: [i16; 2] = [2210, 234];
const SWITCH: [i16; 2] = [2198, 226];
const ACTUATED: [i16; 2] = [2201, 226];
const TWO_COLORS: [i16; 2] = [2202, 226];
const DISCONNECTED: [i16; 2] = [2204, 226];

fn fixture() -> Result<World> {
    let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())?;
    let stone = Tile {
        block: Some(1),
        ..Tile::default()
    };
    let mut tiles = BTreeMap::from([
        (HALF, stone),
        (SLOPE, stone),
        (WIRING, Tile::default()),
        ([HALF[0], HALF[1] - 1], Tile::default()),
        ([SLOPE[0], SLOPE[1] - 1], Tile::default()),
    ]);
    for x in 2198..=2202 {
        tiles.insert([x, 225], Tile::default());
        tiles.insert(
            [x, 226],
            Tile {
                flags: [0, 2, 0, 0],
                ..Tile::default()
            },
        );
        tiles.insert(
            [x, 227],
            Tile {
                flags: [0, 4, 0, 0],
                ..Tile::default()
            },
        );
    }
    for x in 2199..=2200 {
        tiles.insert(
            [x, 225],
            Tile {
                flags: [0, 2, 0, 0],
                ..Tile::default()
            },
        );
    }
    tiles.insert(
        SWITCH,
        Tile {
            block: Some(136),
            frame: Some((0, 0)),
            wall: 4,
            flags: [0, 6, 0, 0],
            ..Tile::default()
        },
    );
    tiles.insert(
        ACTUATED,
        Tile {
            flags: [0, 2, 2, 0],
            ..stone
        },
    );
    tiles.insert(
        TWO_COLORS,
        Tile {
            flags: [0, 6, 2, 0],
            ..stone
        },
    );
    tiles.insert(
        DISCONNECTED,
        Tile {
            flags: [0, 4, 2, 0],
            ..stone
        },
    );
    let [x, y] = SUPPORT;
    for dx in 0..2 {
        for dy in 0..2 {
            tiles.insert(
                [x + dx, y - 2 + dy],
                Tile {
                    block: Some(21),
                    frame: Some((dx * 18, dy * 18)),
                    ..Tile::default()
                },
            );
        }
        tiles.insert([x + dx, y], stone);
    }
    let origin = (i32::from(x), i32::from(y) - 2);
    let containers = BTreeMap::from([(
        origin,
        Some(Chest {
            position: origin,
            name: String::new(),
            capacity: 40,
            items: Vec::new(),
        }),
    )]);
    Ok(world.with_containers(&tiles, &containers)?)
}

pub(super) fn prepare(directory: &Path) -> Result<()> {
    super::save_fixture(fixture()?, directory, "construction.wld")
}

fn edit(action: u8, position: [i16; 2], value: i16) -> Packet {
    Packet::TileChange {
        action,
        position,
        value,
        style: 0,
    }
}

async fn send_edit(
    source: &mut TcpStream,
    observer: &mut TcpStream,
    action: u8,
    position: [i16; 2],
    value: i16,
    expected: impl Fn(Tile) -> bool,
) -> Result<()> {
    let request = edit(action, position, value);
    write_packet(source, &request).await?;
    loop {
        let frame = read_frame(observer)
            .await?
            .ok_or("EOF observing terrain edit")?;
        if frame.id == 2 {
            return Err("disconnected while observing terrain edit".into());
        }
        if matches!(frame.id, 17 | 20) {
            let packet = Packet::decode(&frame)?;
            if packet == request {
                return Ok(());
            }
            if let Some(tile) = visible_tile(&[packet], position)
                && expected(tile)
            {
                return Ok(());
            }
        }
    }
}

fn is_wire_bit(tile: Tile, byte: usize, mask: u8, expected: bool) -> bool {
    (tile.flags[byte] & mask != 0) == expected
}

fn check_tile(position: [i16; 2], tile: Tile) -> Result<()> {
    let expected = match position {
        HALF => tile.block == Some(1) && tile.flags[1] & 0x70 == 16,
        SLOPE => tile.block == Some(1) && tile.flags[1] & 0x70 == 80,
        WIRING => tile.block.is_none() && tile.flags[1] & 14 == 12 && tile.flags[2] & 34 == 0,
        SUPPORT => tile.block == Some(1) && tile.flags[1] & 0x70 == 0,
        SWITCH => tile.block == Some(136) && tile.frame == Some((0, 18)),
        ACTUATED => tile.block == Some(1) && tile.flags[2] & 6 == 6,
        TWO_COLORS | DISCONNECTED => tile.block == Some(1) && tile.flags[2] & 6 == 2,
        _ => return Err("construction test checked an unknown coordinate".into()),
    };
    if !expected {
        return Err(format!("unexpected terrain at {position:?}: {tile:?}").into());
    }
    Ok(())
}

async fn press(source: &mut TcpStream, observer: &mut TcpStream, inactive: bool) -> Result<()> {
    let click = Packet::HitSwitch(SWITCH);
    write_packet(source, &click).await?;
    let mut clicked = false;
    let mut actuated = false;
    while !clicked || !actuated {
        let frame = read_frame(observer)
            .await?
            .ok_or("EOF waiting for an actuator pulse")?;
        match frame.id {
            2 => return Err("player disconnected during switch pulse".into()),
            59 => clicked |= Packet::decode(&frame)? == click,
            20 => {
                actuated |= visible_tile(&[Packet::decode(&frame)?], ACTUATED)
                    .is_some_and(|tile| is_wire_bit(tile, 2, 4, inactive));
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) async fn run(address: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut editor, _, _, _) = join(address, password).await?;
    let (mut observer, other, _, _) = join(address, password).await?;

    for (action, position, value, shape) in [
        (7, HALF, 1, 16),
        (7, HALF, 1, 0),
        (14, HALF, 3, 64),
        (23, HALF, 0, 16),
        (14, SLOPE, 1, 32),
        (14, SLOPE, 2, 48),
        (14, SLOPE, 3, 64),
        (14, SLOPE, 4, 80),
    ] {
        send_edit(
            &mut editor,
            &mut observer,
            action,
            position,
            value,
            |tile| tile.flags[1] & 0x70 == shape,
        )
        .await?;
    }
    // This request must leave the tile supporting the chest unchanged.
    write_packet(&mut editor, &edit(14, SUPPORT, 1)).await?;

    for (action, byte, mask) in [(5, 1, 2), (10, 1, 4), (12, 1, 8), (16, 2, 32), (8, 2, 2)] {
        send_edit(&mut editor, &mut observer, action, WIRING, 0, |tile| {
            is_wire_bit(tile, byte, mask, true)
        })
        .await?;
    }
    let mut seen = BTreeSet::new();
    for (action, byte, mask, item) in [(6, 1, 2, 530), (17, 2, 32, 530), (9, 2, 2, 849)] {
        // The reference can announce a recovered item before relaying the tile change.
        // Gather both responses in one pass.
        write_packet(&mut observer, &edit(action, WIRING, 0)).await?;
        let request = edit(action, WIRING, 0);
        let mut changed = false;
        let mut dropped = false;
        while !changed || !dropped {
            let frame = read_frame(&mut editor)
                .await?
                .ok_or("EOF waiting for wire removal")?;
            match frame.id {
                2 => return Err("disconnected during wire removal".into()),
                17 | 20 => {
                    let packet = Packet::decode(&frame)?;
                    changed |= packet == request
                        || visible_tile(&[packet], WIRING)
                            .is_some_and(|tile| is_wire_bit(tile, byte, mask, false));
                }
                21 => {
                    if let Packet::DroppedItem(drop) = Packet::decode(&frame)?
                        && drop.item == item
                        && drop.stack == 1
                        && seen.insert(drop.slot)
                    {
                        dropped = true;
                    }
                }
                _ => {}
            }
        }
    }
    write_packet(&mut observer, &edit(6, WIRING, 0)).await?;
    let marker = Packet::Life(Vitals {
        player: other,
        current: 99,
        maximum: 100,
    });
    write_packet(&mut observer, &marker).await?;
    loop {
        let frame = read_frame(&mut editor)
            .await?
            .ok_or("EOF checking repeated wire cut")?;
        match frame.id {
            2 => return Err("disconnected checking repeated wire cut".into()),
            16 if Packet::decode(&frame)? == marker => break,
            21 => {
                if let Packet::DroppedItem(drop) = Packet::decode(&frame)?
                    && drop.item == 530
                    && !seen.contains(&drop.slot)
                {
                    return Err("removing an absent wire dropped a second item".into());
                }
            }
            _ => {}
        }
    }
    press(&mut editor, &mut observer, true).await?;
    press(&mut observer, &mut editor, false).await?;
    press(&mut editor, &mut observer, true).await?;
    for inactive in [true, false] {
        send_edit(&mut observer, &mut editor, 19, DISCONNECTED, 0, |tile| {
            is_wire_bit(tile, 2, 4, inactive)
        })
        .await?;
    }
    let mut capture = Vec::new();
    let (_late, _, _, _) = join_capturing(address, password, &mut capture).await?;
    for position in [
        HALF,
        SLOPE,
        WIRING,
        SUPPORT,
        SWITCH,
        ACTUATED,
        TWO_COLORS,
        DISCONNECTED,
    ] {
        check_tile(
            position,
            visible_tile(&capture, position).ok_or("late join missed the edited tile")?,
        )?;
    }
    println!(
        "Verified hammer shapes, wire recovery, a cross-section actuator circuit, and late-entry terrain."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        net::TcpListener,
        sync::oneshot,
        time::{Duration, timeout},
    };

    #[tokio::test]
    async fn hammer_and_wiring_edits_are_shared_and_persist() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("construction.wld");
        let world = fixture()?;
        world.save(&path)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let (send, receive) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve_with_save(
            listener,
            4.try_into()?,
            String::new(),
            Some(world),
            Some(path.clone()),
            async {
                let _ = receive.await;
            },
        ));
        timeout(Duration::from_secs(10), run(address, None)).await??;
        send.send(()).unwrap();
        server.await??;
        let saved = World::load(path)?;
        for position @ [x, y] in [
            HALF,
            SLOPE,
            WIRING,
            SUPPORT,
            SWITCH,
            ACTUATED,
            TWO_COLORS,
            DISCONNECTED,
        ] {
            check_tile(position, saved.tile(x as u32, y as u32)?)?;
        }
        Ok(())
    }
}
