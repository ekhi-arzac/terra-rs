//! The roster owner serializes membership and player updates. Socket writes never run on it.
use crate::terrain::LoadedWorld;
use crate::{Character, Error, read_frame, write_packet};
use bytes::{Bytes, BytesMut};
use std::{collections::BTreeMap, io, time::Duration};
use terraria_protocol::{Equipment, Error as ProtocolError, Movement, NetworkText, Packet};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc, time::timeout};

pub(super) enum Event {
    Join {
        slot: u8,
        character: Box<Character>,
        out: mpsc::Sender<Bytes>,
    },
    Update {
        slot: u8,
        packet: Packet,
    },
    Leave(u8),
}

struct Player {
    character: Box<Character>,
    movement: Option<Movement>,
    zones: Option<([u8; 5], u8)>,
    out: Option<mpsc::Sender<Bytes>>,
}
impl Player {
    fn snapshot(&self, slot: u8) -> Result<Bytes, ProtocolError> {
        let mut bytes = BytesMut::new();
        let mut add = |p: Packet| -> Result<(), ProtocolError> {
            bytes.extend_from_slice(&p.encode()?);
            Ok(())
        };
        add(Packet::PlayerActive {
            player: slot,
            active: true,
        })?;
        if let Some(a) = &self.character.appearance {
            add(Packet::Appearance(a.clone()))?;
        }
        if let Some((zones, town_npcs)) = self.zones {
            add(Packet::PlayerZones {
                player: slot,
                zones,
                town_npcs,
            })?;
        }
        if let Some(m) = self.movement {
            add(Packet::Movement(m))?;
        }
        if let Some(v) = self.character.life {
            add(Packet::Life(v))?;
        }
        if let Some(v) = self.character.mana {
            add(Packet::Mana(v))?;
        }
        add(Packet::Buffs {
            player: slot,
            types: self.character.buffs.clone(),
        })?;
        if let Some((index, hidden_accessories)) = self.character.loadout {
            add(Packet::Loadout {
                player: slot,
                index,
                hidden_accessories,
            })?;
        }
        for index in (0..99).chain(900..990) {
            let equipment = self
                .character
                .equipment
                .get(&index)
                .copied()
                .unwrap_or(Equipment {
                    player: slot,
                    slot: index,
                    stack: 0,
                    prefix: 0,
                    item: 0,
                    flags: 0,
                });
            add(Packet::Equipment(equipment))?;
        }
        Ok(bytes.freeze())
    }
}

#[derive(Default)]
pub(super) struct Roster(BTreeMap<u8, Player>);
impl Roster {
    fn send(&mut self, except: Option<u8>, bytes: Bytes) {
        // Closing a lagging receiver also terminates its socket. Leave is queued before slot reuse.
        for (&slot, p) in &mut self.0 {
            if Some(slot) != except
                && p.out
                    .as_ref()
                    .is_some_and(|out| out.try_send(bytes.clone()).is_err())
            {
                p.out = None;
            }
        }
    }
    pub async fn run(mut self, mut input: mpsc::Receiver<Event>) {
        while let Some(event) = input.recv().await {
            if let Err(error) = self.handle(event) {
                tracing::debug!(%error,"could not serialize player update");
            }
        }
    }
    fn handle(&mut self, event: Event) -> Result<(), ProtocolError> {
        match event {
            Event::Join {
                slot,
                character,
                out,
            } => {
                let player = Player {
                    character,
                    movement: None,
                    zones: None,
                    out: Some(out.clone()),
                };
                let mut roster = BytesMut::new();
                for (&id, peer) in &self.0 {
                    roster.extend_from_slice(&peer.snapshot(id)?);
                }
                roster.extend_from_slice(&Packet::FinishedConnecting.encode()?);
                if out.try_send(roster.freeze()).is_ok() {
                    self.send(None, player.snapshot(slot)?);
                    if let Some(spawn) = player.character.spawn {
                        self.send(None, Packet::Spawn(spawn).encode()?);
                    }
                    self.0.insert(slot, player);
                }
            }
            Event::Leave(slot) => {
                if self.0.remove(&slot).is_some() {
                    self.send(
                        None,
                        Packet::PlayerActive {
                            player: slot,
                            active: false,
                        }
                        .encode()?,
                    );
                }
            }
            Event::Update { slot, packet } => {
                let Some(peer) = self.0.get_mut(&slot) else {
                    return Ok(());
                };
                match packet {
                    Packet::Ping => {
                        if let Some(out) = &peer.out
                            && out.try_send(Packet::Ping.encode()?).is_err()
                        {
                            peer.out = None;
                        }
                    }
                    Packet::ChatCommand { text, .. } => {
                        let flags = peer
                            .character
                            .appearance
                            .as_ref()
                            .map_or(0, |a| a.difficulty_flags);
                        let color = if flags & 8 != 0 {
                            [255; 3]
                        } else if flags & 2 != 0 {
                            [255, 38, 25]
                        } else if flags & 1 != 0 {
                            [255, 153, 0]
                        } else {
                            [255; 3]
                        };
                        self.send(
                            None,
                            Packet::ChatMessage {
                                author: slot,
                                text: NetworkText::Literal(text),
                                color,
                            }
                            .encode()?,
                        );
                    }
                    p => {
                        match &p {
                            Packet::PlayerZones {
                                zones, town_npcs, ..
                            } => peer.zones = Some((*zones, *town_npcs)),
                            Packet::Movement(m) => peer.movement = Some(*m),
                            Packet::Life(v) => peer.character.life = Some(*v),
                            Packet::Mana(v) => peer.character.mana = Some(*v),
                            Packet::Buffs { types, .. } => peer.character.buffs.clone_from(types),
                            Packet::Equipment(e) => {
                                if e.stack == 0 || e.item == 0 {
                                    peer.character.equipment.remove(&e.slot);
                                } else {
                                    peer.character.equipment.insert(e.slot, *e);
                                }
                                if (99..700).contains(&e.slot) {
                                    return Ok(());
                                }
                            }
                            Packet::Loadout {
                                index,
                                hidden_accessories,
                                ..
                            } => peer.character.loadout = Some((*index, *hidden_accessories)),
                            _ => return Err(ProtocolError::Invalid("active update")),
                        }
                        self.send(Some(slot), p.encode()?);
                    }
                }
            }
        }
        Ok(())
    }
}

