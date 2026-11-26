//! Compare liquid edits inside sealed cells where the reference volumes stay stable.
use super::{
    Result,
    entry::visible_tile,
    peers::{join, join_capturing, observe},
};
use std::{collections::BTreeMap, net::SocketAddr, path::Path};
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{LiquidUpdate, Packet};
use terraria_world::{Liquid, Tile, World};
use tokio::net::TcpStream;

const CELLS: [LiquidUpdate; 4] = [
    LiquidUpdate {
        position: [2198, 232],
        kind: Liquid::Water,
        amount: 200,
    },
    LiquidUpdate {
        position: [2205, 232],
        kind: Liquid::Lava,
        amount: 200,
    },
    LiquidUpdate {
        position: [2210, 232],
        kind: Liquid::Honey,
        amount: 200,
    },
    LiquidUpdate {
        position: [2215, 232],
        kind: Liquid::Shimmer,
        amount: 200,
    },
];
const DRAINED: LiquidUpdate = LiquidUpdate {
    amount: 0,
    ..CELLS[0]
};

fn fixture() -> Result<World> {
    let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())?;
    let mut tiles = BTreeMap::new();
    for LiquidUpdate {
        position: [x, y], ..
    } in CELLS
    {
        for dx in -1..=1 {
            for dy in -1..=1 {
                tiles.insert(
                    [x + dx, y + dy],
                    Tile {
                        block: (dx != 0 || dy != 0).then_some(1),
                        ..Tile::default()
                    },
                );
            }
        }
    }
    Ok(world.with_edits(&tiles, &BTreeMap::new())?)
}

pub(super) fn prepare(directory: &Path) -> Result<()> {
    super::save_fixture(fixture()?, directory, "liquids.wld")
}

async fn observe_fill(socket: &mut TcpStream, expected: LiquidUpdate) -> Result<()> {
    loop {
        let frame = read_frame(socket)
            .await?
            .ok_or("EOF waiting for liquid update")?;
        if frame.id == 2 {
            return Err("observer disconnected during liquid probe".into());
        }
        if frame.id == 82
            && frame.payload.starts_with(&[0, 0])
            && let Packet::LiquidBatch(updates) = Packet::decode(&frame)?
            && updates.contains(&expected)
        {
            return Ok(());
        }
    }
}

fn check_cells(capture: &[Packet]) -> Result<()> {
    for &LiquidUpdate {
        position: [x, y],
        kind,
        amount,
    } in &[DRAINED, CELLS[1], CELLS[2], CELLS[3]]
    {
        let actual = visible_tile(capture, [x, y]).map(|tile| tile.liquid);
        if actual != Some((amount > 0).then_some((kind, amount))) {
            return Err(
                format!("late join saw an unexpected liquid state at {x},{y}: {actual:?}").into(),
            );
        }
    }
    Ok(())
}

pub(super) async fn run(address: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut editor, _, _, _) = join(address, password).await?;
    let (mut observer, _, _, _) = join(address, password).await?;
    for update in CELLS {
        write_packet(&mut editor, &Packet::LiquidUpdate(update)).await?;
        observe_fill(&mut observer, update).await?;
    }
    let drain = Packet::LiquidUpdate(DRAINED);
    write_packet(&mut editor, &drain).await?;
    observe(&mut observer, &drain).await?;
    let mut capture = Vec::new();
    let (_late, _, _, _) = join_capturing(address, password, &mut capture).await?;
    check_cells(&capture)?;
    println!(
        "Verified four liquid types, a drained water cell and all four cell states on a late join."
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
    async fn liquid_edits_are_replicated_and_included_in_a_world_checkpoint() -> Result<()> {
        let world = fixture()?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("liquids.wld");
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
        assert_eq!(saved.tile(2198, 232)?.liquid, None);
        assert_eq!(saved.tile(2199, 232)?.block, Some(1));
        for LiquidUpdate {
            position: [x, y],
            kind,
            amount,
        } in &CELLS[1..]
        {
            assert_eq!(
                saved.tile(*x as u32, *y as u32)?.liquid,
                Some((*kind, *amount))
            );
        }
        Ok(())
    }
}
