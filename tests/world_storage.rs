use std::fs;
use std::path::PathBuf;

use flintcore::config::WorldConfig;
use flintcore::entity::player::Player;
use flintcore::gameplay::items::ItemStack;
use flintcore::gameplay::tile::{TileEntity, TileEntityKind};
use flintcore::world::chunk::{Chunk, ChunkPos, ChunkState};
use flintcore::world::storage::{
    ChunkLoader, LevelData, PlayerStore, RegionFileCache, SessionLock,
};
use flintcore::world::world::World;

fn create_temp_world_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "flintcore_{prefix}_{}_{}",
        std::process::id(),
        rand_u64()
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[test]
fn test_level_dat_persistence_and_atomic_swap() {
    let world_dir = create_temp_world_dir("level_test");

    let mut level = LevelData {
        seed: -9876543210,
        spawn_x: 42,
        spawn_y: 70,
        spawn_z: -84,
        spawn_yaw: 45.0,
        spawn_pitch: 10.0,
        time: 12345,
        level_name: "TestLevel".into(),
        ..LevelData::default()
    };

    level.save_to_dir(&world_dir).unwrap();
    assert!(world_dir.join("level.dat").exists());

    // Update level data and save again (triggers atomic swap to level.dat_old)
    level.time = 12500;
    level.save_to_dir(&world_dir).unwrap();
    assert!(world_dir.join("level.dat").exists());
    assert!(world_dir.join("level.dat_old").exists());

    let loaded = LevelData::load_from_dir(&world_dir).unwrap().unwrap();
    assert_eq!(loaded.seed, -9876543210);
    assert_eq!(loaded.spawn_x, 42);
    assert_eq!(loaded.spawn_y, 70);
    assert_eq!(loaded.spawn_z, -84);
    assert_eq!(loaded.spawn_yaw, 45.0);
    assert_eq!(loaded.spawn_pitch, 10.0);
    assert_eq!(loaded.time, 12500);
    assert_eq!(loaded.level_name, "TestLevel");

    let _ = fs::remove_dir_all(&world_dir);
}

#[test]
fn test_session_lock_guard() {
    let world_dir = create_temp_world_dir("session_test");

    let lock1 = SessionLock::acquire(&world_dir).unwrap();
    assert!(lock1.check().is_ok());

    std::thread::sleep(std::time::Duration::from_millis(10));

    // Second acquire overwrites the file timestamp
    let lock2 = SessionLock::acquire(&world_dir).unwrap();
    assert!(lock2.check().is_ok());

    // Original lock now fails check because timestamp on disk was changed
    assert!(lock1.check().is_err());

    let _ = fs::remove_dir_all(&world_dir);
}

#[test]
fn test_player_store_persistence() {
    let world_dir = create_temp_world_dir("player_test");
    let store = PlayerStore::new(&world_dir);

    let mut p1 = Player::new(1, 1, "MinerBob".into(), 50.5, 68.0, -100.5);
    p1.health = 14;
    p1.inventory.items[0] = Some(ItemStack::new(278, 1, 45)); // Diamond pickaxe with damage
    p1.inventory.items[1] = Some(ItemStack::new(50, 64, 0)); // 64 Torches
    p1.inventory.armor[2] = Some(ItemStack::new(311, 1, 10)); // Diamond chestplate

    store.save_player(&p1).unwrap();
    assert!(store.player_file("MinerBob").exists());

    // Restore into a fresh player instance
    let mut p2 = Player::new(2, 2, "MinerBob".into(), 0.0, 64.0, 0.0);
    let loaded = store.load_player(&mut p2).unwrap();
    assert!(loaded);
    assert_eq!(p2.x, 50.5);
    assert_eq!(p2.y, 68.0);
    assert_eq!(p2.z, -100.5);
    assert_eq!(p2.health, 14);
    assert_eq!(p2.inventory.items[0], Some(ItemStack::new(278, 1, 45)));
    assert_eq!(p2.inventory.items[1], Some(ItemStack::new(50, 64, 0)));
    assert_eq!(p2.inventory.armor[2], Some(ItemStack::new(311, 1, 10)));

    let _ = fs::remove_dir_all(&world_dir);
}

#[test]
fn test_mcregion_chunk_and_tile_entity_round_trip() {
    let world_dir = create_temp_world_dir("mcregion_test");
    let mut cache = RegionFileCache::new(&world_dir);

    let pos = ChunkPos::new(3, -2);
    let mut chunk = Chunk::new(pos);
    chunk.state = ChunkState::Populated;
    chunk.set_block(5, 64, 5, 54); // Chest
    chunk.set_metadata(5, 64, 5, 2);
    chunk.set_block(5, 65, 5, 63); // Sign post
    chunk.set_tile_entity(TileEntity::new_sign(
        3 * 16 + 5,
        65,
        -2 * 16 + 5,
        [
            "Welcome to".into(),
            "Poseidon Server".into(),
            "Beta 1.7.3".into(),
            "".into(),
        ],
    ));

    let nbt = ChunkLoader::chunk_to_nbt(&chunk, 5000);
    cache.write_chunk(pos.x, pos.z, &nbt).unwrap();
    cache.flush_all().unwrap();

    let region_file = world_dir.join("region").join("r.0.-1.mcr");
    assert!(region_file.exists());

    // Read back
    let read_nbt = cache.read_chunk(pos.x, pos.z).unwrap().unwrap();
    let loaded_chunk = ChunkLoader::chunk_from_nbt(&read_nbt).unwrap();

    assert_eq!(loaded_chunk.pos, pos);
    assert_eq!(loaded_chunk.state, ChunkState::Populated);
    assert_eq!(loaded_chunk.get_block(5, 64, 5), 54);
    assert_eq!(loaded_chunk.get_metadata(5, 64, 5), 2);
    assert_eq!(loaded_chunk.get_block(5, 65, 5), 63);
    assert_eq!(loaded_chunk.tile_entities.len(), 1);

    if let TileEntityKind::Sign { lines } = &loaded_chunk.tile_entities[0].kind {
        assert_eq!(lines[0], "Welcome to");
        assert_eq!(lines[1], "Poseidon Server");
        assert_eq!(lines[2], "Beta 1.7.3");
    } else {
        panic!("Expected sign tile entity");
    }

    let _ = fs::remove_dir_all(&world_dir);
}

#[test]
fn test_world_disk_save_and_reopen_lifecycle() {
    let world_dir = create_temp_world_dir("world_lifecycle");
    let level_name = world_dir.to_str().unwrap().to_string();

    // 1. First server session: generate world and place blocks
    {
        let config = WorldConfig {
            seed: 555666777,
            level_name: level_name.clone(),
            save_enabled: true,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        let pos = ChunkPos::new(0, 0);
        world.populate_chunk(pos);

        // Place obsidian block at (5, 75, 5)
        world.set_block(5, 75, 5, 49); // Obsidian
        assert_eq!(world.get_block(5, 75, 5), 49);

        world.save_all().unwrap();
    }

    assert!(world_dir.join("level.dat").exists());
    assert!(world_dir.join("region").join("r.0.0.mcr").exists());

    // 2. Second server session: reopen world from disk
    {
        let config = WorldConfig {
            seed: 0, // Should be overridden by level.dat
            level_name: level_name.clone(),
            save_enabled: true,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        assert_eq!(world.seed(), 555666777); // Seed recovered from level.dat!

        let pos = ChunkPos::new(0, 0);
        world.ensure_chunk_base(pos);

        // Verify obsidian block persisted across server restart!
        assert_eq!(world.get_block(5, 75, 5), 49);
    }

    let _ = fs::remove_dir_all(&world_dir);
}
