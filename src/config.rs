use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::util::{Error, Result};
use crate::world::biome::Biome;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub server: ServerConfig,
    pub messages: MessagesConfig,
    pub network: NetworkConfig,
    pub world: WorldConfig,
    pub spawning: SpawningConfig,
    pub worldgen: WorldgenConfig,
    pub mechanics: MechanicsConfig,
    pub fixes: FixesConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            server: ServerConfig::default(),
            messages: MessagesConfig::default(),
            network: NetworkConfig::default(),
            world: WorldConfig::default(),
            spawning: SpawningConfig::default(),
            worldgen: WorldgenConfig::default(),
            mechanics: MechanicsConfig::default(),
            fixes: FixesConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Config =
            toml::from_str(&text).map_err(|error| Error::Config(error.to_string()))?;
        config.migrate()?;
        config.validate()?;
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

    /// Checks the values that the deserializer cannot verify on its own.
    fn validate(&self) -> Result<()> {
        let spawning = &self.spawning;
        if spawning.spawn_interval_ticks == 0 {
            return Err(Error::Config(
                "spawning.spawn_interval_ticks must be at least 1".to_string(),
            ));
        }
        if spawning.spawn_radius_chunks < 0 {
            return Err(Error::Config(
                "spawning.spawn_radius_chunks must not be negative".to_string(),
            ));
        }
        if spawning.chunk_sample_divisor < 1 {
            return Err(Error::Config(
                "spawning.chunk_sample_divisor must be at least 1".to_string(),
            ));
        }
        if spawning.min_player_distance < 0.0 || spawning.min_spawn_distance < 0.0 {
            return Err(Error::Config(
                "spawning.min_player_distance and spawning.min_spawn_distance must not be negative"
                    .to_string(),
            ));
        }
        for (name, mob) in &spawning.mobs {
            if !MOB_SPAWN_NAMES.contains(&name.as_str()) {
                return Err(Error::Config(format!(
                    "unknown mob '{name}' in [spawning.mobs], known mobs: {}",
                    MOB_SPAWN_NAMES.join(", ")
                )));
            }
            if mob.weight < 0 {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name}.weight must not be negative"
                )));
            }
            if mob.max_group < 1 {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name}.max_group must be at least 1"
                )));
            }
            if mob.spawn_chance < 0 || mob.spawn_chance > 100 {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name}.spawn_chance must be between 0 and 100"
                )));
            }
            if mob.light_roll < 0 || mob.sky_light_roll < 0 {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name} light rolls must not be negative"
                )));
            }
            if mob.min_light < 0 {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name}.min_light must not be negative"
                )));
            }
            if mob.min_y > mob.max_y {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name}.min_y must not be greater than max_y"
                )));
            }
            if mob.jockey_chance < 0 {
                return Err(Error::Config(format!(
                    "spawning.mobs.{name}.jockey_chance must not be negative"
                )));
            }
            for biome in &mob.biomes {
                if biome == "*" {
                    continue;
                }
                if Biome::from_name(biome).is_none() {
                    return Err(Error::Config(format!(
                        "unknown biome '{biome}' in spawning.mobs.{name}.biomes"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Server settings.
///
/// Sources: `server.properties` (the vanilla server), `Poseidon`
/// (`settings.check-username-validity.*`, `settings.cracked-username-prefix`,
/// `command.tps`), `Uberbukkit` (`settings.exempt-staff-from-flight-kick`) and
/// the flintcore extensions (the console, the join/leave messages live in
/// `[messages]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub max_players: u32,
    pub motd: String,
    pub online_mode: bool,
    pub white_list: bool,
    pub pvp: bool,
    pub allow_flight: bool,
    pub exempt_staff_from_flight_kick: bool,
    pub allow_nether: bool,
    pub check_username_validity: bool,
    pub username_min_length: usize,
    pub username_max_length: usize,
    pub username_regex: String,
    pub cracked_username_prefix: bool,
    pub save_playerdata: bool,
    pub autosave_interval_ticks: u64,
    pub time_speed_multiplier: i64,
    pub enable_tps_command: bool,
    /// Read server commands from the process' standard input (the vanilla
    /// server console). Only the `flintcore` binary starts the reader.
    pub console_enabled: bool,
    /// Name the console sender appears under in chat and in the log
    /// (`MinecraftServer.getName()`).
    pub console_name: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".to_string(),
            port: 25565,
            max_players: 20,
            motd: "flintcore Beta 1.7.3".to_string(),
            online_mode: false,
            white_list: false,
            pvp: true,
            allow_flight: false,
            exempt_staff_from_flight_kick: false,
            allow_nether: true,
            check_username_validity: true,
            username_min_length: 3,
            username_max_length: 16,
            username_regex: "[a-zA-Z0-9_.]*".to_string(),
            cracked_username_prefix: false,
            save_playerdata: true,
            // Java `World.p`: the world is saved every forty ticks.
            autosave_interval_ticks: 40,
            time_speed_multiplier: 1,
            enable_tps_command: true,
            console_enabled: true,
            console_name: "CONSOLE".to_string(),
        }
    }
}

/// Customizable server messages (`PoseidonConfig: message.*`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MessagesConfig {
    pub join: String,
    pub leave: String,
    pub kick_banned: String,
    pub kick_ip_banned: String,
    pub kick_not_whitelisted: String,
    pub kick_full: String,
    pub kick_shutdown: String,
    pub kick_already_online: String,
    pub remove_join_leave_debug: bool,
}

impl Default for MessagesConfig {
    fn default() -> Self {
        Self {
            join: "\u{00A7}e%player% joined the game.".to_string(),
            leave: "\u{00A7}e%player% left the game.".to_string(),
            kick_banned: "You are banned from this server!".to_string(),
            kick_ip_banned: "Your IP address is banned from this server!".to_string(),
            kick_not_whitelisted: "You are not white-listed on this server!".to_string(),
            kick_full: "The server is full!".to_string(),
            kick_shutdown: "\u{00A7}cServer is shutting down, please rejoin later.".to_string(),
            kick_already_online: "\u{00A7}cA player with your username or uuid is already online, try reconnecting in a minute.".to_string(),
            remove_join_leave_debug: true,
        }
    }
}

/// Network settings.
///
/// Sources: `Poseidon` (`settings.enable-tpc-nodelay`, `settings.faster-packets`,
/// `settings.packet-spam-detection.*`, `settings.release2beta.*`,
/// `settings.bungeecord.*`), `Uberbukkit` (`client.allowed_protocols`) and the
/// flintcore buffer limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// `Poseidon: settings.enable-tpc-nodelay` — the Poseidon default is
    /// `false`, flintcore enables it because Nagle delays the small packets
    /// of the protocol.
    pub tcp_nodelay: bool,
    pub faster_packets: bool,
    pub inbound_capacity: usize,
    pub outbound_capacity: usize,
    pub max_read_buffer: usize,
    pub max_packets_per_tick: usize,
    pub packet_spam_detection: bool,
    pub packet_spam_threshold: usize,
    pub allowed_protocols: Vec<i32>,
    pub release2beta_ip_passthrough: bool,
    pub release2beta_proxy_ip: String,
    pub bungeecord_mode: bool,
    pub bungeecord_kick_message: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            tcp_nodelay: true,
            faster_packets: true,
            inbound_capacity: 4096,
            outbound_capacity: 1024,
            max_read_buffer: 1024 * 1024,
            max_packets_per_tick: 2048,
            packet_spam_detection: true,
            packet_spam_threshold: 10000,
            allowed_protocols: vec![14],
            release2beta_ip_passthrough: false,
            release2beta_proxy_ip: "127.0.0.1".to_string(),
            bungeecord_mode: false,
            bungeecord_kick_message: "You must connect through BungeeCord to join this server!"
                .to_string(),
        }
    }
}

