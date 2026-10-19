use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::util::{Error, Result};

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
            autosave_interval_ticks: 6000,
            time_speed_multiplier: 1,
            enable_tps_command: true,
            join_message: "\u{00A7}e%player% joined the game.".to_string(),
            leave_message: "\u{00A7}e%player% left the game.".to_string(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
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

/// Natural mob spawning and dungeon mob spawner limits (`server.properties` + `Poseidon` + `Uberbukkit`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
}
