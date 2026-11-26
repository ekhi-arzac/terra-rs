//! Projectile relay and ownership check using two direct-IP sockets.
use super::{
    Result,
    peers::{join, observe},
};
use std::net::SocketAddr;
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{Packet, ProjectileKey, ProjectileRemove, ProjectileSync, Vitals};

pub(super) async fn run(addr: SocketAddr, password: Option<&str>) -> Result<()> {
    let (mut owner, player, spawn, _) = join(addr, password).await?;
    let (mut observer, other, _, _) = join(addr, password).await?;
    observe(
        &mut owner,
        &Packet::PlayerActive {
            player: other,
            active: true,
        },
    )
    .await?;
    for (index, kind) in [1, 14].into_iter().enumerate() {
        let position = [f32::from(spawn[0]) * 16., 2000.];
        let mut shot = ProjectileSync {
            key: ProjectileKey {
                spawner: player,
                index: index as u16,
                generation: 2,
            },
            position,
            velocity: [8., 0.],
            kind,
            ai: [Some(1.25), Some(2.25), Some(0.5)],
            banner: Some(1),
            damage: Some(7),
            knockback: Some(1.5),
            original_damage: Some(8),
        };
        write_packet(&mut owner, &Packet::ProjectileSync(shot)).await?;
        observe(&mut observer, &Packet::ProjectileSync(shot)).await?;
        shot.position[0] += 4.;
        write_packet(&mut owner, &Packet::ProjectileSync(shot)).await?;
        observe(&mut observer, &Packet::ProjectileSync(shot)).await?;
        let remove = Packet::ProjectileRemove(ProjectileRemove {
            key: shot.key,
            position: shot.position,
        });

        // The observer attempts removal. Its next valid update is an ordering marker.
        let marker = Packet::Life(Vitals {
            player: other,
            current: 90 - index as i16,
            maximum: 100,
        });
        write_packet(&mut observer, &remove).await?;
        write_packet(&mut observer, &marker).await?;
        loop {
            let frame = read_frame(&mut owner)
                .await?
                .ok_or("EOF during projectile ownership check")?;
            match frame.id {
                2 => return Err("owner disconnected during projectile probe".into()),
                29 if Packet::decode(&frame)? == remove => {
                    return Err("observer removed another player's projectile".into());
                }
                16 if Packet::decode(&frame)? == marker => break,
                _ => {}
            }
        }
        write_packet(&mut owner, &remove).await?;
        observe(&mut observer, &remove).await?;
    }
    println!("Verified projectile updates, owner removals and rejection of cross-player removal.");
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

    async fn generation_cleanup(address: SocketAddr) -> Result<()> {
        let (mut owner, slot, _, _) = join(address, None).await?;
        let (mut observer, _, _, _) = join(address, None).await?;
        let mut shot = ProjectileSync {
            key: ProjectileKey {
                spawner: slot,
                index: 5,
                generation: 16383,
            },
            position: [33518., 3670.],
            velocity: [8., 0.],
            kind: 1,
            ai: [None; 3],
            banner: None,
            damage: None,
            knockback: None,
            original_damage: None,
        };
        write_packet(&mut owner, &Packet::ProjectileSync(shot)).await?;
        observe(&mut observer, &Packet::ProjectileSync(shot)).await?;
        let old = Packet::ProjectileRemove(ProjectileRemove {
            key: shot.key,
            position: shot.position,
        });
        shot.key.generation = 0;
        shot.position[1] += 16.;
        write_packet(&mut owner, &Packet::ProjectileSync(shot)).await?;
        observe(&mut observer, &old).await?;
        observe(&mut observer, &Packet::ProjectileSync(shot)).await?;
        write_packet(&mut owner, &old).await?;
        observe(&mut observer, &old).await?;
        drop(owner);
        observe(
            &mut observer,
            &Packet::ProjectileRemove(ProjectileRemove {
                key: shot.key,
                position: shot.position,
            }),
        )
        .await?;
        observe(
            &mut observer,
            &Packet::PlayerActive {
                player: slot,
                active: false,
            },
        )
        .await
    }

    #[tokio::test]
    async fn projectile_packets_follow_the_owner() -> Result<()> {
        let world = terraria_world::World::parse(
            include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec(),
        )?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
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
        timeout(Duration::from_secs(8), run(address, None)).await??;
        timeout(Duration::from_secs(8), generation_cleanup(address)).await??;
        stop.send(()).unwrap();
        server.await??;
        Ok(())
    }
}
