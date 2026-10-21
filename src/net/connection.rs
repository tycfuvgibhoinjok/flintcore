use std::collections::VecDeque;
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
    outbound: mpsc::UnboundedReceiver<Outbound>,
    config: Arc<Config>,
) {
    let _ = stream.set_nodelay(config.network.tcp_nodelay);
    let (read_half, write_half) = stream.into_split();
    let faster = config.network.faster_packets;
    // The channel of the kick: the packet of the disconnection is sent
    // before the connection is closed (`NetworkManager.b()`).
    let (kick_tx, kick_rx) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write_loop(write_half, outbound, kick_rx, faster));
    if let Err(error) = read_loop(id, read_half, &inbound, &config, &kick_tx).await {
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
    kicks: &mpsc::UnboundedSender<crate::net::packets::Clientbound>,
) -> Result<()> {
    let mut state = ConnectionState::Handshaking;
    let mut buffer: Vec<u8> = Vec::with_capacity(8192);
    let mut consumed = 0usize;
    let mut chunk = vec![0u8; 4096];
    let mut packets_in_window = 0usize;
    let mut window_start = std::time::Instant::now();
    let mut legacy_ping_sent = false;

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
            // Pre-1.7 server-list ping: a lone `0xFE` request. It is not a
            // framed packet, so it must be handled before the codec. The
            // simulation thread owns the MOTD/player counts and answers via
            // `NetEvent::LegacyPing`.
            if state == ConnectionState::Handshaking
                && !legacy_ping_sent
                && buffer.get(consumed) == Some(&0xFE)
            {
                consumed += 1;
                legacy_ping_sent = true;
                if inbound.send(NetEvent::LegacyPing { id }).await.is_err() {
                    return Ok(());
                }
                continue;
            }
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
                    if config.network.packet_spam_detection {
                        if window_start.elapsed().as_secs() >= 5 {
                            window_start = std::time::Instant::now();
                            packets_in_window = 0;
                        }
                        packets_in_window += 1;
                        if packets_in_window > config.network.packet_spam_threshold {
                            // Java `NetworkManager.b()`: the player is
                            // disconnected with a message.
                            let _ = kicks.send(crate::net::packets::Clientbound::KickDisconnect(
                                crate::net::packets::KickDisconnect {
                                    reason: "\u{00A7}c[Poseidon] You have been kicked for packet spamming."
                                        .to_string(),
                                },
                            ));
                            return Err(Error::Network("Packet spam detected".to_string()));
                        }
                    }
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

async fn write_loop(
    mut write: OwnedWriteHalf,
    mut outbound: mpsc::UnboundedReceiver<Outbound>,
    mut kicks: mpsc::UnboundedReceiver<crate::net::packets::Clientbound>,
    faster_packets: bool,
) {
    let mut encoder = Writer::with_capacity(65536);
    // The packets of a low priority wait until the queue of the packets of
    // the normal priority is empty (`NetworkManager.f()`).
    let mut deferred: VecDeque<crate::net::packets::Clientbound> = VecDeque::new();
    // A burst of packets (the login): the packets of a low priority wait
    // until the end of the burst.
    let mut burst = false;
    loop {
        // The kick of the connection: the packet is sent and the
        // connection is closed.
        if let Ok(packet) = kicks.try_recv() {
            encoder.clear();
            if packet.encode(&mut encoder).is_ok() {
                let _ = write.write_all(encoder.as_slice()).await;
                let _ = write.flush().await;
            }
            break;
        }
        encoder.clear();
        let mut should_close = false;
        // The packets of this write: the ones of a low priority are held
        // back until the queue of the other packets is empty.
        loop {
            match outbound.try_recv() {
                Ok(Outbound::Packet(packet)) => {
                    if packet.is_low_priority() {
                        deferred.push_back(packet);
                    } else if packet.encode(&mut encoder).is_err() {
                        should_close = true;
                        break;
                    }
                }
                Ok(Outbound::BurstStart) => {
                    burst = true;
                }
                Ok(Outbound::BurstEnd) => {
                    // The burst is over: the packets of a low priority go.
                    burst = false;
                    while let Some(packet) = deferred.pop_front() {
                        if packet.encode(&mut encoder).is_err() {
                            should_close = true;
                            break;
                        }
                    }
                    if should_close {
                        break;
                    }
                }
                Ok(Outbound::Close) => {
                    should_close = true;
                    break;
                }
                Err(_) => {
                    // The queue is empty: the packets of a low priority may
                    // go — but the packets of the login are sent one by one,
                    // so the queue can be empty in the middle of them.  Wait
                    // a moment for the next packet of the normal priority.
                    if !deferred.is_empty() && !burst {
                        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                        match outbound.try_recv() {
                            Ok(Outbound::Packet(packet)) => {
                                if packet.is_low_priority() {
                                    deferred.push_back(packet);
                                } else if packet.encode(&mut encoder).is_err() {
                                    should_close = true;
                                    break;
                                }
                            }
                            Ok(Outbound::BurstStart) => {
                                burst = true;
                            }
                            Ok(Outbound::BurstEnd) => {
                                burst = false;
                                while let Some(packet) = deferred.pop_front() {
                                    if packet.encode(&mut encoder).is_err() {
                                        should_close = true;
                                        break;
                                    }
                                }
                                if should_close {
                                    break;
                                }
                            }
                            Ok(Outbound::Close) => {
                                should_close = true;
                                break;
                            }
                            Err(_) => {
                                while let Some(packet) = deferred.pop_front() {
                                    if packet.encode(&mut encoder).is_err() {
                                        should_close = true;
                                        break;
                                    }
                                }
                                if should_close {
                                    break;
                                }
                            }
                        }
                    }
                    if !should_close && encoder.as_slice().is_empty() {
                        // Nothing to send: wait for the next packet.
                        match outbound.recv().await {
                            Some(Outbound::Packet(packet)) => {
                                if packet.is_low_priority() {
                                    deferred.push_back(packet);
                                } else if packet.encode(&mut encoder).is_err() {
                                    should_close = true;
                                }
                            }
                            Some(Outbound::BurstStart) => {
                                burst = true;
                            }
                            Some(Outbound::BurstEnd) => {
                                burst = false;
                                while let Some(packet) = deferred.pop_front() {
                                    if packet.encode(&mut encoder).is_err() {
                                        should_close = true;
                                        break;
                                    }
                                }
                            }
                            Some(Outbound::Close) => {
                                should_close = true;
                            }
                            None => {
                                should_close = true;
                            }
                        }
                    }
                    break;
                }
            }
            if !faster_packets || encoder.as_slice().len() >= 262_144 {
                break;
            }
        }
        if !encoder.as_slice().is_empty() {
            if write.write_all(encoder.as_slice()).await.is_err() {
                break;
            }
            if write.flush().await.is_err() {
                break;
            }
        }
        if should_close {
            let _ = write.flush().await;
            break;
        }
    }
    let _ = write.shutdown().await;
}
