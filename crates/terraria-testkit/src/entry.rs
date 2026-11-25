use super::{Result, upload};
use std::collections::{BTreeMap, BTreeSet};
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{Packet, Spawn};
use tokio::net::TcpStream;

pub(super) async fn run(
    stream: &mut TcpStream,
    player: u8,
    log: &mut Vec<Packet>,
    personal: [i32; 2],
    capture_town: bool,
) -> Result<()> {
    upload(stream, player).await?;
    write_packet(stream, &Packet::RequestWorld).await?;
    let mut ancillary = BTreeMap::<u8, usize>::new();
    let frame = loop {
        let frame = read_frame(stream)
            .await?
            .ok_or("EOF waiting for world data")?;
        if frame.id == 7 {
            break frame;
        }
        if !matches!(frame.id, 14 | 23 | 54 | 82) {
            return Err(format!("unexpected response {} before world data", frame.id).into());
        }
        *ancillary.entry(frame.id).or_default() += 1;
    };
    let Packet::WorldData(info) = Packet::decode(&frame)? else {
        return Err("expected world data".into());
    };
    println!(
        "  {} ({} × {}) spawn {:?}",
        info.name, info.size[0], info.size[1], info.spawn
    );
    log.push(Packet::WorldData(info));
    write_packet(
        stream,
        &Packet::RequestSections {
            spawn: personal,
            team: 0,
        },
    )
    .await?;
    let mut coords = BTreeSet::new();
    let mut expected = None;
    let mut sizes = BTreeMap::new();
    let mut slots = BTreeSet::new();
    let mut invited = false;
    for _ in 0..65536 {
        let frame = read_frame(stream).await?.ok_or("EOF during world entry")?;
        match frame.id {
            2 => return Err(format!("server closed entry: {:?}", Packet::decode(&frame)?).into()),
            23 | 54 if capture_town && !invited => log.push(Packet::decode(&frame)?),
            60 if capture_town => log.push(Packet::decode(&frame)?),
            7 | 9 | 10 | 20 | 21 | 22 | 32 | 49 | 101 | 129 | 136 | 155 | 82
                if frame.id != 82 || frame.payload.starts_with(&[11, 0, 0]) =>
            {
                let p = Packet::decode(&frame)?;
                match &p {
                    Packet::Status { count, .. } => {
                        if expected.replace(*count).is_some() {
                            return Err("duplicate status".into());
                        }
                    }
                    Packet::TileSection(s) => {
                        if !invited && !coords.insert(s.origin) {
                            return Err("duplicate section".into());
                        }
                    }
                    Packet::ChestSize { index, capacity } => {
                        if !(0..=256).contains(capacity)
                            || (sizes.insert(*index, *capacity).is_some() && !invited)
                        {
                            return Err("invalid chest size or duplicate chest".into());
                        }
                    }
                    Packet::ChestItem { chest, slot, .. } => {
                        if !invited
                            && (!sizes.get(chest).is_some_and(|n| i16::from(*slot) < *n)
                                || !slots.insert((*chest, *slot)))
                        {
                            return Err("invalid or duplicate chest slot".into());
                        }
                    }
                    Packet::SpawnAllowed => {
                        if invited
                            || expected != Some(coords.len() as i32)
                            || coords.is_empty()
                            || sizes.values().map(|v| *v as usize).sum::<usize>() != slots.len()
                        {
                            return Err("incomplete section or chest transfer".into());
                        }
                        invited = true;
                        write_packet(
                            stream,
                            &Packet::Spawn(Spawn {
                                player,
                                point: [-1, -1],
                                respawn_timer: 0,
                                deaths: [0, 0],
                                team: 0,
                                context: 1,
                            }),
                        )
                        .await?;
                    }
                    Packet::FinishedConnecting if invited => {
                        log.push(p);
                        println!(
                            "  Entered after {} sections, {} chests and {} slots. Other received messages by ID {ancillary:?}",
                            coords.len(),
                            sizes.len(),
                            slots.len()
                        );
                        return Ok(());
                    }
                    Packet::FinishedConnecting => {
                        return Err("unexpected entry acknowledgement".into());
                    }
                    _ => {}
                }
                log.push(p);
            }
            _ => {
                *ancillary.entry(frame.id).or_default() += 1;
            }
        }
    }
    Err("world-entry response limit exceeded".into())
}