/// World and world generation settings.
///
/// Sources: `server.properties` (the vanilla server), `Poseidon`
/// (`world-settings.randomize-spawn`,
/// `world-settings.teleport-to-highest-safe-block`,
/// `emergency.debug.regenerate-corrupt-chunks`) and the flintcore
/// extensions (the spawn coordinates, the worker pools).
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
    pub spawn_yaw: f32,
    pub spawn_pitch: f32,
    pub spawn_radius: i32,
    /// Automatically search for a sandy beach spawn on fresh world creation when `spawn_x/y/z` are at default `(0, 64, 0)`.
    pub auto_find_spawn: bool,
    /// Force `spawn_x/y/z` from `config.toml` to overwrite the saved `level.dat` spawn even on existing worlds.
    pub override_saved_spawn: bool,
    /// `Poseidon: world-settings.randomize-spawn` — apply `±randomize_spawn_radius` X/Z offset on initial player spawn.
    pub randomize_spawn: bool,
    pub randomize_spawn_radius: i32,
    /// `Poseidon: world-settings.teleport-to-highest-safe-block` — elevate spawn/login above solid blocks.
    /// Set to `false` to spawn players inside underground rooms or indoor hubs at exact `spawn_y`.
    pub teleport_to_highest_safe_block: bool,
    /// Keep the `±128` block spawn region permanently loaded in memory (`bukkit.yml: keep-spawn-loaded`).
    pub keep_spawn_in_memory: bool,
    /// `Poseidon: emergency.debug.regenerate-corrupt-chunks.enable`.
    pub regenerate_corrupt_chunks: bool,
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
            spawn_yaw: 0.0,
            spawn_pitch: 0.0,
            spawn_radius: 16,
            auto_find_spawn: true,
            override_saved_spawn: false,
            randomize_spawn: false,
            randomize_spawn_radius: 10,
            teleport_to_highest_safe_block: true,
            keep_spawn_in_memory: true,
            regenerate_corrupt_chunks: false,
        }
    }
}

