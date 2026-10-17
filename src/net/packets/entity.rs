use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityEquipment {
    pub entity_id: i32,
    pub slot: i16,
    pub item_id: i16,
    pub damage: i16,
}

impl EntityEquipment {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            slot: reader.read_i16()?,
            item_id: reader.read_i16()?,
            damage: reader.read_i16()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i16(self.slot);
        writer.write_i16(self.item_id);
        writer.write_i16(self.damage);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateHealth {
    pub health: i16,
}

impl UpdateHealth {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            health: reader.read_i16()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i16(self.health);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedEntitySpawn {
    pub entity_id: i32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: u8,
    pub pitch: u8,
    pub current_item: i16,
}

impl NamedEntitySpawn {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            name: reader.read_string(16)?,
            x: reader.read_i32()?,
            y: reader.read_i32()?,
            z: reader.read_i32()?,
            yaw: reader.read_u8()?,
            pitch: reader.read_u8()?,
            current_item: reader.read_i16()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_string(&self.name, 16)?;
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.yaw);
        writer.write_u8(self.pitch);
        writer.write_i16(self.current_item);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupSpawn {
    pub entity_id: i32,
    pub item_id: i16,
    pub count: i8,
    pub damage: i16,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub mot_x: i8,
    pub mot_y: i8,
    pub mot_z: i8,
}

impl PickupSpawn {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            item_id: reader.read_i16()?,
            count: reader.read_i8()?,
            damage: reader.read_i16()?,
            x: reader.read_i32()?,
            y: reader.read_i32()?,
            z: reader.read_i32()?,
            mot_x: reader.read_i8()?,
            mot_y: reader.read_i8()?,
            mot_z: reader.read_i8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i16(self.item_id);
        writer.write_i8(self.count);
        writer.write_i16(self.damage);
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        writer.write_i8(self.mot_x);
        writer.write_i8(self.mot_y);
        writer.write_i8(self.mot_z);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collect {
    pub collected_id: i32,
    pub collector_id: i32,
}

impl Collect {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            collected_id: reader.read_i32()?,
            collector_id: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.collected_id);
        writer.write_i32(self.collector_id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleSpawn {
    pub entity_id: i32,
    pub vehicle_type: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub thrower_id: i32,
    pub velocity: Option<(i16, i16, i16)>,
}

impl VehicleSpawn {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let entity_id = reader.read_i32()?;
        let vehicle_type = reader.read_u8()?;
        let x = reader.read_i32()?;
        let y = reader.read_i32()?;
        let z = reader.read_i32()?;
        let thrower_id = reader.read_i32()?;
        let velocity = if thrower_id > 0 {
            Some((reader.read_i16()?, reader.read_i16()?, reader.read_i16()?))
        } else {
            None
        };
        Ok(Self {
            entity_id,
            vehicle_type,
            x,
            y,
            z,
            thrower_id,
            velocity,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.vehicle_type);
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        writer.write_i32(self.thrower_id);
        if self.thrower_id > 0 {
            if let Some((vx, vy, vz)) = self.velocity {
                writer.write_i16(vx);
                writer.write_i16(vy);
                writer.write_i16(vz);
            } else {
                writer.write_i16(0);
                writer.write_i16(0);
                writer.write_i16(0);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobSpawn {
    pub entity_id: i32,
    pub mob_type: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: u8,
    pub pitch: u8,
    pub metadata: Vec<u8>,
}

impl MobSpawn {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let entity_id = reader.read_i32()?;
        let mob_type = reader.read_u8()?;
        let x = reader.read_i32()?;
        let y = reader.read_i32()?;
        let z = reader.read_i32()?;
        let yaw = reader.read_u8()?;
        let pitch = reader.read_u8()?;
        let metadata = decode_raw_metadata(reader)?;
        Ok(Self {
            entity_id,
            mob_type,
            x,
            y,
            z,
            yaw,
            pitch,
            metadata,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.mob_type);
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.yaw);
        writer.write_u8(self.pitch);
        if self.metadata.is_empty() {
            writer.write_u8(127);
        } else {
            writer.write_bytes(&self.metadata);
            if *self.metadata.last().unwrap_or(&0) != 127 {
                writer.write_u8(127);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityPainting {
    pub entity_id: i32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub direction: i32,
}

impl EntityPainting {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            title: reader.read_string(13)?,
            x: reader.read_i32()?,
            y: reader.read_i32()?,
            z: reader.read_i32()?,
            direction: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_string(&self.title, 13)?;
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        writer.write_i32(self.direction);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityVelocity {
    pub entity_id: i32,
    pub mot_x: i16,
    pub mot_y: i16,
    pub mot_z: i16,
}

impl EntityVelocity {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            mot_x: reader.read_i16()?,
            mot_y: reader.read_i16()?,
            mot_z: reader.read_i16()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i16(self.mot_x);
        writer.write_i16(self.mot_y);
        writer.write_i16(self.mot_z);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestroyEntity {
    pub entity_id: i32,
}

impl DestroyEntity {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub entity_id: i32,
}

impl Entity {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelEntityMove {
    pub entity_id: i32,
    pub dx: i8,
    pub dy: i8,
    pub dz: i8,
}

impl RelEntityMove {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            dx: reader.read_i8()?,
            dy: reader.read_i8()?,
            dz: reader.read_i8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i8(self.dx);
        writer.write_i8(self.dy);
        writer.write_i8(self.dz);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityLook {
    pub entity_id: i32,
    pub yaw: u8,
    pub pitch: u8,
}

impl EntityLook {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            yaw: reader.read_u8()?,
            pitch: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.yaw);
        writer.write_u8(self.pitch);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelEntityMoveLook {
    pub entity_id: i32,
    pub dx: i8,
    pub dy: i8,
    pub dz: i8,
    pub yaw: u8,
    pub pitch: u8,
}

impl RelEntityMoveLook {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            dx: reader.read_i8()?,
            dy: reader.read_i8()?,
            dz: reader.read_i8()?,
            yaw: reader.read_u8()?,
            pitch: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i8(self.dx);
        writer.write_i8(self.dy);
        writer.write_i8(self.dz);
        writer.write_u8(self.yaw);
        writer.write_u8(self.pitch);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTeleport {
    pub entity_id: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: u8,
    pub pitch: u8,
}

impl EntityTeleport {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            x: reader.read_i32()?,
            y: reader.read_i32()?,
            z: reader.read_i32()?,
            yaw: reader.read_u8()?,
            pitch: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i32(self.x);
        writer.write_i32(self.y);
        writer.write_i32(self.z);
        writer.write_u8(self.yaw);
        writer.write_u8(self.pitch);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityStatus {
    pub entity_id: i32,
    pub status: u8,
}

impl EntityStatus {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            status: reader.read_u8()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_u8(self.status);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachEntity {
    pub entity_id: i32,
    pub vehicle_id: i32,
}

impl AttachEntity {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            entity_id: reader.read_i32()?,
            vehicle_id: reader.read_i32()?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        writer.write_i32(self.vehicle_id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMetadata {
    pub entity_id: i32,
    pub metadata: Vec<u8>,
}

impl EntityMetadata {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let entity_id = reader.read_i32()?;
        let metadata = decode_raw_metadata(reader)?;
        Ok(Self {
            entity_id,
            metadata,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_i32(self.entity_id);
        if self.metadata.is_empty() {
            writer.write_u8(127);
        } else {
            writer.write_bytes(&self.metadata);
            if *self.metadata.last().unwrap_or(&0) != 127 {
                writer.write_u8(127);
            }
        }
        Ok(())
    }
}

fn decode_raw_metadata(reader: &mut Reader<'_>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let header = reader.read_u8()?;
        bytes.push(header);
        if header == 127 {
            break;
        }
        let type_id = (header >> 5) & 7;
        match type_id {
            0 => {
                bytes.push(reader.read_u8()?);
            }
            1 => {
                bytes.extend_from_slice(reader.read_bytes(2)?);
            }
            2 => {
                bytes.extend_from_slice(reader.read_bytes(4)?);
            }
            3 => {
                bytes.extend_from_slice(reader.read_bytes(4)?);
            }
            4 => {
                let len_bytes = reader.read_bytes(2)?;
                let len = i16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
                bytes.extend_from_slice(len_bytes);
                bytes.extend_from_slice(reader.read_bytes(len * 2)?);
            }
            5 => {
                let item_id_bytes = reader.read_bytes(2)?;
                let item_id = i16::from_be_bytes([item_id_bytes[0], item_id_bytes[1]]);
                bytes.extend_from_slice(item_id_bytes);
                if item_id >= 0 {
                    bytes.push(reader.read_u8()?);
                    bytes.extend_from_slice(reader.read_bytes(2)?);
                }
            }
            6 => {
                bytes.extend_from_slice(reader.read_bytes(12)?);
            }
            _ => break,
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_equipment_round_trips() {
        let mut writer = Writer::new();
        let packet = EntityEquipment {
            entity_id: 10,
            slot: 0,
            item_id: 276,
            damage: 0,
        };
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(EntityEquipment::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn named_entity_spawn_round_trips() {
        let mut writer = Writer::new();
        let packet = NamedEntitySpawn {
            entity_id: 42,
            name: "Player".to_string(),
            x: 100,
            y: 64,
            z: -200,
            yaw: 0,
            pitch: 0,
            current_item: 0,
        };
        packet.encode(&mut writer).unwrap();

        let mut reader = Reader::new(writer.as_slice());
        let decoded = NamedEntitySpawn::decode(&mut reader).unwrap();
        assert_eq!(decoded.entity_id, 42);
        assert_eq!(decoded.name, "Player");
        assert_eq!(decoded.x, 100);
        assert_eq!(decoded.y, 64);
        assert_eq!(decoded.z, -200);
    }

    #[test]
    fn rel_entity_move_look_round_trips() {
        let mut writer = Writer::new();
        let packet = RelEntityMoveLook {
            entity_id: 5,
            dx: -2,
            dy: 4,
            dz: 1,
            yaw: 64,
            pitch: 128,
        };
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(RelEntityMoveLook::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn entity_velocity_round_trips() {
        let mut writer = Writer::new();
        let packet = EntityVelocity {
            entity_id: 8,
            mot_x: 2000,
            mot_y: -4000,
            mot_z: 1000,
        };
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(EntityVelocity::decode(&mut reader).unwrap(), packet);
    }

    #[test]
    fn attach_entity_round_trips() {
        let mut writer = Writer::new();
        let packet = AttachEntity {
            entity_id: 1,
            vehicle_id: -1,
        };
        packet.encode(&mut writer).unwrap();
        let mut reader = Reader::new(writer.as_slice());
        assert_eq!(AttachEntity::decode(&mut reader).unwrap(), packet);
    }
}