fn update(frame: &terraria_protocol::Frame, slot: u8) -> Result<Packet, ProtocolError> {
    if !matches!(frame.id, 5 | 13 | 16 | 36 | 42 | 50 | 82 | 147 | 154) {
        return Err(ProtocolError::Unknown(frame.id));
    }
    let mut packet = Packet::decode_client(frame)?;
    match &mut packet {
        Packet::PlayerZones { player, .. } => *player = slot,
        Packet::Ping => {}
        Packet::Movement(m) => m.player = slot,
        Packet::Life(v) => {
            v.player = slot;
            v.maximum = v.maximum.max(20);
        }
        Packet::Mana(v) => v.player = slot,
        Packet::Buffs { player, .. } => *player = slot,
        Packet::Equipment(e) if e.valid() => e.player = slot,
        Packet::Loadout { player, index, .. } if *index < 3 => *player = slot,
        Packet::ChatCommand { command, text } if command == "Say" && text.len() <= 65520 => {}
        _ => return Err(ProtocolError::Invalid("active player update")),
    }
    Ok(packet)
}

pub(super) async fn play(
    stream: &mut TcpStream,
    slot: u8,
    character: Character,
    events: &mpsc::Sender<Event>,
    world: &std::sync::Arc<LoadedWorld>,
    mut sent: std::collections::BTreeSet<[u16; 2]>,
) -> Result<(), Error> {
    let (sender, mut pending) = mpsc::channel::<Bytes>(64);
    events
        .send(Event::Join {
            slot,
            character: Box::new(character),
            out: sender,
        })
        .await
        .map_err(|_| io::Error::other("roster closed"))?;
    let (section_tx, mut section_rx) = mpsc::channel(4);
    let (mut r, mut w) = stream.split();
    let read = async {
        let mut reported_ids = [false; 163];
        let mut reported_modules = std::collections::BTreeSet::new();
        while let Some(frame) = read_frame(&mut r).await? {
            if frame.id == 82
                && let Some(header) = frame.payload.first_chunk::<2>()
            {
                let module = u16::from_le_bytes(*header);
                if module != 1 {
                    if reported_modules.len() < 32 && reported_modules.insert(module) {
                        tracing::info!(slot, module, "ignoring unimplemented active net module");
                    }
                    continue;
                }
            }
            if frame.id == 159
                && let Packet::RequestSection(index) = Packet::decode_client(&frame)?
            {
                section_tx
                    .send(index)
                    .await
                    .map_err(|_| io::Error::other("terrain writer closed"))?;
                continue;
            }
            let packet = match update(&frame, slot) {
                Ok(p) => p,
                Err(ProtocolError::Unknown(id)) if (1..=162).contains(&id) => {
                    if !std::mem::replace(&mut reported_ids[usize::from(id)], true) {
                        tracing::info!(
                            slot,
                            packet_id = id,
                            "unimplemented active packet discarded"
                        );
                    }
                    continue;
                }
                Err(e) => {
                    tracing::warn!(slot,packet_id=frame.id,error=%e,"active packet rejected");
                    return Err(e.into());
                }
            };
            events
                .send(Event::Update { slot, packet })
                .await
                .map_err(|_| io::Error::other("roster closed"))?;
        }
        Ok::<_, Error>(())
    };
    let write = async {
        loop {
            tokio::select! {
                next=pending.recv()=>{
                    let Some(bytes)=next else{break};
                    timeout(Duration::from_secs(10),w.write_all(&bytes)).await.map_err(io::Error::other)??;
                }
                next=section_rx.recv()=>{
                    let Some(index)=next else{break};
                    if world.contains(index) && sent.insert(index){
                        timeout(Duration::from_secs(10),world.send_section(&mut w,index)).await.map_err(io::Error::other)??;
                    }
                }
            }
        }
        Ok(())
    };
    let result = tokio::select! {r=read=>r,r=write=>r};
    if matches!(result, Err(Error::Protocol(_))) {
        let _ = timeout(
            Duration::from_secs(1),
            write_packet(
                stream,
                &Packet::Kick(NetworkText::key("LegacyMultiplayer.2")),
            ),
        )
        .await;
    }
    result
}
