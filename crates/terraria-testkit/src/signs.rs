//! Two-player sign editing on a small, disposable fixture.
use super::{
    Result,
    peers::{join, join_capturing, observe},
};
use std::{collections::BTreeMap, net::SocketAddr, path::Path};
use terraria_network::{read_frame, write_packet};
use terraria_protocol::Packet;
use terraria_world::{Sign, Tile, World, WorldClock};
use tokio::net::TcpStream;

const EXISTING: [i16; 2] = [2199, 232];
const BLANK: [i16; 2] = [2203, 232];
const INITIAL: &str = "Initial sign";
const EDITED: &str = "Trail to the cavern\n← 西";
const ADDED: &str = "Second sign 🪧";

fn world_with_signs() -> Result<World> {
    let world = World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())?;
    let mut tiles = BTreeMap::new();
    for [x, y] in [EXISTING, BLANK] {
        for dx in 0..2 {
            for dy in 0..2 {
                tiles.insert(
                    [x + dx, y + dy],
                    Tile {
                        block: Some(55),
                        frame: Some((dx * 18, dy * 18)),
                        ..Tile::default()
                    },
                );
            }
            tiles.insert(
                [x + dx, y + 2],
                Tile {
                    block: Some(1),
                    ..Tile::default()
                },
            );
        }
    }
    let position = (i32::from(EXISTING[0]), i32::from(EXISTING[1]));
    let signs = BTreeMap::from([(
        0,
        Sign {
            position,
            text: INITIAL.into(),
        },
    )]);
    let clock = WorldClock::from_world(world.metadata().time, &world.world_data()?);
    Ok(world.checkpoint_with_signs(&tiles, &Default::default(), &signs, clock)?)
}

pub(super) fn prepare(directory: &Path) -> Result<()> {
    let world = world_with_signs()?;
    std::fs::create_dir(directory)?;
    let path = directory.join("signs.wld");
    world.save(&path)?;
    println!("{}", path.display());
    Ok(())
}

fn sign_text(index: i16, position: [i16; 2], text: &str, player: u8, flags: u8) -> Packet {
    Packet::SignText {
        index,
        position,
        text: text.into(),
        player,
        flags,
    }
}

async fn open(socket: &mut TcpStream, slot: u8, position: [i16; 2], expected: &str) -> Result<i16> {
    let [x, y] = position;
    write_packet(socket, &Packet::RequestSign([x + 1, y + 1])).await?;
    loop {
        let frame = read_frame(socket).await?.ok_or("EOF waiting for sign")?;
        match frame.id {
            2 => return Err("server disconnected during sign probe".into()),
            47 => {
                let reply = Packet::decode(&frame)?;
                if let Packet::SignText { index, .. } = &reply {
                    if reply != sign_text(*index, position, expected, slot, 0) {
                        return Err(
                            "sign response differs from expected position, text or open flags"
                                .into(),
                        );
                    }
                    return Ok(*index);
                }
            }
            _ => {}
        }
    }
}

pub(super) async fn run(address: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut a, editor, _, _) = join(address, password).await?;
    let (mut b, reader, _, _) = join(address, password).await?;
    for (position, previous, next) in [(EXISTING, INITIAL, EDITED), (BLANK, "", ADDED)] {
        let index = open(&mut a, editor, position, previous).await?;
        write_packet(&mut a, &sign_text(index, position, next, 0, 0)).await?;
        observe(&mut b, &sign_text(index, position, next, editor, 0)).await?;
        if open(&mut b, reader, position, next).await? != index {
            return Err("the second player received a different sign slot".into());
        }
    }

    let mut log = Vec::new();
    let (_c, _, _, _) = join_capturing(address, password, &mut log).await?;
    for (position, text) in [(EXISTING, EDITED), (BLANK, ADDED)] {
        if !log.iter().any(|packet| {
            matches!(packet, Packet::TileSection(section) if section.signs.iter().any(|sign|
                sign.position == position && sign.text == text))
        }) {
            return Err(format!("late join missed the edited sign at {position:?}").into());
        }
    }
    println!(
        "Two players observed edits to a saved sign and a newly opened sign. A late join received both texts."
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
    async fn sign_edits_and_new_sign_records_survive_save_and_reload() -> Result<()> {
        let world = world_with_signs()?;
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("signs.wld");
        world.save(&file)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let (send, receive) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve_with_save(
            listener,
            4.try_into()?,
            String::new(),
            Some(world),
            Some(file.clone()),
            async {
                let _ = receive.await;
            },
        ));
        timeout(Duration::from_secs(10), async {
            // An attempted update cannot address a different sign index or overflow a reply.
            {
                let (mut socket, slot, _, _) = join(address, None).await?;
                let index = open(&mut socket, slot, EXISTING, INITIAL).await?;
                let correction = sign_text(index, EXISTING, INITIAL, slot, 1);
                for rejected in [
                    sign_text(index + 1, EXISTING, "ignored", slot, 0),
                    sign_text(index, EXISTING, &"x".repeat(4097), slot, 0),
                ] {
                    write_packet(&mut socket, &rejected).await?;
                    observe(&mut socket, &correction).await?;
                }
            }
            run(address, None).await
        })
        .await??;
        send.send(()).unwrap();
        server.await??;
        let saved = World::load(&file)?;
        assert_eq!(saved.signs().len(), 2);
        for (section_index, position, expected) in
            [([10, 1], EXISTING, EDITED), ([11, 1], BLANK, ADDED)]
        {
            let section = saved.section(section_index)?;
            let sign = section
                .signs
                .iter()
                .find(|sign| sign.position == position)
                .unwrap();
            assert_eq!(sign.text, expected);
        }
        let previous = World::load(directory.path().join("signs.wld.terra-rs.bak"))?;
        assert_eq!(previous.signs().len(), 1);
        assert_eq!(previous.signs()[0].text, INITIAL);
        Ok(())
    }
}
