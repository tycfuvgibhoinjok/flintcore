pub const PROTOCOL_VERSION: i32 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketId {
    KeepAlive = 0,
    Login = 1,
    Handshake = 2,
    Chat = 3,
    UpdateTime = 4,
    EntityEquipment = 5,
    SpawnPosition = 6,
    UseEntity = 7,
    UpdateHealth = 8,
    Respawn = 9,
    Flying = 10,
    PlayerPosition = 11,
    PlayerLook = 12,
    PlayerLookMove = 13,
    BlockDig = 14,
    Place = 15,
    BlockItemSwitch = 16,
    Packet17 = 17,
    ArmAnimation = 18,
    EntityAction = 19,
    NamedEntitySpawn = 20,
    PickupSpawn = 21,
    Collect = 22,
    VehicleSpawn = 23,
    MobSpawn = 24,
    EntityPainting = 25,
    Packet27 = 27,
    EntityVelocity = 28,
    DestroyEntity = 29,
    Entity = 30,
    RelEntityMove = 31,
    EntityLook = 32,
    RelEntityMoveLook = 33,
    EntityTeleport = 34,
    EntityStatus = 38,
    AttachEntity = 39,
    EntityMetadata = 40,
    NamedSound = 41,
    PreChunk = 50,
    MapChunk = 51,
    MultiBlockChange = 52,
    BlockChange = 53,
    PlayNoteBlock = 54,
    Explosion = 60,
    Packet61 = 61,
    Bed = 70,
    Weather = 71,
    OpenWindow = 100,
    CloseWindow = 101,
    WindowClick = 102,
    SetSlot = 103,
    WindowItems = 104,
    CraftProgressBar = 105,
    Transaction = 106,
    UpdateSign = 130,
    Packet131 = 131,
    Statistic = 200,
    KickDisconnect = 255,
}

impl PacketId {
    pub const ALL: [PacketId; 57] = [
        PacketId::KeepAlive,
        PacketId::Login,
        PacketId::Handshake,
        PacketId::Chat,
        PacketId::UpdateTime,
        PacketId::EntityEquipment,
        PacketId::SpawnPosition,
        PacketId::UseEntity,
        PacketId::UpdateHealth,
        PacketId::Respawn,
        PacketId::Flying,
        PacketId::PlayerPosition,
        PacketId::PlayerLook,
        PacketId::PlayerLookMove,
        PacketId::BlockDig,
        PacketId::Place,
        PacketId::BlockItemSwitch,
        PacketId::Packet17,
        PacketId::ArmAnimation,
        PacketId::EntityAction,
        PacketId::NamedEntitySpawn,
        PacketId::PickupSpawn,
        PacketId::Collect,
        PacketId::VehicleSpawn,
        PacketId::MobSpawn,
        PacketId::EntityPainting,
        PacketId::Packet27,
        PacketId::EntityVelocity,
        PacketId::DestroyEntity,
        PacketId::Entity,
        PacketId::RelEntityMove,
        PacketId::EntityLook,
        PacketId::RelEntityMoveLook,
        PacketId::EntityTeleport,
        PacketId::EntityStatus,
        PacketId::AttachEntity,
        PacketId::EntityMetadata,
        PacketId::PreChunk,
        PacketId::MapChunk,
        PacketId::MultiBlockChange,
        PacketId::BlockChange,
        PacketId::PlayNoteBlock,
        PacketId::Explosion,
        PacketId::Packet61,
        PacketId::Bed,
        PacketId::Weather,
        PacketId::OpenWindow,
        PacketId::CloseWindow,
        PacketId::WindowClick,
        PacketId::SetSlot,
        PacketId::WindowItems,
        PacketId::CraftProgressBar,
        PacketId::Transaction,
        PacketId::UpdateSign,
        PacketId::Packet131,
        PacketId::Statistic,
        PacketId::KickDisconnect,
    ];

