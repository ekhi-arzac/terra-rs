use bytes::{Bytes, BytesMut};
use flate2::{Compression, write::DeflateEncoder};
use std::io::Write;
use terraria_binary::{
    Reader,
    tiles::{Liquid, Tile},
};
use terraria_protocol::{
    Frame, Movement, NetworkText, Packet, Spawn,
    section::{SectionObject, TileEntity, TileRun, TileSection},
};

fn frame(id: u8, payload: &[u8]) -> Frame {
    Frame {
        id,
        payload: Bytes::copy_from_slice(payload),
    }
}
fn recorded(data: &[u8]) -> Packet {
    let mut input = BytesMut::from(data);
    let f = Frame::split(&mut input).unwrap().unwrap();
    assert!(input.is_empty());
    Packet::decode(&f).unwrap()
}
fn movement() -> Movement {
    Movement {
        player: 2,
        flags: [72, 132, 64, 32],
        selected: 1,
        position: [44., 26.],
        velocity: Some([1.5, -2.]),
        mount: Some(2),
        return_points: Some([[1., 2.], [3., 4.]]),
        camera: Some([0., 100.]),
    }
}

#[test]
fn entry_and_player_wire_layouts_roundtrip_and_reject_bad_lengths() {
    let packets = [
        Packet::DroppedItem(terraria_protocol::DroppedItem {
            slot: 400,
            position: [4.0, 6.0],
            velocity: [0.1, -2.0],
            stack: 30,
            prefix: 4,
            ownership: 2,
            item: 1,
            shimmer: Some((true, 0.6)),
            enemy_delay: Some(90),
        }),
        Packet::ItemOwner(terraria_protocol::ItemOwner {
            slot: 399,
            player: 23,
            reservation: 180,
            delayed_player: 255,
            delay: 10000,
            position: [1., 2.],
        }),
        Packet::ReleaseItem {
            slot: 399,
            force: true,
        },
        Packet::RemoveItem(399),
        Packet::RequestChest([2095, 232]),
        Packet::RequestChestName {
            chest: -1,
            position: [2, 4],
        },
        Packet::ChestName {
            chest: 1,
            position: [2, 4],
            name: "Bois 🪵".into(),
        },
        Packet::PlayerChest {
            player: 2,
            chest: 129,
        },
        Packet::ChestState {
            chest: 4,
            position: [18, 20],
            name: Some("Café".into()),
        },
        Packet::DoorToggle {
            action: 0,
            position: [2220, 220],
            facing_right: false,
        },
        Packet::ObjectPlacement {
            position: [2102, 233],
            tile: 18,
            style: 0,
            alternate: 0,
            random: -1,
            facing_right: true,
        },
        Packet::TilePatch(Box::new(terraria_protocol::TilePatch {
            origin: [24, 52],
            size: [1, 2],
            reason: 0,
            tiles: vec![
                Tile::default(),
                Tile {
                    block: Some(5),
                    frame: Some((22, 198)),
                    wall: 310,
                    block_paint: 11,
                    wall_paint: 3,
                    liquid: Some((Liquid::Honey, 40)),
                    flags: [0, 0x2e, 0x26, 0x1e],
                },
            ],
        })),
        Packet::TileChange {
            action: 20,
            position: [21, 22],
            value: 0,
            style: 0,
        },
        Packet::RequestSections {
            spawn: [-1, 100],
            team: 3,
        },
        Packet::Status {
            count: 15,
            text: NetworkText::key("LegacyInterface.44"),
            flags: 0,
        },
        Packet::ChestSize {
            index: 26,
            capacity: 40,
        },
        Packet::ChestItem {
            chest: 26,
            slot: 0,
            stack: 1,
            prefix: 0,
            item: 832,
        },
        Packet::BannerState(Box::default()),
        Packet::TowerShields([0, 50, 100, 0]),
        Packet::CavernMonsters([494, 494, 494, 497, 496, 496]),
        Packet::PlayerZones {
            player: 1,
            zones: [8, 0, 0, 0, 4],
            town_npcs: 2,
        },
        Packet::NpcName {
            slot: 1,
            name: "Tanner".into(),
            variation: 0,
        },
        Packet::RequestNpcName(1),
        Packet::NpcHome {
            slot: 1,
            position: [2095, 232],
            state: 1,
        },
        Packet::NpcBuffs {
            slot: 1,
            buffs: vec![(32, 600), (4, 17)],
        },
        Packet::Ping,
        Packet::SpawnAllowed,
        Packet::Spawn(Spawn {
            player: 2,
            point: [-1, -1],
            respawn_timer: 0,
            deaths: [3, 4],
            team: 0,
            context: 1,
        }),
        Packet::FinishedConnecting,
        Packet::Movement(movement()),
        Packet::PlayerActive {
            player: 2,
            active: true,
        },
        Packet::ChatMessage {
            author: 2,
            text: NetworkText::Literal("hello".into()),
            color: [255; 3],
        },
        Packet::ChatCommand {
            command: "Say".into(),
            text: "hello".into(),
        },
        Packet::RequestSection([20, 7]),
        recorded(include_bytes!("../../../docs/fixtures/world-info-326.bin")),
    ];
    for p in packets {
        let raw = p.encode().unwrap();
        let f = frame(p.id(), &raw[3..]);
        let decode = if matches!(
            p,
            Packet::ChatCommand { .. }
                | Packet::RequestNpcName(_)
                | Packet::RequestChestName { .. }
        ) {
            Packet::decode_client
        } else {
            Packet::decode
        };
        assert_eq!(decode(&f).unwrap(), p);
        for end in 0..f.payload.len() {
            assert!(
                decode(&frame(f.id, &f.payload[..end])).is_err(),
                "ID {} at {end}",
                f.id
            );
        }
        let mut trailing = f.payload.to_vec();
        trailing.push(0);
        if matches!(p, Packet::RequestChestName { .. }) {
            assert_eq!(decode(&frame(f.id, &trailing)).unwrap(), p); // Stock cached-name string.
            trailing.push(0);
        }
        assert!(decode(&frame(f.id, &trailing)).is_err());
    }
    assert_eq!(
        &Packet::ChestItem {
            chest: 26,
            slot: 0,
            stack: 1,
            prefix: 0,
            item: 832
        }
        .encode()
        .unwrap()[3..],
        b"\x1a\0\0\x01\0\0\x40\x03"
    );
    let world = include_bytes!("../../../docs/fixtures/world-info-326.bin");
    assert_eq!(recorded(world).encode().unwrap().as_ref(), world);
    let mut bad = Packet::Movement(movement()).encode().unwrap().to_vec();
    bad[9..13].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(Packet::decode(&frame(13, &bad[3..])).is_err());
}
#[test]
fn official_section_decodes_and_reencodes_semantically() {
    let raw = include_bytes!("../../../docs/fixtures/tile-section-326.bin");
    let packet = recorded(raw);
    let Packet::TileSection(s) = &packet else {
        panic!()
    };
    assert_eq!((s.origin, s.size), ([1600, 150], [200, 150]));
    assert_eq!(
        s.runs.iter().map(|r| usize::from(r.count)).sum::<usize>(),
        30000
    );
    assert_eq!(s.chests.iter().map(|o| o.index).collect::<Vec<_>>(), [26]);
    assert_eq!(recorded(&packet.encode().unwrap()), packet);
    for end in [0, 1, raw.len() / 2, raw.len() - 4] {
        assert!(Packet::decode(&frame(10, &raw[3..3 + end])).is_err());
    }
    let mut trailing = raw[3..].to_vec();
    trailing.push(0);
    assert!(Packet::decode(&frame(10, &trailing)).is_err());
}
#[test]
fn chained_flags_extended_ids_liquids_and_tile_entities() {
    let tile = Tile {
        block: Some(21),
        frame: Some((0, 18)),
        wall: 262,
        block_paint: 9,
        wall_paint: 7,
        liquid: Some((Liquid::Shimmer, 120)),
        flags: [0, 0x7e, 0x26, 0x1e],
    };
    let mut entities = Vec::new();
    for (kind, extra) in [
        (0, vec![0xff; 2]),
        (1, vec![0; 5]),
        (2, vec![0; 2]),
        (3, [vec![1, 2, 4, 7], vec![11; 25]].concat()),
        (4, vec![0; 5]),
        (5, [vec![9], vec![2; 10]].concat()),
        (6, vec![0; 5]),
        (7, vec![]),
        (8, vec![0; 5]),
        (9, vec![0; 2]),
        (10, vec![0; 2]),
    ] {
        let mut record = vec![kind];
        record.extend([0; 8]);
        record.extend(extra);
        let mut r = Reader(&record);
        entities.push(TileEntity::read(&mut r).unwrap());
        assert!(r.0.is_empty());
        for end in 0..record.len() {
            assert!(TileEntity::read(&mut Reader(&record[..end])).is_err());
        }
    }
    let s = TileSection {
        origin: [200, 150],
        size: [200, 2],
        runs: vec![TileRun { tile, count: 400 }],
        signs: vec![SectionObject {
            index: 2,
            position: [200, 150],
            text: "é".into(),
        }],
        chests: vec![],
        entities,
    };
    let p = Packet::TileSection(Box::new(s));
    assert_eq!(recorded(&p.encode().unwrap()), p);
    let raw_section = |raw: &[u8]| {
        let mut z = DeflateEncoder::new(Vec::new(), Compression::fast());
        z.write_all(raw).unwrap();
        frame(10, &z.finish().unwrap())
    };
    let mut overrun = vec![0; 8];
    overrun.extend([1, 0, 1, 0, 64, 1]);
    overrun.extend([0; 6]);
    assert!(Packet::decode(&raw_section(&overrun)).is_err());
    overrun[8] = 0;
    assert!(Packet::decode(&raw_section(&overrun)).is_err());
    assert!(Packet::decode(&raw_section(&vec![0; 2 * 1024 * 1024 + 1])).is_err());
}

