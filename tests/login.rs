use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

use flintcore::config::Config;
use flintcore::net::{listener, NetEvent};
use flintcore::server::Server;
use flintcore::util::bytes::Writer;

struct Harness {
    running: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    sim: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn start() -> Self {
        let config = Config::default();
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
            sim,
        }
    }

    async fn stop(self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.sim.await;
    }
}

async fn read_u8(stream: &mut TcpStream) -> u8 {
    let mut byte = [0u8; 1];
    stream.read_exact(&mut byte).await.unwrap();
    byte[0]
}

async fn read_i32(stream: &mut TcpStream) -> i32 {
    let mut bytes = [0u8; 4];
    stream.read_exact(&mut bytes).await.unwrap();
    i32::from_be_bytes(bytes)
}

async fn read_i64(stream: &mut TcpStream) -> i64 {
    let mut bytes = [0u8; 8];
    stream.read_exact(&mut bytes).await.unwrap();
    i64::from_be_bytes(bytes)
}

async fn read_i16(stream: &mut TcpStream) -> i16 {
    let mut bytes = [0u8; 2];
    stream.read_exact(&mut bytes).await.unwrap();
    i16::from_be_bytes(bytes)
}

async fn read_f64(stream: &mut TcpStream) -> f64 {
    let mut bytes = [0u8; 8];
    stream.read_exact(&mut bytes).await.unwrap();
    f64::from_be_bytes(bytes)
}

async fn read_f32(stream: &mut TcpStream) -> f32 {
    let mut bytes = [0u8; 4];
    stream.read_exact(&mut bytes).await.unwrap();
    f32::from_be_bytes(bytes)
}

async fn read_bool(stream: &mut TcpStream) -> bool {
    read_u8(stream).await != 0
}

async fn read_string(stream: &mut TcpStream) -> String {
    let mut length = [0u8; 2];
    stream.read_exact(&mut length).await.unwrap();
    let length = i16::from_be_bytes(length) as usize;
    let mut bytes = vec![0u8; length * 2];
    stream.read_exact(&mut bytes).await.unwrap();
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or('\u{FFFD}'))
        .collect()
}

