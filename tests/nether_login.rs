//! End-to-end login test for a player whose saved `Dimension` is the Nether.
//!
//! The player file is written before the server starts, so the login must
//! pick the Nether up from the player NBT: the login packet carries the
//! Nether dimension byte, the spawn position is the Nether spawn and the
//! streamed chunks are Nether terrain.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

use flintcore::config::Config;
use flintcore::entity::player::Player;
use flintcore::net::{listener, NetEvent};
use flintcore::server::Server;
use flintcore::util::bytes::Writer;
use flintcore::world::storage::PlayerStore;

struct Harness {
    running: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    sim: Option<tokio::task::JoinHandle<()>>,
    world_dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = fs::remove_dir_all(&self.world_dir);
    }
}

impl Harness {
    async fn start(world_dir: PathBuf) -> Self {
        let mut config = Config::default();
        config.world.level_name = world_dir.to_string_lossy().to_string();
        config.world.save_enabled = true;
        // Keep the startup cheap: no spawn search and no spawn-region
        // preparation, the login test only needs the player file.
        config.world.auto_find_spawn = false;
        config.world.keep_spawn_in_memory = false;
        config.world.view_distance = 2;
        config.server.allow_nether = true;

        let running = Arc::new(AtomicBool::new(true));
        let next_id = Arc::new(AtomicU64::new(1));
        let (inbound_tx, inbound_rx) = mpsc::channel::<NetEvent>(64);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(listener::run(
            listener,
            inbound_tx,
            next_id,
            Arc::new(config.clone()),
            Arc::clone(&running),
        ));

        let server = Server::new(config, inbound_rx, Arc::clone(&running));
        let sim = tokio::task::spawn_blocking(move || server.run().unwrap());

        Self {
            running,
            address,
            sim: Some(sim),
            world_dir,
        }
    }

    async fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(sim) = self.sim.take() {
            let _ = sim.await;
        }
    }
}

fn temp_world_dir() -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("flintcore_nether_login_{nanos}"));
    let _ = fs::create_dir_all(&dir);
    dir
}

async fn read_u8(stream: &mut TcpStream) -> u8 {
    let mut byte = [0u8; 1];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut byte)).await {
        Ok(Ok(_)) => byte[0],
        _ => panic!("read_u8 timeout"),
    }
}

async fn read_i32(stream: &mut TcpStream) -> i32 {
    let mut bytes = [0u8; 4];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i32::from_be_bytes(bytes),
        _ => panic!("read_i32 timeout"),
    }
}

async fn read_i16(stream: &mut TcpStream) -> i16 {
    let mut bytes = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i16::from_be_bytes(bytes),
        _ => panic!("read_i16 timeout"),
    }
}

async fn read_i64(stream: &mut TcpStream) -> i64 {
    let mut bytes = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i64::from_be_bytes(bytes),
        _ => panic!("read_i64 timeout"),
    }
}

async fn read_f64(stream: &mut TcpStream) -> f64 {
    let mut bytes = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => f64::from_be_bytes(bytes),
        _ => panic!("read_f64 timeout"),
    }
}

async fn read_f32(stream: &mut TcpStream) -> f32 {
    let mut bytes = [0u8; 4];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => f32::from_be_bytes(bytes),
        _ => panic!("read_f32 timeout"),
    }
}

async fn read_bool(stream: &mut TcpStream) -> bool {
    read_u8(stream).await != 0
}

async fn read_string(stream: &mut TcpStream) -> String {
    let mut length = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut length)).await {
        Ok(Ok(_)) => {}
        _ => panic!("read_string length timeout"),
    }
    let length = i16::from_be_bytes(length) as usize;
    let mut bytes = vec![0u8; length * 2];
    match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => {}
        _ => panic!("read_string body timeout"),
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or('\u{FFFD}'))
        .collect()
}

fn handshake(username: &str) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.write_u8(2);
    writer.write_string(username, 32).unwrap();
    writer.into_inner()
}

