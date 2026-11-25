//! Multiplayer team and PvP routing through the stock chat and roster messages.
use super::{
    Result,
    peers::{join, join_capturing, observe},
};
use std::net::SocketAddr;
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{NetworkText, Packet};
use tokio::net::TcpStream;

fn chat(author: u8, text: NetworkText, color: [u8; 3]) -> Packet {
    Packet::ChatMessage {
        author,
        text,
        color,
    }
}

fn command(name: &str, text: &str) -> Packet {
    Packet::ChatCommand {
        command: name.into(),
        text: text.into(),
    }
}

async fn observe_without(
    socket: &mut TcpStream,
    expected: &Packet,
    excluded: &[&Packet],
) -> Result<()> {
    loop {
        let frame = read_frame(socket).await?.ok_or("EOF during team probe")?;
        if frame.id == 2 {
            return Err("player disconnected during team probe".into());
        }
        if frame.id == 82 && frame.payload.starts_with(&1u16.to_le_bytes()) {
            let packet = Packet::decode(&frame)?;
            if excluded.contains(&&packet) {
                return Err(format!("unexpected chat delivery {packet:?}").into());
            }
            if &packet == expected {
                return Ok(());
            }
        }
    }
}

pub(super) async fn run(addr: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut a, first, _, _) = join(addr, password).await?;
    let (mut b, second, _, _) = join(addr, password).await?;
    write_packet(&mut a, &command("Party", "without a team")).await?;
    observe(
        &mut a,
        &chat(
            255,
            NetworkText::key("LegacyMultiplayer.10"),
            [255, 240, 20],
        ),
    )
    .await?;
    let red = [218, 59, 59];
    let notice = |slot, key, color| {
        chat(
            255,
            NetworkText::Key(
                format!("LegacyMultiplayer.{key}"),
                vec![NetworkText::Literal(format!("Probe{slot}"))],
            ),
            color,
        )
    };
    let first_team = Packet::PlayerTeam {
        player: first,
        team: 1,
    };
    write_packet(&mut a, &first_team).await?;
    observe(&mut b, &first_team).await?;
    observe(&mut a, &notice(first, 14, red)).await?;
    let pvp = Packet::PlayerPvp {
        player: first,
        hostile: true,
    };
    write_packet(&mut a, &pvp).await?;
    observe(&mut b, &pvp).await?;
    observe(&mut b, &notice(first, 11, red)).await?;
    let mut log = Vec::new();
    let (mut outsider, _, _, _) = join_capturing(addr, password, &mut log).await?;
    if !log.contains(&first_team) || !log.contains(&pvp) {
        return Err("late join missed team or PvP state".into());
    }
    let second_team = Packet::PlayerTeam {
        player: second,
        team: 1,
    };
    write_packet(&mut b, &second_team).await?;
    observe(&mut a, &second_team).await?;
    let joined = notice(second, 14, red);
    observe(&mut a, &joined).await?;
    observe(&mut b, &joined).await?;
    let private = chat(first, NetworkText::Literal("red only".into()), red);
    write_packet(&mut a, &command("Party", "red only")).await?;
    observe(&mut a, &private).await?;
    observe(&mut b, &private).await?;
    write_packet(
        &mut b,
        &Packet::PlayerTeam {
            player: second,
            team: 3,
        },
    )
    .await?;
    let left = notice(second, 16, [59, 149, 218]);
    observe(&mut a, &left).await?;
    observe(&mut b, &left).await?;
    let marker = chat(
        first,
        NetworkText::Literal("team check finished".into()),
        [255; 3],
    );
    write_packet(&mut a, &command("Say", "team check finished")).await?;
    observe_without(&mut outsider, &marker, &[&private, &joined, &left]).await?;
    println!(
        "Verified team updates, PvP, late-join state and private party chat across three players."
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
    async fn team_chat_and_announcements_reach_only_their_recipients() -> Result<()> {
        let world = terraria_world::World::parse(
            include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec(),
        )?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (stop, end) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve(
            listener,
            3.try_into()?,
            String::new(),
            Some(world),
            async {
                let _ = end.await;
            },
        ));
        timeout(Duration::from_secs(10), run(addr, None)).await??;
        stop.send(()).unwrap();
        server.await??;
        Ok(())
    }
}