/// Natural mob spawning and dungeon mob spawner limits
/// (`server.properties` + `Poseidon` + `Uberbukkit` + `flintcore`).
///
/// The per mob settings live in `[spawning.mobs.<mob>]`; every mob has its
/// own section with the values of the Java code as defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(from = "SpawningConfigRaw")]
pub struct SpawningConfig {
    pub spawn_monsters: bool,
    pub spawn_animals: bool,
    pub spawn_water_mobs: bool,
    pub spawn_squids: bool,
    pub spawn_wolves: bool,
    pub spawn_slimes: bool,
    pub spawn_sheep_shades_of_black: bool,
    pub spawn_brown_and_pink_sheep: bool,
    pub monster_cap: usize,
    pub animal_cap: usize,
    pub water_mob_cap: usize,
    pub mob_spawner_area_limit_enabled: bool,
    pub mob_spawner_area_limit: usize,
    pub mob_spawner_chunk_radius: i32,
    /// Ticks between two natural spawn attempts.
    pub spawn_interval_ticks: u64,
    /// Radius in chunks around a player that is scanned for spawns
    /// (`SpawnerCreature.spawnEntities` uses a radius of 8 chunks).
    pub spawn_radius_chunks: i32,
    /// Minimum distance to the closest player (`SpawnerCreature` uses 24).
    pub min_player_distance: f64,
    /// Minimum distance to the world spawn point (`SpawnerCreature` uses 24).
    pub min_spawn_distance: f64,
    /// Only every N-th candidate chunk is rolled per attempt. `1` matches the
    /// Java behaviour, `16` is the flintcore default that keeps the tick cheap.
    pub chunk_sample_divisor: i32,
    /// Per mob settings, keyed by the mob name (`[spawning.mobs.zombie]`).
    pub mobs: BTreeMap<String, MobSpawnSettings>,
}

impl Default for SpawningConfig {
    fn default() -> Self {
        Self {
            spawn_monsters: true,
            spawn_animals: true,
            spawn_water_mobs: true,
            spawn_squids: true,
            spawn_wolves: true,
            spawn_slimes: true,
            spawn_sheep_shades_of_black: true,
            spawn_brown_and_pink_sheep: true,
            monster_cap: 70,
            animal_cap: 15,
            water_mob_cap: 5,
            mob_spawner_area_limit_enabled: true,
            mob_spawner_area_limit: 150,
            mob_spawner_chunk_radius: 8,
            // The Java code spawns every tick and rolls every candidate chunk
            // (`World.doTick` -> `SpawnerCreature.spawnEntities`).
            spawn_interval_ticks: 1,
            spawn_radius_chunks: 8,
            min_player_distance: 24.0,
            min_spawn_distance: 24.0,
            chunk_sample_divisor: 1,
            mobs: default_mob_settings(),
        }
    }
}

/// The names of the mobs that can be configured in `[spawning.mobs]`.
pub const MOB_SPAWN_NAMES: [&str; 14] = [
    "chicken",
    "cow",
    "creeper",
    "giant",
    "ghast",
    "pig",
    "pig_zombie",
    "sheep",
    "skeleton",
    "slime",
    "spider",
    "squid",
    "wolf",
    "zombie",
];

