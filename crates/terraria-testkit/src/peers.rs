use super::{Result, entry, receive};
use std::net::SocketAddr;
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{GREETING, Movement, NetworkText, Packet};
use tokio::net::TcpStream;

pub(super) async fn join(
    addr: SocketAddr,
    password: Option<&str>,
) -> Result<(TcpStream, u8, [i16; 2], Option<[u16; 2]>)> {
    join_capturing(addr, password, &mut Vec::new()).await
}
pub(super) async fn join_capturing(
    addr: SocketAddr,
    password: Option<&str>,
    log: &mut Vec<Packet>,
) -> Result<(TcpStream, u8, [i16; 2], Option<[u16; 2]>)> {
    let mut stream = TcpStream::connect(addr).await?;
    write_packet(&mut stream, &Packet::Hello(GREETING.into())).await?;
    let mut response = receive(&mut stream, log).await?;
    if response == Packet::RequestPassword {
        write_packet(
            &mut stream,
            &Packet::Password(password.ok_or("set TERRARIA_PROBE_PASSWORD")?.into()),
        )
        .await?;
        response = receive(&mut stream, log).await?;
    }
    let Packet::AssignSlot { player, .. } = response else {
        return Err("admission failed".into());
    };
    entry::run(&mut stream, player, log, [-1, -1]).await?;
    let info = log
        .iter()
        .find_map(|p| match p {
            Packet::WorldData(d) => Some(d),
            _ => None,
        })
        .ok_or("missing world spawn")?;
    let [width, height] = info.size;
    if width <= 0 || height <= 0 {
        return Err("invalid world dimensions".into());
    }
    let seen: std::collections::BTreeSet<_> = log
        .iter()
        .filter_map(|p| match p {
            Packet::TileSection(s) => Some(s.origin),
            _ => None,
        })
        .collect();
    let unseen = (0..(width as u16).div_ceil(200))
        .rev()
        .flat_map(|x| {
            (0..(height as u16).div_ceil(150))
                .rev()
                .map(move |y| [x, y])
        })
        .find(|index| !seen.contains(&[i32::from(index[0]) * 200, i32::from(index[1]) * 150]));
    Ok((stream, player, info.spawn, unseen))
}
pub(super) async fn observe(stream: &mut TcpStream, expected: &Packet) -> Result<()> {
    for _ in 0..65536 {
        let frame = read_frame(stream)
            .await?
            .ok_or("unexpected EOF observing second player")?;
        if frame.id == 2 {
            return Err(format!("kick: {:?}", Packet::decode(&frame)?).into());
        }
        if frame.id == expected.id()
            && (frame.id != 82 || frame.payload.starts_with(&1u16.to_le_bytes()))
        {
            let packet = Packet::decode(&frame)?;
            if &packet == expected {
                return Ok(());
            }
        }
    }
    Err("observation limit exceeded".into())
}
pub(super) async fn run(addr: SocketAddr, password: Option<&str>) -> Result<()> {
    println!("Testing two players at {addr}");
    let (mut observer, first, _, _) = join(addr, password).await?;
    let (mut sender, second, spawn, corner) = join(addr, password).await?;
    if first == second {
        return Err("duplicate assigned slots".into());
    }
    observe(
        &mut observer,
        &Packet::PlayerActive {
            player: second,
            active: true,
        },
    )
    .await?;
    let zones = Packet::PlayerZones {
        player: second,
        zones: [8, 0, 0, 0, 0],
        town_npcs: 1,
    };
    write_packet(&mut sender, &zones).await?;
    observe(&mut observer, &zones).await?;
    write_packet(&mut sender, &Packet::Ping).await?;
    observe(&mut sender, &Packet::Ping).await?;
    let movement = Movement {
        player: second,
        flags: [72, 16, 0, 0],
        selected: 0,
        position: [
            f32::from(spawn[0]) * 16.0 + 3.0,
            f32::from(spawn[1]) * 16.0 - 42.0,
        ],
        velocity: None,
        mount: None,
        return_points: None,
        camera: None,
    };
    write_packet(&mut sender, &Packet::Movement(movement)).await?;
    observe(&mut observer, &Packet::Movement(movement)).await?;
    write_packet(
        &mut sender,
        &Packet::ChatCommand {
            command: "Say".into(),
            text: "headless peer check".into(),
        },
    )
    .await?;
    let chat = Packet::ChatMessage {
        author: second,
        text: NetworkText::Literal("headless peer check".into()),
        color: [255; 3],
    };
    observe(&mut observer, &chat).await?;
    observe(&mut sender, &chat).await?;
    if let Some(corner) = corner {
        write_packet(&mut sender, &Packet::RequestSection(corner)).await?;
        loop {
            let f = read_frame(&mut sender)
                .await?
                .ok_or("EOF awaiting requested terrain")?;
            if f.id == 2 {
                return Err("server rejected follow-up section request".into());
            }
            if f.id == 10
                && let Packet::TileSection(s) = Packet::decode(&f)?
                && s.origin == [i32::from(corner[0]) * 200, i32::from(corner[1]) * 150]
            {
                break;
            }
        }
        println!("  Received follow-up section {corner:?}.");
    }
    drop(sender);
    observe(
        &mut observer,
        &Packet::PlayerActive {
            player: second,
            active: false,
        },
    )
    .await?;
    let (_reconnected, slot, _, _) = join(addr, password).await?;
    if slot != second {
        return Err("released slot was not reused".into());
    }
    observe(
        &mut observer,
        &Packet::PlayerActive {
            player: slot,
            active: true,
        },
    )
    .await?;
    println!(
        "Observed activation, movement, chat on both sockets, departure and slot {slot} rejoining."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{num::NonZeroU8, time::Duration};
    use tokio::{net::TcpListener, sync::oneshot, time::timeout};
    #[tokio::test]
    async fn two_players_enter_a_loaded_world_move_chat_and_rejoin() {
        let world = terraria_world::World::parse(
            include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec(),
        )
        .unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve(
            listener,
            NonZeroU8::new(2).unwrap(),
            "secret".into(),
            Some(world),
            async {
                let _ = end.await;
            },
        ));
        timeout(Duration::from_secs(15), run(addr, Some("secret")))
            .await
            .unwrap()
            .unwrap();
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    }
}