async fn read_next_non_chunk_packet_id(stream: &mut TcpStream) -> u8 {
    loop {
        let id = read_u8(stream).await;
        if id == 0 {
            continue;
        } else if id == 50 {
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_u8(stream).await,
            );
            continue;
        } else if id == 51 {
            let _ = (
                read_i32(stream).await,
                read_i16(stream).await,
                read_i32(stream).await,
            );
            let _ = (
                read_u8(stream).await,
                read_u8(stream).await,
                read_u8(stream).await,
            );
            let len = read_i32(stream).await as usize;
            let mut comp = vec![0u8; len];
            stream.read_exact(&mut comp).await.unwrap();
            continue;
        } else {
            return id;
        }
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_completes_login() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        client.write_all(&handshake("Player")).await.unwrap();
        client.write_all(&login(14, "Player")).await.unwrap();

        let handshake_reply = read_u8(&mut client).await;
        assert_eq!(handshake_reply, 2);
        assert_eq!(read_string(&mut client).await, "-");

        let login_reply = read_u8(&mut client).await;
        assert_eq!(login_reply, 1);
        let entity_id = read_i32(&mut client).await;
        assert!(entity_id > 0);
        assert_eq!(read_string(&mut client).await, "");
        assert_eq!(read_i64(&mut client).await, 0);
        assert_eq!(read_u8(&mut client).await, 0);

        let spawn = read_u8(&mut client).await;
        assert_eq!(spawn, 6);
        let _ = (read_i32(&mut client).await, read_i32(&mut client).await);
        let _ = read_i32(&mut client).await;

        let update_time = read_u8(&mut client).await;
        assert_eq!(update_time, 4);
        let _ = read_i64(&mut client).await;
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_streams_chunks_and_teleports() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        client.write_all(&handshake("Player2")).await.unwrap();
        client.write_all(&login(14, "Player2")).await.unwrap();

        assert_eq!(read_u8(&mut client).await, 2);
        assert_eq!(read_string(&mut client).await, "-");

        assert_eq!(read_u8(&mut client).await, 1);
        let _entity_id = read_i32(&mut client).await;
        let _ = read_string(&mut client).await;
        let _ = read_i64(&mut client).await;
        let _ = read_u8(&mut client).await;

        assert_eq!(read_u8(&mut client).await, 6); // SpawnPosition
        let _ = (
            read_i32(&mut client).await,
            read_i32(&mut client).await,
            read_i32(&mut client).await,
        );

        assert_eq!(read_u8(&mut client).await, 4); // UpdateTime
        let _ = read_i64(&mut client).await;

        // Read 9 initial spawn chunks (PreChunk + MapChunk pairs)
        for _ in 0..9 {
            let pre_id = read_u8(&mut client).await;
            assert_eq!(pre_id, 50); // PreChunk
            let _cx = read_i32(&mut client).await;
            let _cz = read_i32(&mut client).await;
            let mode = read_bool(&mut client).await;
            assert!(mode);

            let map_id = read_u8(&mut client).await;
            assert_eq!(map_id, 51); // MapChunk
            let _x = read_i32(&mut client).await;
            let _y = read_i16(&mut client).await;
            let _z = read_i32(&mut client).await;
            let size_x = read_u8(&mut client).await;
            let size_y = read_u8(&mut client).await;
            let size_z = read_u8(&mut client).await;
            assert_eq!(size_x, 15);
            assert_eq!(size_y, 127);
            assert_eq!(size_z, 15);

            let compressed_len = read_i32(&mut client).await as usize;
            assert!(compressed_len > 0);
            let mut compressed = vec![0u8; compressed_len];
            client.read_exact(&mut compressed).await.unwrap();

            let payload = flintcore::util::compress::inflate_zlib(&compressed, 81920).unwrap();
            assert_eq!(payload.len(), 81920);
        }

        // Read PlayerLookMove spawn teleport
        let teleport_id = read_u8(&mut client).await;
        assert_eq!(teleport_id, 13); // PlayerLookMove
        let x = read_f64(&mut client).await;
        let y = read_f64(&mut client).await;
        let stance = read_f64(&mut client).await;
        let z = read_f64(&mut client).await;
        let _yaw = read_f32(&mut client).await;
        let _pitch = read_f32(&mut client).await;
        let on_ground = read_bool(&mut client).await;

        assert_eq!(x, 0.5);
        assert!((y - (stance + 1.62)).abs() < 1e-4);
        assert!(stance >= 64.0);
        assert_eq!(z, 0.5);
        assert!(!on_ground);

        // Send a player movement packet back
        let mut move_writer = Writer::new();
        move_writer.write_u8(11); // PlayerPosition
        move_writer.write_f64(0.5);
        move_writer.write_f64(stance);
        move_writer.write_f64(y);
        move_writer.write_f64(0.5);
        move_writer.write_bool(true);
        client.write_all(move_writer.as_slice()).await.unwrap();
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_interacts_and_does_not_disconnect() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        client.write_all(&handshake("Player3")).await.unwrap();
        client.write_all(&login(14, "Player3")).await.unwrap();

        assert_eq!(read_u8(&mut client).await, 2);
        assert_eq!(read_string(&mut client).await, "-");

        assert_eq!(read_u8(&mut client).await, 1);
        let _ = read_i32(&mut client).await;
        let _ = read_string(&mut client).await;
        let _ = read_i64(&mut client).await;
        let _ = read_u8(&mut client).await;

        assert_eq!(read_u8(&mut client).await, 6); // SpawnPosition
        let _ = (
            read_i32(&mut client).await,
            read_i32(&mut client).await,
            read_i32(&mut client).await,
        );

        assert_eq!(read_u8(&mut client).await, 4); // UpdateTime
        let _ = read_i64(&mut client).await;

        // Consume the 9 initial chunks
        for _ in 0..9 {
            assert_eq!(read_u8(&mut client).await, 50); // PreChunk
            let _ = (
                read_i32(&mut client).await,
                read_i32(&mut client).await,
                read_bool(&mut client).await,
            );

            assert_eq!(read_u8(&mut client).await, 51); // MapChunk
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
        }

        // Consume PlayerLookMove spawn teleport
        assert_eq!(read_u8(&mut client).await, 13);
        let _ = (
            read_f64(&mut client).await,
            read_f64(&mut client).await,
            read_f64(&mut client).await,
            read_f64(&mut client).await,
        );
        let _ = (
            read_f32(&mut client).await,
            read_f32(&mut client).await,
            read_bool(&mut client).await,
        );

        // 1. Send ArmAnimation (id 18) - the packet that caused disconnection before
        let mut anim_writer = Writer::new();
        anim_writer.write_u8(18); // ArmAnimation
        anim_writer.write_i32(1); // entity_id
        anim_writer.write_u8(1); // swing
        client.write_all(anim_writer.as_slice()).await.unwrap();

        // 2. Send BlockItemSwitch (id 16) - change hotbar
        let mut slot_writer = Writer::new();
        slot_writer.write_u8(16);
        slot_writer.write_i16(3); // slot 3
        client.write_all(slot_writer.as_slice()).await.unwrap();

        // 3. Send EntityAction (id 19) - toggle sneak
        let mut action_writer = Writer::new();
        action_writer.write_u8(19);
        action_writer.write_i32(1);
        action_writer.write_u8(1); // sneak
        client.write_all(action_writer.as_slice()).await.unwrap();

        // 4. Send Place (id 15) - place stone block
        let mut place_writer = Writer::new();
        place_writer.write_u8(15);
        place_writer.write_i32(0);
        place_writer.write_u8(86);
        place_writer.write_i32(0);
        place_writer.write_u8(1); // face 1 (+Y)
        place_writer.write_i16(1); // item stone
        place_writer.write_u8(64);
        place_writer.write_i16(0);
        client.write_all(place_writer.as_slice()).await.unwrap();

        // Expect BlockChange (id 53) reply for placement
        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 53);
        assert_eq!(read_i32(&mut client).await, 0);
        assert_eq!(read_u8(&mut client).await, 87); // placed at 86 + 1
        assert_eq!(read_i32(&mut client).await, 0);
        assert_eq!(read_u8(&mut client).await, 1); // stone
        assert_eq!(read_u8(&mut client).await, 0); // meta

        // 5. Send BlockDig (id 14) - break the stone block
        let mut dig_writer = Writer::new();
        dig_writer.write_u8(14);
        dig_writer.write_u8(2); // status 2 = finished digging
        dig_writer.write_i32(0);
        dig_writer.write_u8(87);
        dig_writer.write_i32(0);
        dig_writer.write_u8(1); // face
        client.write_all(dig_writer.as_slice()).await.unwrap();

        // Expect BlockChange (id 53) reply for breaking
        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 53);
        assert_eq!(read_i32(&mut client).await, 0);
        assert_eq!(read_u8(&mut client).await, 87);
        assert_eq!(read_i32(&mut client).await, 0);
        assert_eq!(read_u8(&mut client).await, 0); // air

        // 6. Send Chat (id 3)
        let mut chat_writer = Writer::new();
        chat_writer.write_u8(3);
        chat_writer.write_string("hello flintcore", 100).unwrap();
        client.write_all(chat_writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 3);
        let chat_msg = read_string(&mut client).await;
        assert_eq!(chat_msg, "<Player3> hello flintcore");

        // 7. Clean disconnect (id 255)
        let mut dc_writer = Writer::new();
        dc_writer.write_u8(255);
        dc_writer.write_string("Quitting", 100).unwrap();
        client.write_all(dc_writer.as_slice()).await.unwrap();
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outdated_client_is_kicked() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        client.write_all(&handshake("Player")).await.unwrap();
        client.write_all(&login(13, "Player")).await.unwrap();

        assert_eq!(read_u8(&mut client).await, 2);
        assert_eq!(read_string(&mut client).await, "-");

        assert_eq!(read_u8(&mut client).await, 255);
        let reason = read_string(&mut client).await;
        assert_eq!(reason, "Outdated client! Please use Beta 1.7.3");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}
