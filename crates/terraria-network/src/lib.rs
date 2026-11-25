//! Bounded TCP transport and connection state for direct-IP clients.
mod drops;
mod players;
use players::{Event, Roster};
use tokio::sync::mpsc;
mod session;
mod terrain;
pub use session::{Character, Phase, Session};
use terrain::LoadedWorld;

use bytes::Bytes;
use std::{
    collections::BTreeSet,
    future::Future,
    io,
    num::NonZeroU8,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use terraria_protocol::{
    Error as ProtocolError, Frame, NetworkText, Packet, ServerInfo, frame_len,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time::timeout,
};
use tracing::debug;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    World(#[from] terraria_world::Error),
    #[error("connection task failed")]
    Task(#[from] tokio::task::JoinError),
}

/// A clean EOF is valid only at a frame boundary.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Frame>, Error> {
    let mut header = [0; 2];
    if r.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    r.read_exact(&mut header[1..]).await?;
    let size = frame_len(header)?;
    let id = r.read_u8().await?;
    let mut payload = vec![0; size - 3];
    r.read_exact(&mut payload).await?;
    Ok(Some(Frame {
        id,
        payload: Bytes::from(payload),
    }))
}

pub async fn write_packet<W: AsyncWrite + Unpin>(w: &mut W, packet: &Packet) -> Result<(), Error> {
    w.write_all(&packet.encode()?).await?;
    Ok(())
}

async fn connection(
    mut stream: TcpStream,
    slot: u8,
    password: &str,
    world: Option<&Arc<LoadedWorld>>,
    events: &mpsc::Sender<Event>,
    info: Option<ServerInfo>,
) -> Result<(), Error> {
    stream.set_nodelay(true)?;
    let mut session = world.map_or_else(
        || Session::new(slot, false),
        |w| Session::for_world(slot, &w.data),
    );
    let mut sent = BTreeSet::new();
    let mut revision = 0;
    let join = async {
        if let Some(info) = info {
            write_packet(&mut stream, &Packet::ServerInfo(info)).await?;
        }
        loop {
            let Some(frame) = read_frame(&mut stream).await? else {
                break;
            };
            if !matches!(
                frame.id,
                1 | 4 | 5 | 6 | 8 | 12 | 16 | 38 | 42 | 50 | 68 | 147 | 161 | 164
            ) {
                tracing::warn!(slot, packet_id=frame.id, phase=?session.phase(), "unexpected join packet");
                write_packet(
                    &mut stream,
                    &Packet::Kick(NetworkText::key("LegacyMultiplayer.2")),
                )
                .await?;
                break;
            }
            let packet = match Packet::decode(&frame) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(slot, packet_id=frame.id, phase=?session.phase(), error=%e, "invalid join payload");
                    // Complete but unsupported or malformed payload. The framing is still intact.
                    write_packet(
                        &mut stream,
                        &Packet::Kick(NetworkText::key("LegacyMultiplayer.2")),
                    )
                    .await?;
                    return Err(e.into());
                }
            };
            let request = match packet {
                Packet::RequestSections { spawn, team } => Some((spawn, team)),
                _ => None,
            };
            let phase = *session.phase();
            if let Some(response) = session.handle(packet, password) {
                if session.phase() == &Phase::Closed {
                    tracing::warn!(slot, packet_id = frame.id, ?phase, "join rejected");
                }
                if session.phase() == &Phase::Active {
                    break;
                }
                write_packet(&mut stream, &response).await?;
            }
            if let Some((spawn, team)) = request
                && session.phase() == &Phase::SendingSections
                && let Some(world) = world
            {
                revision = world.revision();
                sent = world.transfer(&mut stream, spawn, team).await?;
                session.phase = Phase::AwaitSpawn;
            }
            if session.phase() == &Phase::Closed {
                break;
            }
        }
        Ok::<_, Error>(())
    };
    timeout(Duration::from_secs(90), join)
        .await
        .map_err(io::Error::other)??;
    if session.phase() == &Phase::Active
        && let Some(world) = world
    {
        players::play(
            &mut stream,
            slot,
            session.into_character(),
            events,
            world,
            sent,
            revision,
        )
        .await?;
    }
    stream.shutdown().await?;
    Ok(())
}

/// The listener owns the free slots. Child tasks own one socket and its character upload.
pub async fn serve(
    listener: TcpListener,
    max_players: NonZeroU8,
    password: String,
    world: Option<terraria_world::World>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    serve_with_save(listener, max_players, password, world, None, shutdown).await
}

/// Publish gameplay edits on orderly shutdown when the listener owns a disk-backed world.
pub async fn serve_with_save(
    listener: TcpListener,
    max_players: NonZeroU8,
    password: String,
    world: Option<terraria_world::World>,
    save_path: Option<std::path::PathBuf>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    let world_info = world.map(LoadedWorld::new).transpose()?.map(Arc::new);
    let mut free: BTreeSet<u8> = (0..max_players.get()).collect();
    let password: Arc<str> = password.into();
    let mut tasks = JoinSet::new();
    let (events, input) = mpsc::channel(256);
    let player_count = Arc::new(AtomicU8::new(0));
    let roster = tokio::spawn(Roster::new(world_info.clone(), player_count.clone()).run(input));
    let local_addr = listener.local_addr()?;
    let server_info = world_info.as_ref().map(|w| ServerInfo {
        port: i32::from(local_addr.port()),
        world: w.data.name.clone(),
        host: local_addr.ip().to_string(),
        width: w.data.size[0] as u16,
        crimson: w.data.progression[1] & 0x20 != 0,
        difficulty: i32::from(w.data.game_mode),
        capacity: max_players.get(),
        players: 0,
        note: String::new(),
        mapped_address: None,
    });
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            Some(done) = tasks.join_next(), if !tasks.is_empty() => { free.insert(done?); }
            accepted = listener.accept() => {
                let (mut socket, peer) = accepted?;
                let Some(slot) = free.pop_first() else {
                    let kick = Packet::Kick(NetworkText::key("CLI.ServerIsFull"));
                    if !matches!(timeout(Duration::from_millis(250), write_packet(&mut socket, &kick)).await, Ok(Ok(()))) {
                        debug!(%peer, "could not deliver full-server response");
                    }
                    continue;
                };
                let password = Arc::clone(&password);
                let world_info=world_info.clone();
                let events=events.clone();
                let info = server_info.clone().map(|mut info| {
                    info.players = player_count.load(Ordering::Relaxed);
                    info
                });
                tasks.spawn(async move {
                    debug!(%peer, slot, "accepted client");
                    if let Err(e)=connection(socket,slot,&password,world_info.as_ref(),&events,info).await{
                        debug!(%peer,slot,error=%e,"connection closed");
                    }
                    let _=events.send(Event::Leave(slot)).await;
                    slot
                });
            }
        }
    }
    tasks.shutdown().await;
    drop(events);
    roster.await?;
    if let (Some(world), Some(path)) = (world_info, save_path) {
        tokio::task::spawn_blocking(move || world.save_changes(&path)).await??;
    }
    Ok(())
}
