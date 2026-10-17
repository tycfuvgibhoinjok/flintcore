pub mod codec;
pub mod connection;
pub mod listener;
pub mod packets;
pub mod protocol;

use std::net::SocketAddr;

use tokio::sync::mpsc::Sender;

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
        sender: Sender<Outbound>,
    },
    Packet {
        id: ConnId,
        packet: packets::Serverbound,
    },
    Disconnected {
        id: ConnId,
    },
}
