pub mod chat;
pub mod chunk;
pub mod disconnect;
pub mod entity;
pub mod interaction;
pub mod keepalive;
pub mod login;
pub mod movement;
pub mod sign;
pub mod spawn;
pub mod window;
pub mod world;

pub use chat::Chat;
pub use chunk::{MapChunk, PreChunk};
pub use disconnect::KickDisconnect;
pub use entity::{
    AttachEntity, Collect, DestroyEntity, Entity, EntityEquipment, EntityLook, EntityMetadata,
    EntityPainting, EntityStatus, EntityTeleport, EntityVelocity, MobSpawn, NamedEntitySpawn,
    PickupSpawn, RelEntityMove, RelEntityMoveLook, UpdateHealth, VehicleSpawn,
};
pub use interaction::{
    ArmAnimation, BlockChange, BlockDig, BlockItemSwitch, EntityAction, Place, Respawn, UseEntity,
};
pub use keepalive::KeepAlive;
pub use login::{Handshake, Login};
pub use movement::{Flying, PlayerLook, PlayerLookMove, PlayerPosition};
pub use sign::UpdateSign;
pub use spawn::{SpawnPosition, UpdateTime};
pub use window::{
    CloseWindow, CraftProgressBar, OpenWindow, SetSlot, SlotItem, Transaction, WindowClick,
    WindowItems,
};
pub use world::{
    Bed, Explosion, LevelSound, MapData, MultiBlockChange, PlayNoteBlock, Sleep, Statistic, Weather,
};

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
    UpdateTime(UpdateTime),
    EntityEquipment(EntityEquipment),
    SpawnPosition(SpawnPosition),
    UpdateHealth(UpdateHealth),
    Respawn(Respawn),
    Sleep(Sleep),
    ArmAnimation(ArmAnimation),
    NamedEntitySpawn(NamedEntitySpawn),
    PickupSpawn(PickupSpawn),
    Collect(Collect),
    VehicleSpawn(VehicleSpawn),
    MobSpawn(MobSpawn),
    EntityPainting(EntityPainting),
    EntityVelocity(EntityVelocity),
    DestroyEntity(DestroyEntity),
    Entity(Entity),
    RelEntityMove(RelEntityMove),
    EntityLook(EntityLook),
    RelEntityMoveLook(RelEntityMoveLook),
    EntityTeleport(EntityTeleport),
    EntityStatus(EntityStatus),
    AttachEntity(AttachEntity),
    EntityMetadata(EntityMetadata),
    PreChunk(PreChunk),
    MapChunk(MapChunk),
    MultiBlockChange(MultiBlockChange),
    BlockChange(BlockChange),
    PlayNoteBlock(PlayNoteBlock),
    Explosion(Explosion),
    LevelSound(LevelSound),
    Bed(Bed),
    Weather(Weather),
    OpenWindow(OpenWindow),
    CloseWindow(CloseWindow),
    SetSlot(SetSlot),
    WindowItems(WindowItems),
    CraftProgressBar(CraftProgressBar),
    Transaction(Transaction),
    UpdateSign(UpdateSign),
    MapData(MapData),
    Statistic(Statistic),
    KickDisconnect(KickDisconnect),
    PlayerLookMove(PlayerLookMove),
}