    pub fn from_u8(id: u8) -> Option<Self> {
        Some(match id {
            0 => PacketId::KeepAlive,
            1 => PacketId::Login,
            2 => PacketId::Handshake,
            3 => PacketId::Chat,
            4 => PacketId::UpdateTime,
            5 => PacketId::EntityEquipment,
            6 => PacketId::SpawnPosition,
            7 => PacketId::UseEntity,
            8 => PacketId::UpdateHealth,
            9 => PacketId::Respawn,
            10 => PacketId::Flying,
            11 => PacketId::PlayerPosition,
            12 => PacketId::PlayerLook,
            13 => PacketId::PlayerLookMove,
            14 => PacketId::BlockDig,
            15 => PacketId::Place,
            16 => PacketId::BlockItemSwitch,
            17 => PacketId::Packet17,
            18 => PacketId::ArmAnimation,
            19 => PacketId::EntityAction,
            20 => PacketId::NamedEntitySpawn,
            21 => PacketId::PickupSpawn,
            22 => PacketId::Collect,
            23 => PacketId::VehicleSpawn,
            24 => PacketId::MobSpawn,
            25 => PacketId::EntityPainting,
            27 => PacketId::Packet27,
            28 => PacketId::EntityVelocity,
            29 => PacketId::DestroyEntity,
            30 => PacketId::Entity,
            31 => PacketId::RelEntityMove,
            32 => PacketId::EntityLook,
            33 => PacketId::RelEntityMoveLook,
            34 => PacketId::EntityTeleport,
            38 => PacketId::EntityStatus,
            39 => PacketId::AttachEntity,
            40 => PacketId::EntityMetadata,
            50 => PacketId::PreChunk,
            51 => PacketId::MapChunk,
            52 => PacketId::MultiBlockChange,
            53 => PacketId::BlockChange,
            54 => PacketId::PlayNoteBlock,
            60 => PacketId::Explosion,
            61 => PacketId::Packet61,
            70 => PacketId::Bed,
            71 => PacketId::Weather,
            100 => PacketId::OpenWindow,
            101 => PacketId::CloseWindow,
            102 => PacketId::WindowClick,
            103 => PacketId::SetSlot,
            104 => PacketId::WindowItems,
            105 => PacketId::CraftProgressBar,
            106 => PacketId::Transaction,
            130 => PacketId::UpdateSign,
            131 => PacketId::Packet131,
            200 => PacketId::Statistic,
            255 => PacketId::KickDisconnect,
            _ => return None,
        })
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn clientbound(self) -> bool {
        matches!(
            self,
            PacketId::KeepAlive
                | PacketId::Login
                | PacketId::Handshake
                | PacketId::Chat
                | PacketId::UpdateTime
                | PacketId::EntityEquipment
                | PacketId::SpawnPosition
                | PacketId::UpdateHealth
                | PacketId::Respawn
                | PacketId::Flying
                | PacketId::PlayerPosition
                | PacketId::PlayerLook
                | PacketId::PlayerLookMove
                | PacketId::Packet17
                | PacketId::ArmAnimation
                | PacketId::NamedEntitySpawn
                | PacketId::PickupSpawn
                | PacketId::Collect
                | PacketId::VehicleSpawn
                | PacketId::MobSpawn
                | PacketId::EntityPainting
                | PacketId::EntityVelocity
                | PacketId::DestroyEntity
                | PacketId::Entity
                | PacketId::RelEntityMove
                | PacketId::EntityLook
                | PacketId::RelEntityMoveLook
                | PacketId::EntityTeleport
                | PacketId::EntityStatus
                | PacketId::AttachEntity
                | PacketId::EntityMetadata
                | PacketId::PreChunk
                | PacketId::MapChunk
                | PacketId::MultiBlockChange
                | PacketId::BlockChange
                | PacketId::PlayNoteBlock
                | PacketId::Explosion
                | PacketId::Packet61
                | PacketId::Bed
                | PacketId::Weather
                | PacketId::OpenWindow
                | PacketId::CloseWindow
                | PacketId::SetSlot
                | PacketId::WindowItems
                | PacketId::CraftProgressBar
                | PacketId::Transaction
                | PacketId::UpdateSign
                | PacketId::Packet131
                | PacketId::Statistic
                | PacketId::KickDisconnect
        )
    }

    pub fn serverbound(self) -> bool {
        matches!(
            self,
            PacketId::KeepAlive
                | PacketId::Login
                | PacketId::Handshake
                | PacketId::Chat
                | PacketId::UseEntity
                | PacketId::Respawn
                | PacketId::Flying
                | PacketId::PlayerPosition
                | PacketId::PlayerLook
                | PacketId::PlayerLookMove
                | PacketId::BlockDig
                | PacketId::Place
                | PacketId::BlockItemSwitch
                | PacketId::ArmAnimation
                | PacketId::EntityAction
                | PacketId::CloseWindow
                | PacketId::WindowClick
                | PacketId::Transaction
                | PacketId::UpdateSign
                | PacketId::KickDisconnect
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_id_table_is_complete() {
        assert_eq!(PacketId::ALL.len(), 57);
        for (index, packet) in PacketId::ALL.iter().enumerate() {
            assert_eq!(
                PacketId::from_u8(packet.id()),
                Some(*packet),
                "round trip failed at index {index}"
            );
            assert_eq!(packet.id() as usize, *packet as usize);
        }
    }

    #[test]
    fn unassigned_ids_are_rejected() {
        assert_eq!(PacketId::from_u8(26), None);
        assert_eq!(PacketId::from_u8(35), None);
        assert_eq!(PacketId::from_u8(49), None);
        assert_eq!(PacketId::from_u8(254), None);
    }

    #[test]
    fn directions_match_reference() {
        assert!(PacketId::Handshake.clientbound() && PacketId::Handshake.serverbound());
        assert!(PacketId::MapChunk.clientbound() && !PacketId::MapChunk.serverbound());
        assert!(PacketId::BlockDig.serverbound() && !PacketId::BlockDig.clientbound());
        assert!(!PacketId::Packet27.clientbound() && !PacketId::Packet27.serverbound());
    }
}
