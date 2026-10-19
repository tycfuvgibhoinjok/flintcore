pub mod codec;
pub mod connection;
pub mod listener;
pub mod packets;
pub mod protocol;

use std::net::SocketAddr;

use tokio::sync::mpsc::UnboundedSender;

use crate::net::packets::Clientbound;

pub type ConnId = u64;

#[derive(Debug)]
pub enum Outbound {
    Packet(Clientbound),
    Close,
}

#[derive(Debug)]
pub enum NetEvent {
    Connected {
        id: ConnId,
        address: SocketAddr,
        sender: UnboundedSender<Outbound>,
    },
    Packet {
        id: ConnId,
        packet: packets::Serverbound,
    },
    /// Legacy `0xFE` server-list ping; the server answers with a raw `0xFF`
    /// packet containing the MOTD, online count and player limit.
    LegacyPing {
        id: ConnId,
    },
    Disconnected {
        id: ConnId,
    },
}