fn login(protocol: i32, username: &str) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.write_u8(1);
    writer.write_i32(protocol);
    writer.write_string(username, 16).unwrap();
    writer.write_i64(0);
    writer.write_i8(0);
    writer.into_inner()
}

/// Writes a player file whose `Dimension` is the Nether.
fn write_nether_player(world_dir: &Path, username: &str) {
    let store = PlayerStore::new(world_dir);
    let mut player = Player::new(1, 1, username.to_string(), 8.5, 64.0, 8.5);
    player.dimension = -1;
    store.save_player(&player).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_saved_in_nether_logs_into_the_nether() {
    let world_dir = temp_world_dir();
    write_nether_player(&world_dir, "NetherMan");
    let harness = Harness::start(world_dir).await;

    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        client.write_all(&handshake("NetherMan")).await.unwrap();
        client.write_all(&login(14, "NetherMan")).await.unwrap();

        // Handshake reply
        assert_eq!(read_u8(&mut client).await, 2);
        assert_eq!(read_string(&mut client).await, "-");

        // Login packet: entity id, username, seed, dimension byte
        assert_eq!(read_u8(&mut client).await, 1);
        let _entity_id = read_i32(&mut client).await;
        let _ = read_string(&mut client).await;
        let _seed = read_i64(&mut client).await;
        let dimension = read_u8(&mut client).await as i8;
        assert_eq!(dimension, -1, "login must report the Nether dimension");

        // Spawn position: the Nether spawn is (0, 64, 0)
        assert_eq!(read_u8(&mut client).await, 6);
        let spawn_x = read_i32(&mut client).await;
        let spawn_y = read_i32(&mut client).await;
        let spawn_z = read_i32(&mut client).await;
        assert_eq!((spawn_x, spawn_y, spawn_z), (0, 64, 0));

        // Update time
        assert_eq!(read_u8(&mut client).await, 4);
        let _ = read_i64(&mut client).await;

        // The spawn area is streamed from the Nether: collect the chunk
        // payloads and check that they are Nether terrain (bedrock at the
        // bottom, netherrack above it) instead of overworld stone/dirt.
        let mut payloads = Vec::new();
        let mut teleported = false;
        while !teleported {
            let id = read_u8(&mut client).await;
            match id {
                50 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_bool(&mut client).await,
                    );
                }
                51 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i16(&mut client).await,
                        read_i32(&mut client).await,
                    );
                    let _ = (
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                    let len = read_i32(&mut client).await as usize;
                    let mut comp = vec![0u8; len];
                    client.read_exact(&mut comp).await.unwrap();
                    let payload = flintcore::util::compress::inflate_zlib(&comp, 81920).unwrap();
                    payloads.push(payload);
                }
                13 => {
                    let x = read_f64(&mut client).await;
                    let y = read_f64(&mut client).await;
                    let stance = read_f64(&mut client).await;
                    let z = read_f64(&mut client).await;
                    let _ = (read_f32(&mut client).await, read_f32(&mut client).await);
                    let _on_ground = read_bool(&mut client).await;
                    assert_eq!(x, 8.5);
                    assert_eq!(z, 8.5);
                    assert!((y - (stance + 1.62)).abs() < 1e-4);
                    teleported = true;
                }
                _ => {}
            }
        }
        assert!(!payloads.is_empty(), "no chunk was streamed");

        // Wire layout: (x << 11) | (z << 7) | y
        let wire_index = |x: usize, y: usize, z: usize| (x << 11) | (z << 7) | y;
        let first = &payloads[0];
        // Bedrock floor (7) and netherrack (87) somewhere in the column.
        let column: Vec<u8> = (0..128).map(|y| first[wire_index(0, y, 0)]).collect();
        assert!(
            column.contains(&7),
            "nether chunk must have a bedrock floor"
        );
        assert!(column.contains(&87), "nether chunk must contain netherrack");
        assert!(
            !column.contains(&2) && !column.contains(&3),
            "nether chunk must not contain overworld grass/dirt"
        );
    };

    timeout(Duration::from_secs(60), exchange).await.unwrap();
    harness.stop().await;
}