/// The default spawn settings of a single mob.
///
/// The values are the ones of the Project-Poseidon code: the weights come
/// from `BiomeBase` (`BiomeMeta.b`), the group size from `EntityLiving.l()`,
/// the light and ground rules from `EntityAnimal.d()` and the special rules
/// from `EntitySlime.d()` and `EntityGhast.d()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MobSpawnSettings {
    /// Whether the mob can spawn naturally at all.
    pub enabled: bool,
    /// Weight of the mob inside the biome spawn table (`BiomeMeta.b`).
    pub weight: i32,
    /// Maximum number of mobs spawned in one group (`EntityLiving.l()`).
    pub max_group: i32,
    /// Biomes the mob spawns in. `*` means every overworld biome, `hell` the
    /// Nether. An empty list keeps the mob out of every biome.
    pub biomes: Vec<String>,
    /// The light level has to be at least this value (`EntityAnimal.d()`
    /// requires a light level above 8).
    pub min_light: i32,
    /// The block light has to be at most `random(light_roll)`. `0` disables
    /// the check.
    pub light_roll: i32,
    /// The sky light has to be at most `random(sky_light_roll)`. `0` disables
    /// the check.
    pub sky_light_roll: i32,
    /// Lowest block height the mob can spawn at.
    pub min_y: i32,
    /// Highest block height the mob can spawn at.
    pub max_y: i32,
    /// The mob spawns with a probability of `spawn_chance` percent. `100`
    /// means that the mob always spawns (`EntitySlime.d()` uses 10, which is
    /// the `random.nextInt(10) == 0` of the Java code, `EntityGhast.d()` 5).
    pub spawn_chance: i32,
    /// The block below the mob has to be grass (`EntityAnimal.d()`).
    pub require_grass_below: bool,
    /// The mob only spawns in water (`EnumCreatureType.WATER_CREATURE`).
    pub require_water: bool,
    /// The mob only spawns in slime chunks (`EntitySlime.d()`).
    pub slime_chunk_only: bool,
    /// Chance of the spider jockey (`SpawnerCreature` uses 1/100).
    pub jockey_chance: i32,
}

impl Default for MobSpawnSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 10,
            max_group: 4,
            biomes: vec!["*".to_string()],
            min_light: 0,
            light_roll: 8,
            sky_light_roll: 32,
            min_y: 0,
            max_y: 127,
            spawn_chance: 100,
            require_grass_below: false,
            require_water: false,
            slime_chunk_only: false,
            jockey_chance: 0,
        }
    }
}

/// The Java defaults of every configurable mob.
pub fn default_mob_settings() -> BTreeMap<String, MobSpawnSettings> {
    let base = MobSpawnSettings::default();
    // The animals of `EntityAnimal` are checked against the light level of
    // the block they stand on instead of the random light rolls.
    let animal = MobSpawnSettings {
        light_roll: 0,
        sky_light_roll: 0,
        min_light: 9,
        require_grass_below: true,
        ..base.clone()
    };
    let mut mobs = BTreeMap::new();
    for (name, settings) in [
        ("chicken", animal.clone()),
        (
            "cow",
            MobSpawnSettings {
                weight: 8,
                ..animal.clone()
            },
        ),
        ("creeper", base.clone()),
        // `EntityGiant` is not part of the biome spawn tables.
        (
            "giant",
            MobSpawnSettings {
                enabled: false,
                weight: 0,
                ..base.clone()
            },
        ),
        (
            "ghast",
            MobSpawnSettings {
                max_group: 1,
                biomes: vec!["hell".to_string()],
                spawn_chance: 5,
                ..base.clone()
            },
        ),
        ("pig", animal.clone()),
        (
            "pig_zombie",
            MobSpawnSettings {
                biomes: vec!["hell".to_string()],
                ..base.clone()
            },
        ),
        (
            "sheep",
            MobSpawnSettings {
                weight: 12,
                ..animal.clone()
            },
        ),
        ("skeleton", base.clone()),
        // `EntitySlime.d()` checks the height, the slime chunk and a 1/10
        // chance, but not the light level.
        (
            "slime",
            MobSpawnSettings {
                light_roll: 0,
                sky_light_roll: 0,
                max_y: 15,
                spawn_chance: 10,
                slime_chunk_only: true,
                ..base.clone()
            },
        ),
        (
            "spider",
            MobSpawnSettings {
                jockey_chance: 100,
                ..base.clone()
            },
        ),
        (
            "squid",
            MobSpawnSettings {
                light_roll: 0,
                sky_light_roll: 0,
                require_water: true,
                ..base.clone()
            },
        ),
        (
            "wolf",
            MobSpawnSettings {
                weight: 2,
                max_group: 8,
                biomes: vec!["forest".to_string(), "taiga".to_string()],
                ..animal.clone()
            },
        ),
        ("zombie", base.clone()),
    ] {
        mobs.insert(name.to_string(), settings);
    }
    mobs
}

impl MobSpawnSettings {
    /// Whether the mob may spawn in the given biome.
    ///
    /// `*` covers every overworld biome, the Nether has to be listed with the
    /// name of its biome (`hell`).
    pub fn spawns_in(&self, biome: Biome) -> bool {
        self.biomes.iter().any(|name| {
            if name == "*" {
                biome != Biome::Hell
            } else {
                Biome::from_name(name) == Some(biome)
            }
        })
    }
}

