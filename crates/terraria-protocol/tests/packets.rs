use bytes::{Bytes, BytesMut};
use proptest::prelude::*;
use terraria_protocol::{
    Appearance, Equipment, Error, Frame, GREETING, MAX_BUFFS, MAX_FRAME, NetworkText, Packet,
    PlayerPlatform, ServerInfo, Vitals,
};

fn frame(id: u8, payload: &[u8]) -> Frame {
    Frame {
        id,
        payload: Bytes::copy_from_slice(payload),
    }
}

fn examples() -> Vec<(Packet, u8, Vec<u8>)> {
    let a = Appearance {
        player: 3,
        skin: 2,
        voice: 4,
        pitch: -0.5,
        hair: 9,
        name: "é".into(),
        hair_dye: 7,
        hidden_accessories: 258,
        hidden_misc: 6,
        colors: [[1, 2, 3]; 7],
        difficulty_flags: 4,
        torch_cart_flags: 16,
        permanent_upgrades: 80,
    };
    let mut appearance = vec![3, 2, 4, 0, 0, 0, 0xbf, 9, 2, 0xc3, 0xa9, 7, 2, 1, 6];
    appearance.extend([1, 2, 3].repeat(7));
    appearance.extend([4, 16, 80]);
    let vitals = Vitals {
        player: 3,
        current: -10,
        maximum: 300,
    };
    vec![
        (
            Packet::Hello(GREETING.into()),
            1,
            b"\x0bTerraria329".to_vec(),
        ),
        (
            Packet::Kick(NetworkText::key("LegacyMultiplayer.4")),
            2,
            b"\x02\x13LegacyMultiplayer.4\0".to_vec(),
        ),
        (
            Packet::AssignSlot {
                player: 0,
                special: false,
            },
            3,
            vec![0, 0],
        ),
        (Packet::Appearance(a), 4, appearance),
        (
            Packet::PlayerHurt(terraria_protocol::PlayerHurt {
                player: 0,
                reason: terraria_protocol::DeathReason {
                    other: Some(0),
                    ..Default::default()
                },
                damage: 20,
                direction: 0,
                critical: false,
                pvp: false,
                dodgeable: false,
                immunity_slot: -1,
            }),
            117,
            vec![0, 8, 0, 20, 0, 1, 0, 255],
        ),
        (
            Packet::PlayerDeath(terraria_protocol::PlayerDeath {
                player: 0,
                reason: terraria_protocol::DeathReason {
                    other: Some(0),
                    ..Default::default()
                },
                damage: 99,
                direction: 0,
                pvp: false,
            }),
            118,
            vec![0, 8, 0, 99, 0, 1, 0],
        ),
        (
            Packet::PlayerDeath(terraria_protocol::PlayerDeath {
                player: 2,
                reason: terraria_protocol::DeathReason {
                    player: Some(1),
                    npc: Some(4),
                    projectile: Some(901),
                    other: Some(0),
                    projectile_type: Some(3),
                    item: Some(5),
                    prefix: Some(6),
                    custom: Some("ouch".into()),
                },
                damage: 34,
                direction: -1,
                pvp: true,
            }),
            118,
            vec![
                2, 255, 1, 0, 4, 0, 133, 3, 0, 3, 0, 5, 0, 6, 4, 111, 117, 99, 104, 34, 0, 0, 1,
            ],
        ),
        (Packet::SetDeadAppearance(4), 135, vec![4]),
        (Packet::PlayerTeam { player: 4, team: 5 }, 45, vec![4, 5]),
        (
            Packet::PlayerPvp {
                player: 13,
                hostile: true,
            },
            30,
            vec![13, 1],
        ),
        (
            Packet::UseTimeDial {
                player: 2,
                dusk: false,
            },
            51,
            vec![2, 3],
        ),
        (
            Packet::UseTimeDial {
                player: 3,
                dusk: true,
            },
            51,
            vec![3, 6],
        ),
        (
            Packet::Equipment(Equipment {
                player: 3,
                slot: 989,
                stack: 99,
                prefix: 4,
                item: 4000,
                flags: 3,
            }),
            5,
            vec![3, 0xdd, 3, 99, 0, 4, 0xa0, 15, 3],
        ),
        (Packet::RequestWorld, 6, vec![]),
        (Packet::Life(vitals), 16, vec![3, 246, 255, 44, 1]),
        (Packet::RequestPassword, 37, vec![]),
        (Packet::Password("abc".into()), 38, b"\x03abc".to_vec()),
        (Packet::Mana(vitals), 42, vec![3, 246, 255, 44, 1]),
        (
            Packet::Buffs {
                player: 3,
                types: vec![258, 4],
            },
            50,
            vec![3, 2, 1, 4, 0, 0, 0],
        ),
        (Packet::ClientUuid("xyz".into()), 68, b"\x03xyz".to_vec()),
        (
            Packet::Loadout {
                player: 3,
                index: 2,
                hidden_accessories: 256,
            },
            147,
            vec![3, 2, 0, 1],
        ),
        (Packet::HostToken("abc".into()), 161, b"\x03abc".to_vec()),
        (
            Packet::ServerInfo(ServerInfo {
                port: 18890,
                world: "Oracle".into(),
                host: "ekhilaptop".into(),
                width: 4200,
                crimson: true,
                difficulty: 0,
                capacity: 8,
                players: 0,
                note: String::new(),
                mapped_address: None,
            }),
            163,
            b"\xca\x49\0\0\x06Oracle\x0aekhilaptoph\x10\x01\0\0\0\0\x08\0\0\0".to_vec(),
        ),
        (
            Packet::PlayerPlatform(PlayerPlatform {
                player: 3,
                platform: 5,
                id: "123".into(),
                name: "Ada".into(),
                host: false,
            }),
            164,
            b"\x03\x05\x03123\x03Ada\0".to_vec(),
        ),
    ]
}

