//! Direct TCP probes. Keep reference and candidate isolated and idle for slot comparisons.
mod clock;
mod entry;
mod hurt;
mod liquids;
mod peers;
mod projectiles;
mod resources;
mod respawn;
mod signs;
mod teams;
use std::{env, error::Error, fs, io, net::SocketAddr, path::Path, time::Duration};
use terraria_network::{read_frame, write_packet};
use terraria_protocol::{Appearance, Equipment, GREETING, NetworkText, Packet, Vitals};
use tokio::{net::TcpStream, time::timeout};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, PartialEq)]
enum Scenario {
    Join,
    Enter,
    EnterPersonal,
    Town,
    WrongVersion,
    WrongPassword,
    Upload,
    JourneyOnClassic,
}

impl Scenario {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "town" => Ok(Self::Town),
            "enter" => Ok(Self::Enter),
            "enter-personal" => Ok(Self::EnterPersonal),
            "join" => Ok(Self::Join),
            "wrong-version" => Ok(Self::WrongVersion),
            "wrong-password" => Ok(Self::WrongPassword),
            "upload" => Ok(Self::Upload),
            "journey-on-classic" => Ok(Self::JourneyOnClassic),
            _ => Err("scenario must be town, enter, enter-personal, join, wrong-version, wrong-password, upload or journey-on-classic".into()),
        }
    }
}

