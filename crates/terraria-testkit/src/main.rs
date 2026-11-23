//! Direct TCP probes. Keep reference and candidate isolated and idle for slot comparisons.
use std::{env, error::Error, fs, io, net::SocketAddr, path::Path, time::Duration};
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{Appearance, Equipment, GREETING, NetworkText, Packet, Vitals};
use tokio::{net::TcpStream, time::timeout};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, PartialEq)]
enum Scenario {
    Join,
    WrongVersion,
    WrongPassword,
    Upload,
    JourneyOnClassic,
}

impl Scenario {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "join" => Ok(Self::Join),
            "wrong-version" => Ok(Self::WrongVersion),
            "wrong-password" => Ok(Self::WrongPassword),
            "upload" => Ok(Self::Upload),
            "journey-on-classic" => Ok(Self::JourneyOnClassic),
            _ => Err("scenario must be join, wrong-version, wrong-password, upload or journey-on-classic".into()),
        }
    }
}

async fn receive(stream: &mut TcpStream, log: &mut Vec<Packet>) -> Result<Packet> {
    let frame = read_frame(stream)
        .await?
        .ok_or("unexpected end of response sequence")?;
    if !matches!(frame.id, 2 | 3 | 37) {
        return Err(format!("unexpected response ID {}", frame.id).into());
    }
    let packet = Packet::decode(&frame)?;
    println!("  <- {packet:?}  [{:02x?}]", frame.payload);
    log.push(packet.clone());
    Ok(packet)
}

fn appearance(slot: u8) -> Appearance {
    Appearance {
        player: slot,
        skin: 0,
        voice: 1,
        pitch: 0.0,
        hair: 0,
        name: "Probe".into(),
        hair_dye: 0,
        hidden_accessories: 0,
        hidden_misc: 0,
        colors: [[128; 3]; 7],
        difficulty_flags: 0,
        torch_cart_flags: 0,
        permanent_upgrades: 0,
    }
}

async fn upload(stream: &mut TcpStream, slot: u8) -> Result<()> {
    let v = Vitals {
        player: slot,
        current: 100,
        maximum: 100,
    };
    for p in [
        Packet::Appearance(appearance(slot)),
        Packet::ClientUuid("direct-tcp-probe".into()),
        Packet::Life(v),
        Packet::Mana(Vitals {
            current: 20,
            maximum: 20,
            ..v
        }),
        Packet::Buffs {
            player: slot,
            types: vec![],
        },
        Packet::Loadout {
            player: slot,
            index: 0,
            hidden_accessories: 0,
        },
        Packet::Equipment(Equipment {
            player: slot,
            slot: 0,
            stack: 1,
            prefix: 0,
            item: 1,
            flags: 0,
        }),
    ] {
        write_packet(stream, &p).await?;
    }
    Ok(())
}

async fn capture_world(stream: &mut TcpStream, path: &Path) -> Result<()> {
    write_packet(stream, &Packet::RequestWorld).await?;
    let frame = read_frame(stream)
        .await?
        .ok_or("EOF waiting for world data")?;
    if frame.id != 7 {
        return Err(format!("expected WorldData (7), received {}", frame.id).into());
    }
    let len = u16::try_from(frame.payload.len() + 3)?;
    let mut bytes = Vec::with_capacity(usize::from(len));
    bytes.extend(len.to_le_bytes());
    bytes.push(frame.id);
    bytes.extend(frame.payload);
    fs::write(path, bytes)?;
    println!("Recorded {len}-byte WorldData frame.");
    Ok(())
}

