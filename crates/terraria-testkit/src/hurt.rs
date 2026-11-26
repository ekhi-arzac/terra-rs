//! Observe hurt reports and their subsequent health uploads across three clients.
use super::{
    Result,
    peers::{join, join_capturing, observe},
};
use std::net::SocketAddr;
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{DeathReason, Packet, PlayerHurt, Vitals};
use tokio::net::TcpStream;

async fn check_filtered(
    sender: &mut TcpStream,
    observer: &mut TcpStream,
    hurt: &PlayerHurt,
    sender_slot: u8,
) -> Result<()> {
    let marker = Packet::Life(Vitals {
        player: sender_slot,
        current: 93,
        maximum: 100,
    });
    write_packet(sender, &Packet::PlayerHurt(hurt.clone())).await?;
    write_packet(sender, &marker).await?;
    loop {
        let frame = read_frame(observer)
            .await?
            .ok_or("EOF awaiting the sender's health update")?;
        match frame.id {
            2 => return Err("observer disconnected during hurt test".into()),
            117 => return Err("a hit reached a player outside PvP".into()),
            16 if Packet::decode(&frame)? == marker => return Ok(()),
            _ => {}
        }
    }
}

pub(super) async fn run(addr: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut a, first, _, _) = join(addr, password).await?;
    let (mut b, second, _, _) = join(addr, password).await?;
    let (mut observer, _, _, _) = join(addr, password).await?;

    let environmental = Packet::PlayerHurt(PlayerHurt {
        player: first,
        reason: DeathReason {
            other: Some(0),
            ..Default::default()
        },
        damage: 20,
        direction: 0,
        critical: false,
        pvp: false,
        dodgeable: false,
        immunity_slot: -1,
    });
    write_packet(&mut a, &environmental).await?;
    observe(&mut b, &environmental).await?;
    observe(&mut observer, &environmental).await?;

    let pvp = PlayerHurt {
        player: second,
        reason: DeathReason {
            player: Some(i16::from(first)),
            item: Some(3507),
            ..Default::default()
        },
        damage: 13,
        direction: 1,
        critical: true,
        pvp: true,
        dodgeable: true,
        immunity_slot: -1,
    };
    check_filtered(&mut a, &mut observer, &pvp, first).await?;
    for (sender, player) in [(&mut a, first), (&mut b, second)] {
        let toggle = Packet::PlayerPvp {
            player,
            hostile: true,
        };
        write_packet(sender, &toggle).await?;
        observe(&mut observer, &toggle).await?;
    }

    let hit = Packet::PlayerHurt(pvp);
    write_packet(&mut a, &hit).await?;
    observe(&mut b, &hit).await?;
    observe(&mut observer, &hit).await?;
    let life = Packet::Life(Vitals {
        player: second,
        current: 87,
        maximum: 100,
    });
    write_packet(&mut b, &life).await?;
    observe(&mut observer, &life).await?;
    drop(a);
    let mut next_join = Vec::new();
    let (_new, _, _, _) = join_capturing(addr, password, &mut next_join).await?;
    if !next_join.contains(&life) {
        return Err("late join missed the victim's updated health".into());
    }
    println!(
        "Verified environmental hurt, PvP opt-in, and the victim's health update on a later join."
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
    async fn hurt_packets_respect_pvp_and_the_victim_can_sync_health() -> Result<()> {
        let world = terraria_world::World::parse(
            include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec(),
        )?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (stop, end) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve(
            listener,
            4.try_into()?,
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