/// A partial per mob configuration; unset fields keep the Java default.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct MobSpawnOverride {
    enabled: Option<bool>,
    weight: Option<i32>,
    max_group: Option<i32>,
    biomes: Option<Vec<String>>,
    min_light: Option<i32>,
    light_roll: Option<i32>,
    sky_light_roll: Option<i32>,
    min_y: Option<i32>,
    max_y: Option<i32>,
    spawn_chance: Option<i32>,
    require_grass_below: Option<bool>,
    require_water: Option<bool>,
    slime_chunk_only: Option<bool>,
    jockey_chance: Option<i32>,
}

impl MobSpawnOverride {
    fn apply(&self, settings: &mut MobSpawnSettings) {
        if let Some(value) = self.enabled {
            settings.enabled = value;
        }
        if let Some(value) = self.weight {
            settings.weight = value;
        }
        if let Some(value) = self.max_group {
            settings.max_group = value;
        }
        if let Some(value) = &self.biomes {
            settings.biomes = value.clone();
        }
        if let Some(value) = self.min_light {
            settings.min_light = value;
        }
        if let Some(value) = self.light_roll {
            settings.light_roll = value;
        }
        if let Some(value) = self.sky_light_roll {
            settings.sky_light_roll = value;
        }
        if let Some(value) = self.min_y {
            settings.min_y = value;
        }
        if let Some(value) = self.max_y {
            settings.max_y = value;
        }
        if let Some(value) = self.spawn_chance {
            settings.spawn_chance = value;
        }
        if let Some(value) = self.require_grass_below {
            settings.require_grass_below = value;
        }
        if let Some(value) = self.require_water {
            settings.require_water = value;
        }
        if let Some(value) = self.slime_chunk_only {
            settings.slime_chunk_only = value;
        }
        if let Some(value) = self.jockey_chance {
            settings.jockey_chance = value;
        }
    }
}

/// The deserialised shape of [`SpawningConfig`]; the per mob overrides are
/// merged onto the Java defaults while loading.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SpawningConfigRaw {
    spawn_monsters: bool,
    spawn_animals: bool,
    spawn_water_mobs: bool,
    spawn_squids: bool,
    spawn_wolves: bool,
    spawn_slimes: bool,
    spawn_sheep_shades_of_black: bool,
    spawn_brown_and_pink_sheep: bool,
    monster_cap: usize,
    animal_cap: usize,
    water_mob_cap: usize,
    mob_spawner_area_limit_enabled: bool,
    mob_spawner_area_limit: usize,
    mob_spawner_chunk_radius: i32,
    spawn_interval_ticks: u64,
    spawn_radius_chunks: i32,
    min_player_distance: f64,
    min_spawn_distance: f64,
    chunk_sample_divisor: i32,
    mobs: BTreeMap<String, MobSpawnOverride>,
}

impl Default for SpawningConfigRaw {
    fn default() -> Self {
        let defaults = SpawningConfig::default();
        Self {
            spawn_monsters: defaults.spawn_monsters,
            spawn_animals: defaults.spawn_animals,
            spawn_water_mobs: defaults.spawn_water_mobs,
            spawn_squids: defaults.spawn_squids,
            spawn_wolves: defaults.spawn_wolves,
            spawn_slimes: defaults.spawn_slimes,
            spawn_sheep_shades_of_black: defaults.spawn_sheep_shades_of_black,
            spawn_brown_and_pink_sheep: defaults.spawn_brown_and_pink_sheep,
            monster_cap: defaults.monster_cap,
            animal_cap: defaults.animal_cap,
            water_mob_cap: defaults.water_mob_cap,
            mob_spawner_area_limit_enabled: defaults.mob_spawner_area_limit_enabled,
            mob_spawner_area_limit: defaults.mob_spawner_area_limit,
            mob_spawner_chunk_radius: defaults.mob_spawner_chunk_radius,
            spawn_interval_ticks: defaults.spawn_interval_ticks,
            spawn_radius_chunks: defaults.spawn_radius_chunks,
            min_player_distance: defaults.min_player_distance,
            min_spawn_distance: defaults.min_spawn_distance,
            chunk_sample_divisor: defaults.chunk_sample_divisor,
            mobs: BTreeMap::new(),
        }
    }
}