#[test]
fn layouts_round_trip_and_reject_truncation_and_trailing_bytes() {
    assert!(Packet::decode_client(&frame(117, &[0, 0, 20, 0, 1, 0, 6])).is_err());
    assert!(Packet::decode_client(&frame(117, &[0, 0, 20, 0, 3, 0, 255])).is_err());
    assert!(Packet::decode_client(&frame(117, &[255, 0, 20, 0, 1, 0, 255])).is_err());
    assert_eq!(
        Packet::decode_client(&frame(51, &[2, 1])),
        Err(Error::Unknown(51))
    );
    assert!(Packet::decode_client(&frame(45, &[3, 6])).is_err());
    for (packet, id, payload) in examples() {
        let mut bytes = BytesMut::from(packet.encode().unwrap().as_ref());
        assert_eq!(bytes[..2], ((payload.len() + 3) as u16).to_le_bytes());
        let raw = Frame::split(&mut bytes).unwrap().unwrap();
        assert_eq!(raw, frame(id, &payload));
        assert!(bytes.is_empty());
        assert_eq!(Packet::decode(&raw).unwrap(), packet);
        for cut in 0..payload.len() {
            assert!(
                Packet::decode(&frame(id, &payload[..cut])).is_err(),
                "id={id} cut={cut}"
            );
        }
        let mut extra = payload;
        extra.push(0);
        assert_eq!(Packet::decode(&frame(id, &extra)), Err(Error::Trailing(1)));
    }
}

#[test]
fn fragmented_and_coalesced_frames_are_consumed_exactly_once() {
    let first = Packet::Hello(GREETING.into()).encode().unwrap();
    let second = Packet::RequestPassword.encode().unwrap();
    for cut in 0..first.len() {
        let mut buf = BytesMut::from(&first[..cut]);
        assert_eq!(Frame::split(&mut buf).unwrap(), None);
        assert_eq!(buf.as_ref(), &first[..cut]);
        buf.extend_from_slice(&first[cut..]);
        buf.extend_from_slice(&second);
        assert_eq!(Frame::split(&mut buf).unwrap().unwrap().id, 1);
        assert_eq!(Frame::split(&mut buf).unwrap().unwrap().id, 37);
        assert!(buf.is_empty());
    }
    for len in 0..3 {
        assert_eq!(
            Frame::split(&mut BytesMut::from(&[len, 0][..])),
            Err(Error::Length(len.into()))
        );
    }
}

