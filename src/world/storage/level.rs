use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{Error, Result};
use crate::world::storage::nbt::NbtTag;

pub const CURRENT_MCREGION_VERSION: i32 = 19132;

#[derive(Debug, Clone)]
pub struct LevelData {
    pub seed: i64,
    pub spawn_x: i32,
    pub spawn_y: i32,
    pub spawn_z: i32,
    pub spawn_yaw: f32,
    pub spawn_pitch: f32,
    pub time: i64,
    pub last_played: i64,
    pub size_on_disk: i64,
    pub level_name: String,
    pub version: i32,
    pub rain_time: i32,
    pub raining: bool,
    pub thunder_time: i32,
    pub thundering: bool,
    /// Java `WorldData.a(List)`: the data of the first player of the world
    /// is kept in the compound `Player`.
    pub player: Option<NbtTag>,
}

impl Default for LevelData {
    fn default() -> Self {
        Self {
            seed: 0,
            spawn_x: 0,
            spawn_y: 64,
            spawn_z: 0,
            spawn_yaw: 0.0,
            spawn_pitch: 0.0,
            time: 0,
            player: None,
            last_played: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            size_on_disk: 0,
            level_name: "world".to_string(),
            version: CURRENT_MCREGION_VERSION,
            rain_time: 24000,
            raining: false,
            thunder_time: 24000,
            thundering: false,
        }
    }
}

impl LevelData {
    pub fn to_nbt(&self) -> NbtTag {
        let mut data = NbtTag::compound();
        data.insert("RandomSeed", NbtTag::Long(self.seed));
        data.insert("SpawnX", NbtTag::Int(self.spawn_x));
        data.insert("SpawnY", NbtTag::Int(self.spawn_y));
        data.insert("SpawnZ", NbtTag::Int(self.spawn_z));
        data.insert("SpawnYaw", NbtTag::Float(self.spawn_yaw));
        data.insert("SpawnPitch", NbtTag::Float(self.spawn_pitch));
        data.insert("Time", NbtTag::Long(self.time));
        data.insert("LastPlayed", NbtTag::Long(self.last_played));
        data.insert("SizeOnDisk", NbtTag::Long(self.size_on_disk));
        data.insert("LevelName", NbtTag::String(self.level_name.clone()));
        data.insert("version", NbtTag::Int(self.version));
        data.insert("rainTime", NbtTag::Int(self.rain_time));
        data.insert("raining", NbtTag::Byte(if self.raining { 1 } else { 0 }));
        data.insert("thunderTime", NbtTag::Int(self.thunder_time));
        data.insert(
            "thundering",
            NbtTag::Byte(if self.thundering { 1 } else { 0 }),
        );
        if let Some(player) = &self.player {
            data.insert("Player", player.clone());
        }

        let mut root = NbtTag::compound();
        root.insert("Data", data);
        root
    }

    pub fn from_nbt(root: &NbtTag) -> Result<Self> {
        let data = root
            .get_compound("Data")
            .ok_or(Error::Malformed("missing 'Data' compound in level.dat"))?;

        let seed = match data.get("RandomSeed") {
            Some(NbtTag::Long(v)) => *v,
            _ => 0,
        };
        let spawn_x = match data.get("SpawnX") {
            Some(NbtTag::Int(v)) => *v,
            _ => 0,
        };
        let spawn_y = match data.get("SpawnY") {
            Some(NbtTag::Int(v)) => *v,
            _ => 64,
        };
        let spawn_z = match data.get("SpawnZ") {
            Some(NbtTag::Int(v)) => *v,
            _ => 0,
        };
        let spawn_yaw = match data.get("SpawnYaw") {
            Some(NbtTag::Float(v)) => *v,
            _ => 0.0,
        };
        let spawn_pitch = match data.get("SpawnPitch") {
            Some(NbtTag::Float(v)) => *v,
            _ => 0.0,
        };
        let time = match data.get("Time") {
            Some(NbtTag::Long(v)) => *v,
            _ => 0,
        };
        let last_played = match data.get("LastPlayed") {
            Some(NbtTag::Long(v)) => *v,
            _ => 0,
        };
        let size_on_disk = match data.get("SizeOnDisk") {
            Some(NbtTag::Long(v)) => *v,
            _ => 0,
        };
        let level_name = match data.get("LevelName") {
            Some(NbtTag::String(v)) => v.clone(),
            _ => "world".to_string(),
        };
        let version = match data.get("version") {
            Some(NbtTag::Int(v)) => *v,
            _ => CURRENT_MCREGION_VERSION,
        };
        let rain_time = match data.get("rainTime") {
            Some(NbtTag::Int(v)) => *v,
            _ => 24000,
        };
        let raining = match data.get("raining") {
            Some(NbtTag::Byte(v)) => *v != 0,
            _ => false,
        };
        let thunder_time = match data.get("thunderTime") {
            Some(NbtTag::Int(v)) => *v,
            _ => 24000,
        };
        let thundering = match data.get("thundering") {
            Some(NbtTag::Byte(v)) => *v != 0,
            _ => false,
        };

        // Java `WorldData.a(List)`: the compound `Player` holds the data of
        // the first player of the world.
        let player = match data.get("Player") {
            Some(player @ NbtTag::Compound(_)) => Some(player.clone()),
            _ => None,
        };

        Ok(Self {
            player,
            seed,
            spawn_x,
            spawn_y,
            spawn_z,
            spawn_yaw,
            spawn_pitch,
            time,
            last_played,
            size_on_disk,
            level_name,
            version,
            rain_time,
            raining,
            thunder_time,
            thundering,
        })
    }

