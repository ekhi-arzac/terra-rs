//! Exercise the normal stock-client death, vitals and respawn sequence.
use super::{
    Result,
    peers::{join, join_capturing, observe},
};
use std::net::SocketAddr;
use terraria_network::write_packet;
use terraria_protocol::{DeathReason, NetworkText, Packet, PlayerDeath, Spawn, Vitals};

pub(super) async fn run(addr: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut player, slot, _, _) = join(addr, password).await?;
    let (mut observer, _, _, _) = join(addr, password).await?;
    let death = Packet::PlayerDeath(PlayerDeath {
        player: slot,
        reason: DeathReason {
            other: Some(9),
            ..Default::default()
        },
        damage: 50,
        direction: 0,
        pvp: false,
    });
    write_packet(&mut player, &death).await?;
    let announcement = Packet::ChatMessage {
        author: 255,
        text: NetworkText::Key(
            "DeathText.Poisoned".into(),
            vec![NetworkText::Literal(format!("Probe{slot}"))],
        ),
        color: [225, 25, 25],
    };
    observe(&mut player, &announcement).await?;
    observe(&mut observer, &announcement).await?;
    observe(&mut observer, &death).await?;
    let mut life = Vitals {
        player: slot,
        current: 0,
        maximum: 300,
    };
    write_packet(&mut player, &Packet::Life(life)).await?;
    observe(&mut observer, &Packet::Life(life)).await?;
    let mut before = Vec::new();
    let (late, late_slot, _, _) = join_capturing(addr, password, &mut before).await?;
    if !before.contains(&Packet::SetDeadAppearance(slot)) || !before.contains(&Packet::Life(life)) {
        return Err("late join missed dead player".into());
    }
    drop(late);
    observe(
        &mut observer,
        &Packet::PlayerActive {
            player: late_slot,
            active: false,
        },
    )
    .await?;
    let spawn = Packet::Spawn(Spawn {
        player: slot,
        point: [-1, -1],
        respawn_timer: 0,
        deaths: [1, 0],
        team: 0,
        context: 0,
    });
    write_packet(&mut player, &spawn).await?;
    observe(&mut observer, &spawn).await?;
    life.current = 150;
    let mut after = Vec::new();
    let (_late, _, _, _) = join_capturing(addr, password, &mut after).await?;
    if !after.contains(&Packet::Life(life)) || after.contains(&Packet::SetDeadAppearance(slot)) {
        return Err("late join missed respawn health".into());
    }
    println!(
        "Verified death announcement, observer death and respawn, and late-join vitals on each side of respawn."
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
    async fn poison_death_and_respawn_are_shared_with_late_joiners() -> Result<()> {
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