impl From<SpawningConfigRaw> for SpawningConfig {
    fn from(raw: SpawningConfigRaw) -> Self {
        // The mobs that are not mentioned in the file keep their Java
        // defaults.
        let mut mobs = SpawningConfig::default().mobs;
        for (name, over) in raw.mobs {
            let mut settings = mobs.get(&name).cloned().unwrap_or_default();
            over.apply(&mut settings);
            mobs.insert(name, settings);
        }
        SpawningConfig {
            spawn_monsters: raw.spawn_monsters,
            spawn_animals: raw.spawn_animals,
            spawn_water_mobs: raw.spawn_water_mobs,
            spawn_squids: raw.spawn_squids,
            spawn_wolves: raw.spawn_wolves,
            spawn_slimes: raw.spawn_slimes,
            spawn_sheep_shades_of_black: raw.spawn_sheep_shades_of_black,
            spawn_brown_and_pink_sheep: raw.spawn_brown_and_pink_sheep,
            monster_cap: raw.monster_cap,
            animal_cap: raw.animal_cap,
            water_mob_cap: raw.water_mob_cap,
            mob_spawner_area_limit_enabled: raw.mob_spawner_area_limit_enabled,
            mob_spawner_area_limit: raw.mob_spawner_area_limit,
            mob_spawner_chunk_radius: raw.mob_spawner_chunk_radius,
            spawn_interval_ticks: raw.spawn_interval_ticks,
            spawn_radius_chunks: raw.spawn_radius_chunks,
            min_player_distance: raw.min_player_distance,
            min_spawn_distance: raw.min_spawn_distance,
            chunk_sample_divisor: raw.chunk_sample_divisor,
            mobs,
        }
    }
}

/// World generation toggles (`Uberbukkit: worldgen.*` + `Poseidon: world.settings.block-tree-growth.*`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldgenConfig {
    pub cocoabeans_loot: bool,
    pub pre_b1_2_ore_generation: bool,
    pub pre_b1_2_tree_generation: bool,
    pub generate_sandstone: bool,
    pub generate_spruces: bool,
    pub generate_birches: bool,
    pub generate_steveco_chests: bool,
    pub generate_lapis_ores: bool,
    pub generate_tallgrass: bool,
    pub custom_ore_seed_enabled: bool,
    pub custom_ore_seed: i64,
    pub block_tree_growth_enabled: bool,
    pub block_tree_growth_blacklist: Vec<u8>,
}

impl Default for WorldgenConfig {
    fn default() -> Self {
        Self {
            cocoabeans_loot: true,
            pre_b1_2_ore_generation: false,
            pre_b1_2_tree_generation: false,
            generate_sandstone: true,
            generate_spruces: true,
            generate_birches: true,
            generate_steveco_chests: false,
            generate_lapis_ores: true,
            generate_tallgrass: true,
            custom_ore_seed_enabled: false,
            custom_ore_seed: 0,
            block_tree_growth_enabled: true,
            block_tree_growth_blacklist: vec![54, 63, 68],
        }
    }
}

/// Gameplay mechanics and historical version switches (`Poseidon` + `Uberbukkit` + `flintcore` extras).
///
/// The `Uberbukkit` options are named `mechanics.*` in `settings.yml`, the
/// `Poseidon` ones live in `poseidon.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MechanicsConfig {
    // Explosions & Fire
    pub optimized_explosions: bool,
    pub send_explosion_velocity: bool,
    pub creeper_block_damage: bool,
    pub tnt_block_damage: bool,
    pub ghast_block_damage: bool,
    pub tnt_require_lighter: bool,
    pub fire_spread: bool,
    pub flammable_fences_stairs: bool,
    pub pre_1_6_fire: bool,

    // Vehicles (Minecarts & Boats)
    pub allow_minecart_boosters: bool,
    pub minecart_max_speed: f64,
    pub minecart_slow_when_empty: bool,
    pub boats_drop_boat_not_wood: bool,
    pub boats_break_on_collision: bool,

    // Combat, Players & Mobs
    pub player_knockback_fix: bool,
    pub fix_drowning_push_down: bool,
    pub skeleton_shooting_sound_fix: bool,
    pub arrows_pickup_by_others: bool,
    pub sheep_drop_wool_on_punch: bool,
    pub burning_pig_drop_cooked_meat: bool,
    pub spiders_trample_crops: bool,
    pub spiders_climb_walls: bool,
    pub allow_milking_squids: bool,
    pub keep_inventory_on_death: bool,

    // Blocks, Weather, Farming & Recipes
    pub do_weather: bool,
    pub ice_generate_only_when_snowing: bool,
    pub leaf_decay: bool,
    pub drop_saplings_of_leaf_type: bool,
    pub tile_grass_drop_seeds: bool,
    pub allow_bone_meal_on_grass: bool,
    pub allow_grow_tallgrass: bool,
    pub mushroom_spread: bool,
    pub trample_farmland_above_fence: bool,
    pub seeds_replace_blocks: bool,
    pub use_modern_fence_bounding_boxes: bool,
    pub allow_1_7_fence_placement: bool,
    pub allow_blocks_at_y_127: bool,
    pub allow_ladder_gap: bool,
    pub glowstone_pre_1_6_6: bool,
    pub wool_recipe_pre_1_6_6: bool,
    pub old_slab_recipe: bool,
    pub drop_lapis_as_b1_2: bool,
    pub pre_b1_5_pumpkins: bool,
    pub beds_set_spawnpoint: bool,
    pub nether_bed_explode: bool,
    pub flowing_lava_fix: bool,
    pub optimize_sponges: bool,
    pub sponge_absorption_enabled: bool,
    pub sponge_radius: i32,
}