#[test]
fn codec_limits_and_noncanonical_inputs() {
    for input in [
        &[128; 5][..],
        &[255, 255, 255, 255, 8],
        &[1, 0xff],
        &[0xff, 0x7f],
        &[0x80],
    ] {
        assert!(Packet::decode(&frame(1, input)).is_err());
    }
    assert_eq!(
        Packet::decode(&frame(3, &[9, 255])).unwrap(),
        Packet::AssignSlot {
            player: 9,
            special: true
        }
    );
    assert_eq!(Packet::decode(&frame(253, &[])), Err(Error::Unknown(253)));
    let max = Packet::Hello("a".repeat(MAX_FRAME - 6)).encode().unwrap();
    assert_eq!(max.len(), MAX_FRAME);
    assert_eq!(&max[..2], &[255, 255]);
    assert!(Packet::Hello("a".repeat(MAX_FRAME - 5)).encode().is_err());
    assert!(
        Packet::Buffs {
            player: 0,
            types: vec![0]
        }
        .encode()
        .is_err()
    );
    let mut oversized = vec![3];
    oversized.extend([1, 0].repeat(MAX_BUFFS + 1));
    oversized.extend([0, 0]);
    assert!(Packet::decode(&frame(50, &oversized)).is_err());
    let max_buffs = Packet::Buffs {
        player: 3,
        types: vec![3; MAX_BUFFS],
    };
    assert_eq!(
        Packet::decode(&frame(50, &max_buffs.encode().unwrap()[3..])).unwrap(),
        max_buffs
    );
}

#[test]
fn nested_network_text_and_limits() {
    use NetworkText::*;
    let packet = Packet::Kick(Key(
        "k".into(),
        vec![Formattable("{0}".into(), vec![Literal("é".into())])],
    ));
    let payload = b"\x02\x01k\x01\x01\x03{0}\x01\0\x02\xc3\xa9";
    assert_eq!(&packet.encode().unwrap()[3..], payload);
    assert_eq!(Packet::decode(&frame(2, payload)).unwrap(), packet);
    for bad in [&[3, 0][..], &[2, 0, 1], &[0, 0, 0]] {
        assert!(Packet::decode(&frame(2, bad)).is_err());
    }
    let mut nested = [2, 0, 1].repeat(32);
    nested.extend([0, 0]);
    assert!(Packet::decode(&frame(2, &nested)).is_err());
    assert!(
        Packet::Kick(Key("k".into(), vec![Literal("".into()); 256]))
            .encode()
            .is_err()
    );
}

proptest! {
    #[test]
    fn utf8_string_lengths_count_bytes(text in ".{0,400}") {
        let packet = Packet::Hello(text);
        let wire = packet.encode().unwrap();
        prop_assert_eq!(Packet::decode(&frame(1,&wire[3..])).unwrap(),packet);
    }
    #[test]
    fn arbitrary_wire_data_is_fallible_without_panics(id: u8, payload in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let _ = Packet::decode(&frame(id,&payload));
        let _ = Frame::split(&mut BytesMut::from(payload.as_slice()));
    }
}

#[test]
fn responses_recorded_from_official_1458_server() {
    let cases: &[(&[u8], Packet)] = &[
        (
            b"\x05\0\x03\0\0",
            Packet::AssignSlot {
                player: 0,
                special: false,
            },
        ),
        (b"\x03\0\x25", Packet::RequestPassword),
        (
            b"\x19\0\x02\x02\x13LegacyMultiplayer.4\0",
            Packet::Kick(NetworkText::key("LegacyMultiplayer.4")),
        ),
        (
            b"\x19\0\x02\x02\x13LegacyMultiplayer.1\0",
            Packet::Kick(NetworkText::key("LegacyMultiplayer.1")),
        ),
        (
            b"\x13\0\x02\x02\x0dNet.EmptyName\0",
            Packet::Kick(NetworkText::key("Net.EmptyName")),
        ),
    ];
    for (raw, expected) in cases {
        let f = Frame::split(&mut BytesMut::from(*raw)).unwrap().unwrap();
        assert_eq!(Packet::decode(&f).unwrap(), *expected);
        assert_eq!(expected.encode().unwrap().as_ref(), *raw);
    }
}