impl Clientbound {
    pub fn id(&self) -> PacketId {
        match self {
            Self::KeepAlive(_) => PacketId::KeepAlive,
            Self::Login(_) => PacketId::Login,
            Self::Handshake(_) => PacketId::Handshake,
            Self::Chat(_) => PacketId::Chat,
            Self::UpdateTime(_) => PacketId::UpdateTime,
            Self::EntityEquipment(_) => PacketId::EntityEquipment,
            Self::SpawnPosition(_) => PacketId::SpawnPosition,
            Self::UpdateHealth(_) => PacketId::UpdateHealth,
            Self::Respawn(_) => PacketId::Respawn,
            Self::Sleep(_) => PacketId::Packet17,
            Self::ArmAnimation(_) => PacketId::ArmAnimation,
            Self::NamedEntitySpawn(_) => PacketId::NamedEntitySpawn,
            Self::PickupSpawn(_) => PacketId::PickupSpawn,
            Self::Collect(_) => PacketId::Collect,
            Self::VehicleSpawn(_) => PacketId::VehicleSpawn,
            Self::MobSpawn(_) => PacketId::MobSpawn,
            Self::EntityPainting(_) => PacketId::EntityPainting,
            Self::EntityVelocity(_) => PacketId::EntityVelocity,
            Self::DestroyEntity(_) => PacketId::DestroyEntity,
            Self::Entity(_) => PacketId::Entity,
            Self::RelEntityMove(_) => PacketId::RelEntityMove,
            Self::EntityLook(_) => PacketId::EntityLook,
            Self::RelEntityMoveLook(_) => PacketId::RelEntityMoveLook,
            Self::EntityTeleport(_) => PacketId::EntityTeleport,
            Self::EntityStatus(_) => PacketId::EntityStatus,
            Self::AttachEntity(_) => PacketId::AttachEntity,
            Self::EntityMetadata(_) => PacketId::EntityMetadata,
            Self::PreChunk(_) => PacketId::PreChunk,
            Self::MapChunk(_) => PacketId::MapChunk,
            Self::MultiBlockChange(_) => PacketId::MultiBlockChange,
            Self::BlockChange(_) => PacketId::BlockChange,
            Self::PlayNoteBlock(_) => PacketId::PlayNoteBlock,
            Self::Explosion(_) => PacketId::Explosion,
            Self::LevelSound(_) => PacketId::Packet61,
            Self::Bed(_) => PacketId::Bed,
            Self::Weather(_) => PacketId::Weather,
            Self::OpenWindow(_) => PacketId::OpenWindow,
            Self::CloseWindow(_) => PacketId::CloseWindow,
            Self::SetSlot(_) => PacketId::SetSlot,
            Self::WindowItems(_) => PacketId::WindowItems,
            Self::CraftProgressBar(_) => PacketId::CraftProgressBar,
            Self::Transaction(_) => PacketId::Transaction,
            Self::UpdateSign(_) => PacketId::UpdateSign,
            Self::MapData(_) => PacketId::Packet131,
            Self::Statistic(_) => PacketId::Statistic,
            Self::KickDisconnect(_) => PacketId::KickDisconnect,
            Self::PlayerLookMove(_) => PacketId::PlayerLookMove,
        }
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_u8(self.id().id());
        match self {
            Self::KeepAlive(p) => p.encode(writer),
            Self::Login(p) => p.encode(writer),
            Self::Handshake(p) => p.encode(writer),
            Self::Chat(p) => p.encode(writer),
            Self::UpdateTime(p) => p.encode(writer),
            Self::EntityEquipment(p) => p.encode(writer),
            Self::SpawnPosition(p) => p.encode(writer),
            Self::UpdateHealth(p) => p.encode(writer),
            Self::Respawn(p) => p.encode(writer),
            Self::Sleep(p) => p.encode(writer),
            Self::ArmAnimation(p) => p.encode(writer),
            Self::NamedEntitySpawn(p) => p.encode(writer),
            Self::PickupSpawn(p) => p.encode(writer),
            Self::Collect(p) => p.encode(writer),
            Self::VehicleSpawn(p) => p.encode(writer),
            Self::MobSpawn(p) => p.encode(writer),
            Self::EntityPainting(p) => p.encode(writer),
            Self::EntityVelocity(p) => p.encode(writer),
            Self::DestroyEntity(p) => p.encode(writer),
            Self::Entity(p) => p.encode(writer),
            Self::RelEntityMove(p) => p.encode(writer),
            Self::EntityLook(p) => p.encode(writer),
            Self::RelEntityMoveLook(p) => p.encode(writer),
            Self::EntityTeleport(p) => p.encode(writer),
            Self::EntityStatus(p) => p.encode(writer),
            Self::AttachEntity(p) => p.encode(writer),
            Self::EntityMetadata(p) => p.encode(writer),
            Self::PreChunk(p) => p.encode(writer),
            Self::MapChunk(p) => p.encode(writer),
            Self::MultiBlockChange(p) => p.encode(writer),
            Self::BlockChange(p) => p.encode(writer),
            Self::PlayNoteBlock(p) => p.encode(writer),
            Self::Explosion(p) => p.encode(writer),
            Self::LevelSound(p) => p.encode(writer),
            Self::Bed(p) => p.encode(writer),
            Self::Weather(p) => p.encode(writer),
            Self::OpenWindow(p) => p.encode(writer),
            Self::CloseWindow(p) => p.encode(writer),
            Self::SetSlot(p) => p.encode(writer),
            Self::WindowItems(p) => p.encode(writer),
            Self::CraftProgressBar(p) => p.encode(writer),
            Self::Transaction(p) => p.encode(writer),
            Self::UpdateSign(p) => p.encode(writer),
            Self::MapData(p) => p.encode(writer),
            Self::Statistic(p) => p.encode(writer),
            Self::KickDisconnect(p) => p.encode(writer),
            Self::PlayerLookMove(p) => p.encode(writer),
        }
    }
}
