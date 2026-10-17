pub mod chat;
pub mod chunk;
pub mod disconnect;
pub mod interaction;
pub mod keepalive;
pub mod login;
pub mod movement;
pub mod sign;
pub mod spawn;
pub mod window;

pub use chat::Chat;
pub use chunk::{MapChunk, PreChunk};
pub use disconnect::KickDisconnect;
pub use interaction::{
    ArmAnimation, BlockChange, BlockDig, BlockItemSwitch, EntityAction, Place, Respawn, UseEntity,
};
pub use keepalive::KeepAlive;
pub use login::{Handshake, Login};
pub use movement::{Flying, PlayerLook, PlayerLookMove, PlayerPosition};
pub use sign::UpdateSign;
pub use spawn::{SpawnPosition, UpdateTime};
pub use window::{CloseWindow, Transaction, WindowClick};

use crate::net::protocol::PacketId;
use crate::util::bytes::{Reader, Writer};
use crate::util::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Serverbound {
    KeepAlive(KeepAlive),
    Login(Login),
    Handshake(Handshake),
    Chat(Chat),
    UseEntity(UseEntity),
    Respawn(Respawn),
    Flying(Flying),
    PlayerPosition(PlayerPosition),
    PlayerLook(PlayerLook),
    PlayerLookMove(PlayerLookMove),
    BlockDig(BlockDig),
    Place(Place),
    BlockItemSwitch(BlockItemSwitch),
    ArmAnimation(ArmAnimation),
    EntityAction(EntityAction),
    CloseWindow(CloseWindow),
    WindowClick(WindowClick),
    Transaction(Transaction),
    UpdateSign(UpdateSign),
    KickDisconnect(KickDisconnect),
}

impl Serverbound {
    pub fn decode(id: PacketId, reader: &mut Reader<'_>) -> Result<Self> {
        match id {
            PacketId::KeepAlive => Ok(Self::KeepAlive(KeepAlive::decode(reader)?)),
            PacketId::Login => Ok(Self::Login(Login::decode(reader)?)),
            PacketId::Handshake => Ok(Self::Handshake(Handshake::decode(reader)?)),
            PacketId::Chat => Ok(Self::Chat(Chat::decode(reader)?)),
            PacketId::UseEntity => Ok(Self::UseEntity(UseEntity::decode(reader)?)),
            PacketId::Respawn => Ok(Self::Respawn(Respawn::decode(reader)?)),
            PacketId::Flying => Ok(Self::Flying(Flying::decode(reader)?)),
            PacketId::PlayerPosition => Ok(Self::PlayerPosition(PlayerPosition::decode(reader)?)),
            PacketId::PlayerLook => Ok(Self::PlayerLook(PlayerLook::decode(reader)?)),
            PacketId::PlayerLookMove => Ok(Self::PlayerLookMove(PlayerLookMove::decode(reader)?)),
            PacketId::BlockDig => Ok(Self::BlockDig(BlockDig::decode(reader)?)),
            PacketId::Place => Ok(Self::Place(Place::decode(reader)?)),
            PacketId::BlockItemSwitch => {
                Ok(Self::BlockItemSwitch(BlockItemSwitch::decode(reader)?))
            }
            PacketId::ArmAnimation => Ok(Self::ArmAnimation(ArmAnimation::decode(reader)?)),
            PacketId::EntityAction => Ok(Self::EntityAction(EntityAction::decode(reader)?)),
            PacketId::CloseWindow => Ok(Self::CloseWindow(CloseWindow::decode(reader)?)),
            PacketId::WindowClick => Ok(Self::WindowClick(WindowClick::decode(reader)?)),
            PacketId::Transaction => Ok(Self::Transaction(Transaction::decode(reader)?)),
            PacketId::UpdateSign => Ok(Self::UpdateSign(UpdateSign::decode(reader)?)),
            PacketId::KickDisconnect => Ok(Self::KickDisconnect(KickDisconnect::decode(reader)?)),
            other => Err(Error::UnknownPacketId(other.id())),
        }
    }