async fn receive(stream: &mut TcpStream, log: &mut Vec<Packet>) -> Result<Packet> {
    let mut frame = read_frame(stream)
        .await?
        .ok_or("unexpected end of response sequence")?;
    if frame.id == 163 {
        let info = Packet::decode(&frame)?;
        println!("  <- {info:?}");
        frame = read_frame(stream)
            .await?
            .ok_or("missing admission response")?;
    }
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
        Packet::Appearance(Appearance {
            name: format!("Probe{slot}"),
            ..appearance(slot)
        }),
        Packet::ClientUuid("direct-tcp-probe".into()),
        Packet::PlayerPlatform(terraria_protocol::PlayerPlatform {
            player: slot,
            platform: 0,
            id: String::new(),
            name: String::new(),
            host: false,
        }),
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
        "Terraria326"
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
        } else if matches!(
            scenario,
            Scenario::Enter | Scenario::EnterPersonal | Scenario::Town
        ) {
            let spawn = if scenario != Scenario::EnterPersonal {
                [-1, -1]
            } else {
                [410, 465]
            };
            entry::run(
                &mut stream,
                player,
                &mut log,
                spawn,
                scenario == Scenario::Town,
            )
            .await?;
            if scenario == Scenario::Town {
                return town_snapshot(&mut stream, log).await;
            }
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

async fn town_snapshot(stream: &mut TcpStream, log: Vec<Packet>) -> Result<Vec<Packet>> {
    // Section entry and the full spawn list can announce the same NPC twice.
    // The housing list identifies permanent residents independently of wildlife.
    let mut records = std::collections::BTreeMap::new();
    let mut houses = std::collections::BTreeSet::new();
    for packet in log {
        let slot = match &packet {
            Packet::NpcSync(npc) => i16::from(npc.slot),
            Packet::NpcBuffs { slot, .. } => *slot,
            Packet::NpcHome { slot, .. } => {
                houses.insert(*slot);
                *slot
            }
            _ => continue,
        };
        records.insert((slot, packet.id()), packet);
    }
    if houses.is_empty() {
        return Err("comparison requires a populated town".into());
    }
    records.retain(|(slot, _), _| houses.contains(slot));
    for slot in houses {
        if !records.contains_key(&(slot, 23)) || !records.contains_key(&(slot, 54)) {
            return Err(format!("missing NPC state or buffs for resident {slot}").into());
        }
        write_packet(stream, &Packet::RequestNpcName(slot)).await?;
        loop {
            let frame = read_frame(stream)
                .await?
                .ok_or("EOF waiting for resident name")?;
            if frame.id == 2 {
                return Err("disconnected waiting for resident name".into());
            }
            if frame.id == 56 {
                let packet = Packet::decode(&frame)?;
                if !matches!(&packet, Packet::NpcName{slot:n,..} if *n==slot) {
                    return Err("resident name slot mismatch".into());
                }
                records.insert((slot, 56), packet);
                break;
            }
        }
    }
    println!(
        "Received state, buffs, housing and name for {} residents.",
        records.len() / 4
    );
    Ok(records.into_values().collect())
}

fn validate(scenario: Scenario, responses: &[Packet]) -> Result<()> {
    let authenticated = responses
        .strip_prefix(&[Packet::RequestPassword])
        .unwrap_or(responses);
    let expected_kick = |key| Packet::Kick(NetworkText::key(key));
    let valid = match scenario {
        Scenario::Town => !responses.is_empty() && responses.len().is_multiple_of(4),
        Scenario::Enter | Scenario::EnterPersonal => matches!(
            authenticated,
            [
                Packet::AssignSlot { .. },
                Packet::WorldData(_),
                Packet::WorldData(_),
                Packet::Status { .. },
                ..,
                Packet::SpawnAllowed,
                Packet::FinishedConnecting
            ]
        ),
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
    if let (Some(Packet::TileSection(a)), Some(Packet::TileSection(b))) =
        (reference.get(index), candidate.get(index))
    {
        let cells = |s: &terraria_protocol::section::TileSection| {
            s.runs
                .iter()
                .flat_map(|r| std::iter::repeat_n(r.tile, usize::from(r.count)))
                .collect::<Vec<_>>()
        };
        let a_tiles = cells(a);
        let b_tiles = cells(b);
        let first = a_tiles.iter().zip(&b_tiles).position(|(a, b)| a != b);
        return Err(format!("first differing section at response {}, origins {:?}/{:?}, first differing tile {:?}, chest/sign/entity lists equal {}/{}/{}",index+1,a.origin,b.origin,first.map(|i|(a.origin[0]+(i%usize::from(a.size[0])) as i32,a.origin[1]+(i/usize::from(a.size[0])) as i32,a_tiles[i],b_tiles[i])),a.chests==b.chests,a.signs==b.signs,a.entities==b.entities).into());
    }
    Err(io::Error::other(format!(
        "first mismatch at response {}: reference {:?}, candidate {:?}",
        index + 1,
        reference.get(index),
        candidate.get(index)
    ))
    .into())
}

fn save_fixture(world: terraria_world::World, directory: &Path, name: &str) -> Result<()> {
    fs::create_dir(directory)?;
    let path = directory.join(name);
    world.save(&path)?;
    println!("{}", path.display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if let [cmd, addr] = args.as_slice() {
        let password = env::var("TERRARIA_PROBE_PASSWORD").ok();
        let deadline = Duration::from_secs(30);
        match cmd.as_str() {
            "liquid-fixture" => return liquids::prepare(Path::new(addr)),
            "liquids" => {
                return timeout(deadline, liquids::run(addr.parse()?, password.as_deref())).await?;
            }
            "sign-fixture" => return signs::prepare(Path::new(addr)),
            "signs" => {
                return timeout(deadline, signs::run(addr.parse()?, password.as_deref())).await?;
            }
            "teams" => {
                return timeout(deadline, teams::run(addr.parse()?, password.as_deref())).await?;
            }
            "projectiles" => {
                return timeout(
                    deadline,
                    projectiles::run(addr.parse()?, password.as_deref()),
                )
                .await?;
            }
            "hurt" => {
                return timeout(deadline, hurt::run(addr.parse()?, password.as_deref())).await?;
            }
            "respawn" => {
                return timeout(deadline, respawn::run(addr.parse()?, password.as_deref())).await?;
            }
            "clock" => {
                timeout(deadline, clock::run(addr.parse()?, password.as_deref())).await??;
                return Ok(());
            }
            _ => {}
        }
    }
    if let [cmd, addr, x, y] = args.as_slice()
        && cmd == "resources"
    {
        return timeout(
            Duration::from_secs(30),
            resources::run(
                addr.parse()?,
                env::var("TERRARIA_PROBE_PASSWORD").ok().as_deref(),
                [x.parse()?, y.parse()?],
            ),
        )
        .await?;
    }
    if let [cmd, addresses @ ..] = args.as_slice()
        && cmd == "peers"
        && (1..=2).contains(&addresses.len())
    {
        let password = env::var("TERRARIA_PROBE_PASSWORD").ok();
        for addr in addresses {
            timeout(
                Duration::from_secs(30),
                peers::run(addr.parse()?, password.as_deref()),
            )
            .await??;
        }
        return Ok(());
    }
    let (left,right,scenario,capture) = match args.as_slice() {
        [cmd,addr,case] if cmd == "probe" => (addr,None,Scenario::parse(case)?,None),
        [cmd,a,b,case] if cmd == "compare" => (a,Some(b),Scenario::parse(case)?,None),
        [cmd,addr,path] if cmd == "capture-world" => (addr,None,Scenario::Join,Some(Path::new(path))),
        _ => return Err("usage: terraria-testkit liquid-fixture NEW_DIR | liquids ADDR | sign-fixture NEW_DIR | signs ADDR | clock ADDR | teams ADDR | respawn ADDR | hurt ADDR | projectiles ADDR | resources ADDR CHEST_X CHEST_Y | peers ADDR [ADDR] | probe ADDR CASE | compare REFERENCE CANDIDATE CASE | capture-world ADDR OUTPUT".into()),
    };
    let password = env::var("TERRARIA_PROBE_PASSWORD").ok();
    let run = |address: SocketAddr| {
        timeout(
            Duration::from_secs(30),
            probe(address, scenario, password.as_deref(), capture),
        )
    };
    let reference = run(left.parse()?).await??;
    if let Some(right) = right {
        let candidate = run(right.parse()?).await??;
        compare(&reference, &candidate)?;
        validate(scenario, &candidate)?;
        println!("Matched {} compared responses.", reference.len());
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

#[cfg(test)]
mod town_tests {
    use super::*;
    use tokio::{net::TcpListener, sync::oneshot};

    #[tokio::test]
    async fn saved_guide_and_old_man_are_available_on_each_join() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, end) = oneshot::channel();
        let world = terraria_world::World::parse(
            include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec(),
        )
        .unwrap();
        let server = tokio::spawn(terraria_network::serve(
            listener,
            2.try_into().unwrap(),
            String::new(),
            Some(world),
            async {
                let _ = end.await;
            },
        ));
        let run = || {
            timeout(
                Duration::from_secs(5),
                probe(addr, Scenario::Town, None, None),
            )
        };
        let first = run().await.unwrap().unwrap();
        assert!(
            first
                .iter()
                .any(|p| matches!(p,Packet::NpcSync(n) if n.slot==0 && n.kind==37))
        );
        assert!(first.contains(&Packet::NpcName {
            slot: 1,
            name: "Tanner".into(),
            variation: 0
        }));
        assert!(first.contains(&Packet::NpcHome {
            slot: 1,
            position: [2095, 232],
            state: 1
        }));
        assert_eq!(first.len(), 8);
        assert_eq!(first, run().await.unwrap().unwrap());
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    }
}
