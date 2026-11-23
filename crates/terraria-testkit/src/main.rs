//! Direct TCP probes. Keep reference and candidate isolated and idle for slot comparisons.
use std::{env, error::Error, io, net::SocketAddr, time::Duration};
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
}

impl Scenario {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "join" => Ok(Self::Join),
            "wrong-version" => Ok(Self::WrongVersion),
            "wrong-password" => Ok(Self::WrongPassword),
            "upload" => Ok(Self::Upload),
            _ => Err("scenario must be join, wrong-version, wrong-password or upload".into()),
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

async fn upload(stream: &mut TcpStream, slot: u8) -> Result<()> {
    let mut a = Appearance {
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
    };
    let v = Vitals {
        player: slot,
        current: 100,
        maximum: 100,
    };
    for p in [
        Packet::Appearance(a.clone()),
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
    // A second appearance elicits a deterministic response at the end of the upload.
    a.name.clear();
    write_packet(stream, &Packet::Appearance(a)).await?;
    Ok(())
}

async fn probe(
    addr: SocketAddr,
    scenario: Scenario,
    password: Option<&str>,
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
    if scenario == Scenario::WrongVersion {
        if response != Packet::Kick(NetworkText::key("LegacyMultiplayer.4")) {
            return Err("expected version rejection".into());
        }
        return Ok(log);
    }
    if response == Packet::RequestPassword {
        let value = if scenario == Scenario::WrongPassword {
            "intentional-mismatch"
        } else {
            password.ok_or("set TERRARIA_PROBE_PASSWORD for a protected server")?
        };
        write_packet(&mut stream, &Packet::Password(value.into())).await?;
        response = receive(&mut stream, &mut log).await?;
    }
    if scenario == Scenario::WrongPassword {
        if response != Packet::Kick(NetworkText::key("LegacyMultiplayer.1")) {
            return Err("expected a password-protected server and a password rejection".into());
        }
    } else if let Packet::AssignSlot {
        player,
        special: false,
    } = response
    {
        if scenario == Scenario::Upload {
            upload(&mut stream, player).await?;
            if receive(&mut stream, &mut log).await?
                != Packet::Kick(NetworkText::key("Net.EmptyName"))
            {
                return Err("expected empty-name rejection after the upload".into());
            }
        }
    } else {
        return Err("expected a dedicated-server slot assignment".into());
    }
    Ok(log)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let (left, right, scenario) = match args.as_slice() {
        [cmd, addr, case] if cmd == "probe" => (addr, None, case),
        [cmd, a, b, case] if cmd == "compare" => (a, Some(b), case),
        _ => return Err(
            "usage: terraria-testkit probe ADDR SCENARIO | compare REFERENCE CANDIDATE SCENARIO"
                .into(),
        ),
    };
    let scenario = Scenario::parse(scenario)?;
    let password = env::var("TERRARIA_PROBE_PASSWORD").ok();
    let run = |address: SocketAddr| {
        timeout(
            Duration::from_secs(10),
            probe(address, scenario, password.as_deref()),
        )
    };
    let reference = run(left.parse()?).await??;
    if let Some(right) = right {
        let candidate = run(right.parse()?).await??;
        if reference != candidate {
            let index = reference
                .iter()
                .zip(&candidate)
                .position(|(a, b)| a != b)
                .unwrap_or(reference.len().min(candidate.len()));
            return Err(io::Error::other(format!(
                "response sequences diverged at message {}",
                index + 1
            ))
            .into());
        }
        println!("Matched {} response(s).", reference.len());
    }
    Ok(())
}
