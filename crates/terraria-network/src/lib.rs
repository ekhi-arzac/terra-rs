//! Bounded TCP transport and connection state for direct-IP clients.
mod session;
pub use session::{Character, Phase, Session};

use bytes::Bytes;
use std::{collections::BTreeSet, future::Future, io, num::NonZeroU8, sync::Arc, time::Duration};
use terraria_protocol::{Error as ProtocolError, Frame, NetworkText, Packet, frame_len};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time::timeout,
};
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
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

async fn connection(mut stream: TcpStream, slot: u8, password: &str) -> Result<(), Error> {
    stream.set_nodelay(true)?;
    let mut session = Session::new(slot);
    while let Some(frame) = read_frame(&mut stream).await? {
        let packet = match Packet::decode(&frame) {
            Ok(p) => p,
            Err(e) => {
                // Complete but unsupported or malformed payload. The framing is still intact.
                write_packet(
                    &mut stream,
                    &Packet::Kick(NetworkText::key("LegacyMultiplayer.2")),
                )
                .await?;
                return Err(e.into());
            }
        };
        if let Some(response) = session.handle(packet, password) {
            write_packet(&mut stream, &response).await?;
        }
        if session.phase() == &Phase::Closed {
            break;
        }
    }
    stream.shutdown().await?;
    Ok(())
}

/// The listener owns the free slots. Child tasks own one socket and its character upload.
pub async fn serve(
    listener: TcpListener,
    max_players: NonZeroU8,
    password: String,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    let mut free: BTreeSet<u8> = (0..max_players.get()).collect();
    let password: Arc<str> = password.into();
    let mut tasks = JoinSet::new();
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
                tasks.spawn(async move {
                    debug!(%peer, slot, "accepted client");
                    match timeout(Duration::from_secs(90), connection(socket, slot, &password)).await {
                        Ok(Err(e)) => debug!(%peer, slot, error = %e, "connection closed"),
                        Err(_) => warn!(%peer, slot, "join deadline elapsed"),
                        Ok(Ok(())) => {}
                    }
                    slot
                });
            }
        }
    }
    tasks.shutdown().await;
    Ok(())
}