    pub fn load_from_dir(world_dir: &Path) -> Result<Option<Self>> {
        let level_dat = world_dir.join("level.dat");
        if level_dat.exists() {
            if let Ok(bytes) = fs::read(&level_dat) {
                if let Ok((_name, nbt)) = NbtTag::from_gzip(&bytes) {
                    if let Ok(data) = Self::from_nbt(&nbt) {
                        return Ok(Some(data));
                    }
                }
            }
        }

        // Fallback to level.dat_old
        let level_dat_old = world_dir.join("level.dat_old");
        if level_dat_old.exists() {
            if let Ok(bytes) = fs::read(&level_dat_old) {
                if let Ok((_name, nbt)) = NbtTag::from_gzip(&bytes) {
                    if let Ok(data) = Self::from_nbt(&nbt) {
                        return Ok(Some(data));
                    }
                }
            }
        }

        Ok(None)
    }

    pub fn save_to_dir(&mut self, world_dir: &Path) -> Result<()> {
        fs::create_dir_all(world_dir)?;

        self.last_played = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let nbt = self.to_nbt();
        let bytes = nbt.to_gzip("")?;

        let level_new = world_dir.join("level.dat_new");
        let level_old = world_dir.join("level.dat_old");
        let level_cur = world_dir.join("level.dat");

        // 1. Write to level.dat_new
        fs::write(&level_new, &bytes)?;

        // 2. If level.dat_old exists, remove it
        if level_old.exists() {
            let _ = fs::remove_file(&level_old);
        }

        // 3. Rename level.dat -> level.dat_old
        if level_cur.exists() {
            let _ = fs::rename(&level_cur, &level_old);
        }

        // 4. Rename level.dat_new -> level.dat
        fs::rename(&level_new, &level_cur)?;

        Ok(())
    }
}

pub struct SessionLock {
    lock_path: PathBuf,
    session_id: i64,
}

impl SessionLock {
    pub fn acquire(world_dir: &Path) -> Result<Self> {
        fs::create_dir_all(world_dir)?;
        let lock_path = world_dir.join("session.lock");
        let session_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let mut file = File::create(&lock_path)?;
        file.write_all(&session_id.to_be_bytes())?;
        file.flush()?;

        Ok(Self {
            lock_path,
            session_id,
        })
    }

    pub fn check(&self) -> Result<()> {
        if !self.lock_path.exists() {
            return Err(Error::InvalidState("session.lock file missing"));
        }
        let mut file = File::open(&self.lock_path)?;
        let mut buf = [0u8; 8];
        file.read_exact(&mut buf)?;
        let id_on_disk = i64::from_be_bytes(buf);
        if id_on_disk != self.session_id {
            return Err(Error::InvalidState(
                "session.lock mismatch - world is being accessed by another server process",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file of the level holds the compound `Player`.
    ///
    /// Java `WorldData.a(List)`: the compound `Player` holds the data of
    /// the first player of the world.
    #[test]
    fn the_file_of_the_level_holds_the_compound_of_the_player() {
        let temp_dir =
            std::env::temp_dir().join(format!("flintcore_level_player_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let mut level = LevelData::default();
        let mut player = NbtTag::compound();
        player.insert("Pos", NbtTag::List(vec![NbtTag::Double(1.0)]));
        level.player = Some(player.clone());
        level.save_to_dir(&temp_dir).unwrap();

        let bytes = fs::read(temp_dir.join("level.dat")).unwrap();
        let (_name, nbt) = NbtTag::from_gzip(&bytes).unwrap();
        let data = nbt.get_compound("Data").expect("the data of the level");
        assert!(data.get("Player").is_some(), "the compound of the player");
        let loaded = LevelData::load_from_dir(&temp_dir).unwrap().unwrap();
        assert!(loaded.player.is_some(), "the player of the level");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn level_data_atomic_round_trip() {
        let temp_dir =
            std::env::temp_dir().join(format!("flintcore_level_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let mut level = LevelData {
            seed: 1234567890,
            spawn_x: 10,
            spawn_y: 65,
            spawn_z: -20,
            spawn_yaw: 90.0,
            spawn_pitch: 0.0,
            time: 500,
            level_name: "TestWorld".into(),
            ..LevelData::default()
        };

        level.save_to_dir(&temp_dir).unwrap();
        assert!(temp_dir.join("level.dat").exists());

        let loaded = LevelData::load_from_dir(&temp_dir).unwrap().unwrap();
        assert_eq!(loaded.seed, 1234567890);
        assert_eq!(loaded.spawn_x, 10);
        assert_eq!(loaded.spawn_y, 65);
        assert_eq!(loaded.spawn_z, -20);
        assert_eq!(loaded.level_name, "TestWorld");
        assert_eq!(loaded.time, 500);

        let lock = SessionLock::acquire(&temp_dir).unwrap();
        assert!(lock.check().is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
