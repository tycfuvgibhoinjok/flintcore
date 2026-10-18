use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::net::protocol::PacketId;
use crate::net::{codec, ConnId, NetEvent, Outbound};
use crate::util::bytes::Writer;
use crate::util::{log, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Handshaking,
    LoggingIn,
    Playing,
}

impl ConnectionState {
    pub fn accepts(self, id: PacketId) -> bool {
        match self {
            Self::Handshaking => matches!(id, PacketId::Handshake | PacketId::KeepAlive),
            Self::LoggingIn => matches!(id, PacketId::Login | PacketId::KeepAlive),
            Self::Playing => true,
        }
    }

    pub fn transition(self, id: PacketId) -> Self {
        match (self, id) {
            (Self::Handshaking, PacketId::Handshake) => Self::LoggingIn,
            (Self::LoggingIn, PacketId::Login) => Self::Playing,
            _ => self,
        }
    }
}

pub async fn serve(
    id: ConnId,
    stream: TcpStream,
    inbound: mpsc::Sender<NetEvent>,
    outbound: mpsc::Receiver<Outbound>,
    config: Arc<Config>,
) {
    let _ = stream.set_nodelay(config.network.tcp_nodelay);
    let (read_half, write_half) = stream.into_split();
    let writer = tokio::spawn(write_loop(write_half, outbound));
    if let Err(error) = read_loop(id, read_half, &inbound, &config).await {
        log::warn(&format!("connection {id} closed: {error}"));
    }
    let _ = inbound.send(NetEvent::Disconnected { id }).await;
    writer.abort();
}

async fn read_loop(
    id: ConnId,
    mut read: OwnedReadHalf,
    inbound: &mpsc::Sender<NetEvent>,
    config: &Config,
) -> Result<()> {
    let mut state = ConnectionState::Handshaking;
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    let mut consumed = 0usize;
    let mut chunk = vec![0u8; 4096];

    loop {
        if consumed > 0 {
            buffer.drain(..consumed);
            consumed = 0;
            if buffer.capacity() > 16384 && buffer.len() <= 4096 {
                buffer.shrink_to(8192);
            }
        }
        let read_count = read.read(&mut chunk).await?;
        if read_count == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read_count]);
        if buffer.len() > config.network.max_read_buffer {
            return Err(Error::Network("read buffer overflow".to_string()));
        }

        loop {
            match codec::decode_serverbound(&buffer[consumed..]) {
                Ok((packet, length)) => {
                    let id_now = packet.id();
                    if !state.accepts(id_now) {
                        return Err(Error::InvalidState(
                            "packet not allowed in current connection state",
                        ));
                    }
                    state = state.transition(id_now);
                    consumed += length;
                    if inbound.send(NetEvent::Packet { id, packet }).await.is_err() {
                        return Ok(());
                    }
                }
                Err(Error::Incomplete) => break,
                Err(error) => return Err(error),
            }
        }
    }
}

async fn write_loop(mut write: OwnedWriteHalf, mut outbound: mpsc::Receiver<Outbound>) {
    let mut encoder = Writer::with_capacity(4096);
    while let Some(message) = outbound.recv().await {
        match message {
            Outbound::Packet(packet) => {
                encoder.clear();
                if packet.encode(&mut encoder).is_err() {
                    break;
                }
                if write.write_all(encoder.as_slice()).await.is_err() {
                    break;
                }
                if write.flush().await.is_err() {
                    break;
                }
            }
            Outbound::Close => {
                let _ = write.flush().await;
                break;
            }
        }
    }
    let _ = write.shutdown().await;
}