async fn probe(
    addr: SocketAddr,
    scenario: Scenario,
    password: Option<&str>,
    capture: Option<&Path>,
) -> Result<Vec<Packet>> {
    println!("{addr}");
    let mut stream = TcpStream::connect(addr).await?;
    let mut log = Vec::new();
    let version = if scenario == Scenario::WrongVersion {
        "Terraria325"
    } else {
        GREETING
    };
    write_packet(&mut stream, &Packet::Hello(version.into())).await?;
    let mut response = receive(&mut stream, &mut log).await?;
    if response == Packet::RequestPassword && scenario != Scenario::WrongVersion {
        let value = if scenario == Scenario::WrongPassword {
            "intentional-mismatch"
        } else {
            password.ok_or("set TERRARIA_PROBE_PASSWORD for a protected server")?
        };
        write_packet(&mut stream, &Packet::Password(value.into())).await?;
        response = receive(&mut stream, &mut log).await?;
    }
    if let Packet::AssignSlot { player, .. } = response {
        if let Some(path) = capture {
            upload(&mut stream, player).await?;
            capture_world(&mut stream, path).await?;
        } else if matches!(scenario, Scenario::Upload | Scenario::JourneyOnClassic) {
            let mut a = appearance(player);
            if scenario == Scenario::Upload {
                upload(&mut stream, player).await?;
                // A second appearance elicits a deterministic result at the end of the upload.
                a.name.clear();
            } else {
                a.difficulty_flags = 8;
            }
            write_packet(&mut stream, &Packet::Appearance(a)).await?;
            receive(&mut stream, &mut log).await?;
        }
    }
    Ok(log)
}

fn validate(scenario: Scenario, responses: &[Packet]) -> Result<()> {
    let authenticated = responses
        .strip_prefix(&[Packet::RequestPassword])
        .unwrap_or(responses);
    let expected_kick = |key| Packet::Kick(NetworkText::key(key));
    let valid = match scenario {
        Scenario::WrongVersion => responses == [expected_kick("LegacyMultiplayer.4")],
        Scenario::WrongPassword => {
            responses
                == [
                    Packet::RequestPassword,
                    expected_kick("LegacyMultiplayer.1"),
                ]
        }
        Scenario::Join => matches!(authenticated, [Packet::AssignSlot { special: false, .. }]),
        Scenario::Upload | Scenario::JourneyOnClassic => {
            let key = if scenario == Scenario::Upload {
                "Net.EmptyName"
            } else {
                "Net.PlayerIsCreativeAndWorldIsNotCreative"
            };
            matches!(authenticated,[Packet::AssignSlot {special:false,..}, kick] if *kick == expected_kick(key))
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("unexpected response sequence {responses:?}").into())
    }
}

fn compare(reference: &[Packet], candidate: &[Packet]) -> Result<()> {
    if reference == candidate {
        return Ok(());
    }
    let index = reference
        .iter()
        .zip(candidate)
        .position(|(a, b)| a != b)
        .unwrap_or(reference.len().min(candidate.len()));
    Err(io::Error::other(format!(
        "first mismatch at response {}: reference {:?}, candidate {:?}",
        index + 1,
        reference.get(index),
        candidate.get(index)
    ))
    .into())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let (left,right,scenario,capture) = match args.as_slice() {
        [cmd,addr,case] if cmd == "probe" => (addr,None,Scenario::parse(case)?,None),
        [cmd,a,b,case] if cmd == "compare" => (a,Some(b),Scenario::parse(case)?,None),
        [cmd,addr,path] if cmd == "capture-world" => (addr,None,Scenario::Join,Some(Path::new(path))),
        _ => return Err("usage: terraria-testkit probe ADDR CASE | compare REFERENCE CANDIDATE CASE | capture-world ADDR OUTPUT".into()),
    };
    let password = env::var("TERRARIA_PROBE_PASSWORD").ok();
    let run = |address: SocketAddr| {
        timeout(
            Duration::from_secs(10),
            probe(address, scenario, password.as_deref(), capture),
        )
    };
    let reference = run(left.parse()?).await??;
    if let Some(right) = right {
        let candidate = run(right.parse()?).await??;
        compare(&reference, &candidate)?;
        validate(scenario, &candidate)?;
        println!("Matched {} response(s).", reference.len());
    } else {
        validate(scenario, &reference)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_identifies_the_first_differing_observation() {
        let accepted = Packet::AssignSlot {
            player: 0,
            special: false,
        };
        let reference = [Packet::RequestPassword, accepted.clone()];
        assert!(compare(&reference, &reference).is_ok());
        assert!(
            compare(&reference, &reference[..1])
                .unwrap_err()
                .to_string()
                .contains("response 2")
        );
        assert!(
            compare(&reference, &[accepted])
                .unwrap_err()
                .to_string()
                .contains("response 1")
        );
        assert!(validate(Scenario::Join, &reference).is_ok());
        assert!(validate(Scenario::Upload, &reference).is_err());
    }
}