impl Default for MechanicsConfig {
    fn default() -> Self {
        Self {
            optimized_explosions: false,
            send_explosion_velocity: true,
            creeper_block_damage: true,
            tnt_block_damage: true,
            ghast_block_damage: true,
            tnt_require_lighter: true,
            fire_spread: true,
            flammable_fences_stairs: true,
            pre_1_6_fire: false,

            allow_minecart_boosters: false,
            minecart_max_speed: 0.4,
            minecart_slow_when_empty: true,
            boats_drop_boat_not_wood: false,
            boats_break_on_collision: true,

            player_knockback_fix: true,
            fix_drowning_push_down: true,
            skeleton_shooting_sound_fix: true,
            arrows_pickup_by_others: true,
            sheep_drop_wool_on_punch: false,
            burning_pig_drop_cooked_meat: true,
            spiders_trample_crops: false,
            spiders_climb_walls: true,
            allow_milking_squids: false,
            keep_inventory_on_death: false,

            do_weather: true,
            ice_generate_only_when_snowing: false,
            leaf_decay: true,
            drop_saplings_of_leaf_type: true,
            tile_grass_drop_seeds: false,
            allow_bone_meal_on_grass: true,
            allow_grow_tallgrass: true,
            mushroom_spread: true,
            trample_farmland_above_fence: false,
            seeds_replace_blocks: false,
            use_modern_fence_bounding_boxes: false,
            allow_1_7_fence_placement: true,
            allow_blocks_at_y_127: false,
            allow_ladder_gap: false,
            glowstone_pre_1_6_6: false,
            wool_recipe_pre_1_6_6: false,
            old_slab_recipe: false,
            drop_lapis_as_b1_2: false,
            pre_b1_5_pumpkins: false,
            beds_set_spawnpoint: true,
            nether_bed_explode: true,
            flowing_lava_fix: true,
            optimize_sponges: true,
            sponge_absorption_enabled: false,
            sponge_radius: 2,
        }
    }
}

/// Anti-exploit, dupe patches, and movement validation (`Poseidon` + `Uberbukkit`).
///
/// Sources: `Poseidon` (`world.settings.pistons.*`,
/// `world.settings.speed-hack-check.*`), `Uberbukkit`
/// (`patch.container-interaction.max-distance`) and the flintcore patches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FixesConfig {
    pub block_pistons_pushing_furnaces: bool,
    pub piston_transmutation_fix: bool,
    pub piston_sand_gravel_duping_fix: bool,
    pub piston_other_fixes: bool,
    pub speed_hack_check_enabled: bool,
    pub speed_hack_teleport: bool,
    pub speed_hack_max_distance: f64,
    pub container_interaction_check_enabled: bool,
    pub container_interaction_max_distance: i32,
    pub container_interaction_log: bool,
    pub head_rotation_patch_enabled: bool,
    pub prevent_vehicle_chest_open: bool,
}