    pub fn id(&self) -> PacketId {
        match self {
            Self::KeepAlive(_) => PacketId::KeepAlive,
            Self::Login(_) => PacketId::Login,
            Self::Handshake(_) => PacketId::Handshake,
            Self::Chat(_) => PacketId::Chat,
            Self::UseEntity(_) => PacketId::UseEntity,
            Self::Respawn(_) => PacketId::Respawn,
            Self::Flying(_) => PacketId::Flying,
            Self::PlayerPosition(_) => PacketId::PlayerPosition,
            Self::PlayerLook(_) => PacketId::PlayerLook,
            Self::PlayerLookMove(_) => PacketId::PlayerLookMove,
            Self::BlockDig(_) => PacketId::BlockDig,
            Self::Place(_) => PacketId::Place,
            Self::BlockItemSwitch(_) => PacketId::BlockItemSwitch,
            Self::ArmAnimation(_) => PacketId::ArmAnimation,
            Self::EntityAction(_) => PacketId::EntityAction,
            Self::CloseWindow(_) => PacketId::CloseWindow,
            Self::WindowClick(_) => PacketId::WindowClick,
            Self::Transaction(_) => PacketId::Transaction,
            Self::UpdateSign(_) => PacketId::UpdateSign,
            Self::KickDisconnect(_) => PacketId::KickDisconnect,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Clientbound {
    KeepAlive(KeepAlive),
    Login(Login),
    Handshake(Handshake),
    Chat(Chat),
    KickDisconnect(KickDisconnect),
    SpawnPosition(SpawnPosition),
    UpdateTime(UpdateTime),
    Respawn(Respawn),
    ArmAnimation(ArmAnimation),
    PreChunk(PreChunk),
    MapChunk(MapChunk),
    BlockChange(BlockChange),
    PlayerLookMove(PlayerLookMove),
    CloseWindow(CloseWindow),
    Transaction(Transaction),
    UpdateSign(UpdateSign),
}

impl Clientbound {
    pub fn id(&self) -> PacketId {
        match self {
            Self::KeepAlive(_) => PacketId::KeepAlive,
            Self::Login(_) => PacketId::Login,
            Self::Handshake(_) => PacketId::Handshake,
            Self::Chat(_) => PacketId::Chat,
            Self::KickDisconnect(_) => PacketId::KickDisconnect,
            Self::SpawnPosition(_) => PacketId::SpawnPosition,
            Self::UpdateTime(_) => PacketId::UpdateTime,
            Self::Respawn(_) => PacketId::Respawn,
            Self::ArmAnimation(_) => PacketId::ArmAnimation,
            Self::PreChunk(_) => PacketId::PreChunk,
            Self::MapChunk(_) => PacketId::MapChunk,
            Self::BlockChange(_) => PacketId::BlockChange,
            Self::PlayerLookMove(_) => PacketId::PlayerLookMove,
            Self::CloseWindow(_) => PacketId::CloseWindow,
            Self::Transaction(_) => PacketId::Transaction,
            Self::UpdateSign(_) => PacketId::UpdateSign,
        }
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.id().id());
        match self {
            Self::KeepAlive(packet) => packet.encode(writer),
            Self::Login(packet) => packet.encode(writer),
            Self::Handshake(packet) => packet.encode(writer),
            Self::Chat(packet) => packet.encode(writer),
            Self::KickDisconnect(packet) => packet.encode(writer),
            Self::SpawnPosition(packet) => packet.encode(writer),
            Self::UpdateTime(packet) => packet.encode(writer),
            Self::Respawn(packet) => packet.encode(writer),
            Self::ArmAnimation(packet) => packet.encode(writer),
            Self::PreChunk(packet) => packet.encode(writer),
            Self::MapChunk(packet) => packet.encode(writer),
            Self::BlockChange(packet) => packet.encode(writer),
            Self::PlayerLookMove(packet) => packet.encode(writer),
            Self::CloseWindow(packet) => packet.encode(writer),
            Self::Transaction(packet) => packet.encode(writer),
            Self::UpdateSign(packet) => packet.encode(writer),
        }
    }
}