#[test]
fn resident_capture_and_optional_npc_fields() {
    use terraria_protocol::NpcLife;
    // Guide, first cold join to the 1.4.6.1 server using entry-326.wld.
    let capture = [
        1, 0, 0x5a, 0x1b, 3, 0x47, 0, 0x80, 0x65, 0x45, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 130, 0, 22,
        0,
    ];
    let Packet::NpcSync(mut npc) = Packet::decode(&frame(23, &capture)).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        (npc.slot, npc.kind, npc.target, npc.life),
        (1, 22, 255, NpcLife::Full)
    );
    assert_eq!(Packet::NpcSync(npc).encode().unwrap()[3..], capture);
    npc.ai = [Some(2.), Some(3.), None, Some(5.)];
    npc.scaled_players = Some(3);
    npc.difficulty = Some(2.);
    npc.spawning = true;
    npc.shimmer = true;
    npc.kind = 46;
    npc.release_owner = Some(17);
    for life in [
        NpcLife::Full,
        NpcLife::Byte(10),
        NpcLife::Short(240),
        NpcLife::Int(40000),
    ] {
        npc.life = life;
        let packet = Packet::NpcSync(npc);
        let data = packet.encode().unwrap();
        assert_eq!(Packet::decode(&frame(23, &data[3..])).unwrap(), packet);
        for end in 3..data.len() {
            assert!(Packet::decode(&frame(23, &data[3..end])).is_err());
        }
        let mut extra = data[3..].to_vec();
        extra.push(0);
        assert!(Packet::decode(&frame(23, &extra)).is_err());
    }
}