impl Default for FixesConfig {
    fn default() -> Self {
        Self {
            block_pistons_pushing_furnaces: true,
            piston_transmutation_fix: true,
            piston_sand_gravel_duping_fix: true,
            piston_other_fixes: true,
            speed_hack_check_enabled: true,
            speed_hack_teleport: true,
            speed_hack_max_distance: 100.0,
            container_interaction_check_enabled: true,
            container_interaction_max_distance: 4,
            container_interaction_log: false,
            head_rotation_patch_enabled: true,
            prevent_vehicle_chest_open: true,
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
        assert!(parsed.world.auto_find_spawn);
        assert!(!parsed.world.override_saved_spawn);
        assert!(parsed.mechanics.tnt_require_lighter);
        assert!(!parsed.mechanics.allow_minecart_boosters);
        assert!(parsed.fixes.block_pistons_pushing_furnaces);
        assert_eq!(parsed.spawning.monster_cap, 70);
    }

    #[test]
    fn newer_schema_is_rejected() {
        let mut config = Config {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..Config::default()
        };
        assert!(config.migrate().is_err());
    }

    #[test]
    fn mob_spawn_defaults_match_the_java_values() {
        let config = Config::default();
        let mobs = &config.spawning.mobs;
        assert_eq!(mobs.len(), MOB_SPAWN_NAMES.len());
        for name in MOB_SPAWN_NAMES {
            assert!(mobs.contains_key(name), "{name} is missing");
        }

        // `BiomeBase` (the weights of the biome spawn tables).
        assert_eq!(mobs["zombie"].weight, 10);
        assert_eq!(mobs["creeper"].weight, 10);
        assert_eq!(mobs["sheep"].weight, 12);
        assert_eq!(mobs["cow"].weight, 8);
        assert_eq!(mobs["wolf"].weight, 2);
        assert_eq!(mobs["squid"].weight, 10);
        // `EntityLiving.l()` (the size of a spawn group).
        assert_eq!(mobs["zombie"].max_group, 4);
        assert_eq!(mobs["wolf"].max_group, 8);
        assert_eq!(mobs["ghast"].max_group, 1);
        // `BiomeForest`, `BiomeTaiga` and `BiomeHell`.
        assert_eq!(mobs["wolf"].biomes, vec!["forest", "taiga"]);
        assert_eq!(mobs["ghast"].biomes, vec!["hell"]);
        assert_eq!(mobs["pig_zombie"].biomes, vec!["hell"]);
        assert_eq!(mobs["zombie"].biomes, vec!["*"]);
        // `EntityAnimal.d()` (grass below and a light level above 8).
        assert_eq!(mobs["pig"].min_light, 9);
        assert!(mobs["pig"].require_grass_below);
        assert_eq!(mobs["pig"].light_roll, 0);
        // `EntitySlime.d()` and `EntityGhast.d()`.
        assert_eq!(mobs["slime"].spawn_chance, 10);
        assert!(mobs["slime"].slime_chunk_only);
        assert_eq!(mobs["slime"].max_y, 15);
        assert_eq!(mobs["ghast"].spawn_chance, 5);
        // Every other mob always spawns.
        assert_eq!(mobs["zombie"].spawn_chance, 100);
        assert_eq!(mobs["sheep"].spawn_chance, 100);
        // `SpawnerCreature` (the spider jockey).
        assert_eq!(mobs["spider"].jockey_chance, 100);
        // `EntityGiant` is not part of the spawn tables.
        assert!(!mobs["giant"].enabled);
        // `EnumCreatureType` (the caps of the categories).
        assert_eq!(config.spawning.monster_cap, 70);
        assert_eq!(config.spawning.animal_cap, 15);
        assert_eq!(config.spawning.water_mob_cap, 5);
        // The Java code spawns every tick and rolls every candidate chunk.
        assert_eq!(config.spawning.spawn_interval_ticks, 1);
        assert_eq!(config.spawning.chunk_sample_divisor, 1);
        assert_eq!(config.spawning.spawn_radius_chunks, 8);
        assert_eq!(config.spawning.min_player_distance, 24.0);
        assert_eq!(config.spawning.min_spawn_distance, 24.0);
    }

    #[test]
    fn partial_mob_override_keeps_the_java_defaults() {
        let text = r#"
[spawning.mobs.zombie]
weight = 25

[spawning.mobs.sheep]
enabled = false
"#;
        let config: Config = toml::from_str(text).unwrap();
        let mobs = &config.spawning.mobs;
        assert_eq!(mobs["zombie"].weight, 25);
        assert_eq!(mobs["zombie"].max_group, 4);
        assert_eq!(mobs["zombie"].biomes, vec!["*"]);
        assert!(!mobs["sheep"].enabled);
        assert_eq!(mobs["sheep"].weight, 12);
        // The mobs that are not mentioned keep their defaults.
        assert_eq!(mobs["cow"].weight, 8);
        assert_eq!(mobs["wolf"].max_group, 8);
    }

    #[test]
    fn unknown_mob_and_biome_are_rejected() {
        let text = "[spawning.mobs.zombi]\nweight = 5\n";
        let config: Config = toml::from_str(text).unwrap();
        assert!(config.validate().is_err());

        let text = "[spawning.mobs.zombie]\nbiomes = [\"swamp\"]\n";
        let config: Config = toml::from_str(text).unwrap();
        assert!(config.validate().is_err());

        let text = "[spawning.mobs.zombie]\nweight = \"ten\"\n";
        assert!(toml::from_str::<Config>(text).is_err());
    }

    #[test]
    fn the_saved_config_contains_every_mob() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        for name in MOB_SPAWN_NAMES {
            assert!(
                text.contains(&format!("[spawning.mobs.{name}]")),
                "{name} is missing in the saved config"
            );
        }
    }
}
