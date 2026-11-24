use std::{io::ErrorKind, num::NonZeroU8, time::Duration};
use terraria_network::{Error, Phase, Session, read_frame, serve, write_packet};
use terraria_protocol::{Equipment, GREETING, NetworkText, Packet, Vitals};
use tokio::{
    io::{AsyncWriteExt, duplex},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

#[test]
fn gates_upload_on_version_and_password() {
    let mut s = Session::new(2, false);
    assert_eq!(
        s.handle(Packet::Hello("different".into()), ""),
        Some(Packet::Kick(NetworkText::key("LegacyMultiplayer.4")))
    );
    assert_eq!(s.phase(), &Phase::Closed);
    assert_eq!(s.handle(Packet::Hello(GREETING.into()), ""), None);
    for p in [Packet::RequestWorld, Packet::Password("right".into())] {
        let mut s = Session::new(2, false);
        assert_eq!(
            s.handle(p, "right"),
            Some(Packet::Kick(NetworkText::key("LegacyMultiplayer.2")))
        );
    }
    let mut s = Session::new(2, false);
    assert_eq!(
        s.handle(Packet::Hello(GREETING.into()), "right"),
        Some(Packet::RequestPassword)
    );
    assert_eq!(s.phase(), &Phase::AwaitPassword);
    assert_eq!(
        s.handle(Packet::Password("wrong".into()), "right"),
        Some(Packet::Kick(NetworkText::key("LegacyMultiplayer.1")))
    );
}

#[test]
fn upload_uses_socket_identity_and_bounds_sparse_slots() {
    let mut s = Session::new(3, false);
    s.handle(Packet::Hello(GREETING.into()), "abc");
    assert_eq!(
        s.handle(Packet::Password("abc".into()), "abc"),
        Some(Packet::AssignSlot {
            player: 3,
            special: false
        })
    );
    let v = Vitals {
        player: 255,
        current: 10,
        maximum: 5,
    };
    s.handle(Packet::Life(v), "abc");
    s.handle(Packet::Mana(v), "abc");
    assert_eq!(
        s.character().life,
        Some(Vitals {
            player: 3,
            maximum: 20,
            ..v
        })
    );
    assert_eq!(s.character().mana.unwrap().player, 3);
    let item = Equipment {
        player: 255,
        slot: 989,
        item: 1,
        prefix: 2,
        stack: 5,
        flags: 1,
    };
    s.handle(Packet::Equipment(item), "abc");
    assert_eq!(s.character().equipment[&989].player, 3);
    s.handle(Packet::Equipment(Equipment { stack: 0, ..item }), "abc");
    assert!(s.character().equipment.is_empty());
    assert!(matches!(
        s.handle(Packet::Equipment(Equipment { slot: 990, ..item }), "abc"),
        Some(Packet::Kick(_))
    ));
}

#[tokio::test]
async fn end_of_stream_and_partial_reads() {
    for data in [&[][..], &[4], &[4, 0], &[4, 0, 68], &[2, 0]] {
        let mut input = data;
        let result = read_frame(&mut input).await;
        match data {
            [] => assert!(result.unwrap().is_none()),
            [2, 0] => assert!(matches!(result, Err(Error::Protocol(_)))),
            _ => assert!(matches!(result,Err(Error::Io(e)) if e.kind()==ErrorKind::UnexpectedEof)),
        }
    }
    let (mut w, mut r) = duplex(1);
    let writer = tokio::spawn(async move {
        w.write_all(&[3, 0, 37, 5, 0, 3, 7, 0]).await.unwrap();
    });
    for p in [
        Packet::RequestPassword,
        Packet::AssignSlot {
            player: 7,
            special: false,
        },
    ] {
        assert_eq!(
            Packet::decode(&read_frame(&mut r).await.unwrap().unwrap()).unwrap(),
            p
        );
    }
    writer.await.unwrap();
}

async fn recv(s: &mut TcpStream) -> Packet {
    Packet::decode(&read_frame(s).await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn loopback_admission_capacity_reconnect_and_shutdown() {
    timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(serve(
            listener,
            NonZeroU8::new(2).unwrap(),
            String::new(),
            None,
            async {
                let _ = stopped.await;
            },
        ));
        let mut clients = Vec::new();
        for player in 0..2 {
            let mut s = TcpStream::connect(addr).await.unwrap();
            write_packet(&mut s, &Packet::Hello(GREETING.into()))
                .await
                .unwrap();
            assert_eq!(
                recv(&mut s).await,
                Packet::AssignSlot {
                    player,
                    special: false
                }
            );
            clients.push(s);
        }
        let mut overflow = TcpStream::connect(addr).await.unwrap();
        assert_eq!(
            recv(&mut overflow).await,
            Packet::Kick(NetworkText::key("CLI.ServerIsFull"))
        );
        let mut left = clients.remove(0);
        write_packet(&mut left, &Packet::RequestWorld)
            .await
            .unwrap();
        assert!(matches!(
            recv(&mut left).await,
            Packet::Kick(NetworkText::Literal(_))
        ));
        assert!(read_frame(&mut left).await.unwrap().is_none());
        let mut again = TcpStream::connect(addr).await.unwrap();
        write_packet(&mut again, &Packet::Hello(GREETING.into()))
            .await
            .unwrap();
        assert_eq!(
            recv(&mut again).await,
            Packet::AssignSlot {
                player: 0,
                special: false
            }
        );
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(read_frame(&mut again).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[test]
fn appearance_normalization_and_world_mode_admission() {
    use terraria_protocol::Appearance;
    let base = Appearance {
        player: 200,
        skin: 255,
        voice: 0,
        pitch: f32::NAN,
        hair: 228,
        name: "  Terra  ".into(),
        hair_dye: 0,
        hidden_accessories: 0,
        hidden_misc: 0,
        colors: [[0; 3]; 7],
        difficulty_flags: 0,
        torch_cart_flags: 0,
        permanent_upgrades: 0,
    };
    for (journey_world, journey_character, reason) in [
        (false, false, None),
        (true, true, None),
        (
            true,
            false,
            Some("Net.PlayerIsNotCreativeAndWorldIsCreative"),
        ),
        (
            false,
            true,
            Some("Net.PlayerIsCreativeAndWorldIsNotCreative"),
        ),
    ] {
        let mut s = Session::new(7, journey_world);
        s.handle(Packet::Hello(GREETING.into()), "");
        let mut a = base.clone();
        if journey_character {
            a.difficulty_flags |= 8;
        }
        assert_eq!(
            s.handle(Packet::Appearance(a), ""),
            reason.map(|key| Packet::Kick(NetworkText::key(key)))
        );
        if reason.is_none() {
            let a = s.character().appearance.as_ref().unwrap();
            assert_eq!(
                (a.player, a.skin, a.voice, a.hair, a.name.as_str()),
                (7, 11, 1, 0, "Terra")
            );
            assert_eq!(a.pitch, 0.0);
        }
    }
    for (name, key) in [
        ("  ".into(), "Net.EmptyName"),
        ("🍄".repeat(11), "Net.NameTooLong"),
    ] {
        let mut s = Session::new(4, false);
        s.handle(Packet::Hello(GREETING.into()), "");
        assert_eq!(
            s.handle(
                Packet::Appearance(Appearance {
                    name,
                    ..base.clone()
                }),
                ""
            ),
            Some(Packet::Kick(NetworkText::key(key)))
        );
    }
}
