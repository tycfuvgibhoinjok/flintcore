use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::util::{Error, Result};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub server: ServerConfig,
    pub network: NetworkConfig,
    pub world: WorldConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            server: ServerConfig::default(),
            network: NetworkConfig::default(),
            world: WorldConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Config =
            toml::from_str(&text).map_err(|error| Error::Config(error.to_string()))?;
        config.migrate()?;
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            let config = Config::default();
            config.save(path)?;
            Ok(config)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text =
            toml::to_string_pretty(self).map_err(|error| Error::Config(error.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(Error::Config(format!(
                "schema version {} is newer than supported {}",
                self.schema_version, CURRENT_SCHEMA_VERSION
            )));
        }
        self.schema_version = CURRENT_SCHEMA_VERSION;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub max_players: u32,
    pub motd: String,
    pub online_mode: bool,
    pub join_message: String,
    pub leave_message: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".to_string(),
            port: 25565,
            max_players: 20,
            motd: "flintcore Beta 1.7.3".to_string(),
            online_mode: false,
            join_message: "\u{00A7}e%player% joined the game.".to_string(),
            leave_message: "\u{00A7}e%player% left the game.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub tcp_nodelay: bool,
    pub inbound_capacity: usize,
    pub outbound_capacity: usize,
    pub max_read_buffer: usize,
    pub max_packets_per_tick: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            tcp_nodelay: true,
            inbound_capacity: 4096,
            outbound_capacity: 1024,
            max_read_buffer: 1024 * 1024,
            max_packets_per_tick: 2048,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldConfig {
    pub seed: i64,
    pub level_name: String,
    pub save_enabled: bool,
    pub view_distance: i32,
    pub chunks_per_tick: usize,
    pub gen_workers: usize,
    pub compression_workers: usize,
    pub spawn_x: i32,
    pub spawn_y: i32,
    pub spawn_z: i32,
    pub spawn_radius: i32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            level_name: "world".to_string(),
            save_enabled: true,
            view_distance: 10,
            chunks_per_tick: 16,
            gen_workers: 0,
            compression_workers: 0,
            spawn_x: 0,
            spawn_y: 64,
            spawn_z: 0,
            spawn_radius: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.server.port, 25565);
        assert_eq!(parsed.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(parsed.world.seed, 0);
        assert_eq!(parsed.world.view_distance, 10);
    }

    #[test]
    fn newer_schema_is_rejected() {
        let mut config = Config {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..Config::default()
        };
        assert!(config.migrate().is_err());
    }
}
