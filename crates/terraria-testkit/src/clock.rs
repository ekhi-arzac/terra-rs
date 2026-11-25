//! Two-player probe for a disposable world saved shortly before dawn or dusk.
use crate::{Result, peers::join_capturing};
use std::net::SocketAddr;
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{Packet, WorldData};
use tokio::net::TcpStream;

fn initial(log: &[Packet]) -> Result<&WorldData> {
    log.iter()
        .find_map(|p| match p {
            Packet::WorldData(data) => Some(data.as_ref()),
            _ => None,
        })
        .ok_or_else(|| "missing initial clock".into())
}

async fn next_clock(socket: &mut TcpStream) -> Result<Box<WorldData>> {
    loop {
        let frame = read_frame(socket).await?.ok_or("EOF awaiting clock")?;
        if frame.id == 2 {
            return Err(format!("join rejected {:?}", Packet::decode(&frame)?).into());
        }
        if frame.id == 7
            && let Packet::WorldData(data) = Packet::decode(&frame)?
        {
            return Ok(data);
        }
    }
}

pub(super) async fn run(addr: SocketAddr, password: Option<&str>) -> Result<i32> {
    let mut log = Vec::new();
    let (mut a, player, _, _) = join_capturing(addr, password, &mut log).await?;
    let start = initial(&log)?;
    let dusk = start.time_flags & 1 != 0;
    let phase = start.moon_phase;
    if (if dusk { start.moondial } else { start.sundial }) != 0 {
        return Err("fixture needs a ready dial for the approaching transition".into());
    }
    let mut observer_log = Vec::new();
    let (mut b, _, _, _) = join_capturing(addr, password, &mut observer_log).await?;
    let observer = initial(&observer_log)?;
    if observer.time < start.time || observer.time_flags != start.time_flags {
        return Err("observer entered in a different clock phase".into());
    }
    write_packet(&mut a, &Packet::UseTimeDial { player, dusk }).await?;
    let fast = |info: &WorldData| {
        if dusk {
            info.progression[8] & 128 != 0
        } else {
            info.progression[2] & 2 != 0
        }
    };
    let cooldown = |info: &WorldData| if dusk { info.moondial } else { info.sundial };
    let next_phase = if dusk { phase } else { (phase + 1) % 8 };
    for socket in [&mut a, &mut b] {
        let activated = loop {
            let state = next_clock(socket).await?;
            if fast(&state) {
                break state;
            }
        };
        if cooldown(&activated) != 8 {
            return Err("incorrect initial dial cooldown".into());
        }
        let transition = loop {
            let state = next_clock(socket).await?;
            if (state.time_flags & 1 != 0) != dusk {
                break state;
            }
        };
        if fast(&transition)
            || cooldown(&transition) != 7
            || transition.time != 0
            || transition.moon_phase != next_phase
        {
            return Err(format!(
                "unexpected transition at tick {}, moon {}, cooldown {}",
                transition.time,
                transition.moon_phase,
                cooldown(&transition)
            )
            .into());
        }
    }
    // With the first player still active, a replacement for the observer sees the live clock.
    drop(b);
    let mut reconnect_log = Vec::new();
    let (_rejoined, _, _, _) = join_capturing(addr, password, &mut reconnect_log).await?;
    let rejoined = initial(&reconnect_log)?;
    if (rejoined.time_flags & 1 != 0) == dusk
        || rejoined.moon_phase != next_phase
        || cooldown(rejoined) != 7
        || fast(rejoined)
    {
        return Err("rejoining player missed the clock transition".into());
    }
    println!(
        "Both players observed the dial and phase transition at tick zero. Rejoin saw moon phase {next_phase} and cooldown 7."
    );
    Ok(start.time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terraria_world::{World, WorldClock};
    use tokio::{
        net::TcpListener,
        sync::oneshot,
        time::{Duration, sleep, timeout},
    };

    async fn exercise(daytime: bool) -> Result<()> {
        let fixture =
            World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec())?;
        let clock = WorldClock {
            time: if daytime { 53_400.5 } else { 31_800.5 },
            daytime,
            moon_phase: 7,
            ..Default::default()
        };
        let world = fixture.checkpoint(&Default::default(), &Default::default(), clock)?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("clock.wld");
        world.save(&path)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (stop, end) = oneshot::channel();
        let server = tokio::spawn(terraria_network::serve_with_save(
            listener,
            3.try_into()?,
            String::new(),
            Some(world),
            Some(path.clone()),
            async {
                let _ = end.await;
            },
        ));
        // A pre-spawn world request observes the exact saved time even after several idle ticks.
        sleep(Duration::from_millis(100)).await;
        assert_eq!(run(addr, None).await?, clock.time as i32);
        stop.send(()).unwrap();
        server.await??;
        let saved = World::load(path)?;
        let end = WorldClock::from_world(saved.metadata().time, &saved.world_data()?);
        assert_eq!(end.daytime, !daytime);
        assert_eq!(
            (end.moon_phase, end.sundial, end.moondial),
            if daytime { (7, 0, 7) } else { (0, 7, 0) }
        );
        assert!(!end.fast_dawn && !end.fast_dusk);
        Ok(())
    }

    #[tokio::test]
    async fn dawn_and_dusk_reach_both_players_rejoin_and_disk() {
        for daytime in [false, true] {
            timeout(Duration::from_secs(15), exercise(daytime))
                .await
                .unwrap()
                .unwrap();
        }
    }
}
