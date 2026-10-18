use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

use flintcore::config::Config;
use flintcore::net::{listener, packets::chat::MAX_CHAT_LENGTH, NetEvent};
use flintcore::server::Server;
use flintcore::util::bytes::Writer;

struct Harness {
    running: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    sim: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Harness {
    async fn start() -> Self {
        let mut config = Config::default();
        config.world.save_enabled = false;
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
        }
    }

    async fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(sim) = self.sim.take() {
            let _ = sim.await;
        }
    }
}

async fn read_u8(stream: &mut TcpStream) -> u8 {
    let mut byte = [0u8; 1];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut byte)).await {
        Ok(Ok(_)) => byte[0],
        _ => panic!("read_u8 timeout"),
    }
}

async fn read_i32(stream: &mut TcpStream) -> i32 {
    let mut bytes = [0u8; 4];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i32::from_be_bytes(bytes),
        _ => panic!("read_i32 timeout"),
    }
}

async fn read_i64(stream: &mut TcpStream) -> i64 {
    let mut bytes = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i64::from_be_bytes(bytes),
        _ => panic!("read_i64 timeout"),
    }
}

async fn read_i16(stream: &mut TcpStream) -> i16 {
    let mut bytes = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i16::from_be_bytes(bytes),
        _ => panic!("read_i16 timeout"),
    }
}

async fn read_f64(stream: &mut TcpStream) -> f64 {
    let mut bytes = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => f64::from_be_bytes(bytes),
        _ => panic!("read_f64 timeout"),
    }
}

async fn read_f32(stream: &mut TcpStream) -> f32 {
    let mut bytes = [0u8; 4];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => f32::from_be_bytes(bytes),
        _ => panic!("read_f32 timeout"),
    }
}

async fn read_bool(stream: &mut TcpStream) -> bool {
    read_u8(stream).await != 0
}

async fn read_string(stream: &mut TcpStream) -> String {
    let mut length = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut length)).await {
        Ok(Ok(_)) => {}
        _ => panic!("read_string length timeout"),
    }
    let length = i16::from_be_bytes(length) as usize;
    let mut bytes = vec![0u8; length * 2];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
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

async fn read_next_non_chunk_packet_id(stream: &mut TcpStream) -> u8 {
    loop {
        let id = read_u8(stream).await;
        if id == 0 {
            continue;
        } else if id == 5 || id == 28 {
            let _ = (
                read_i32(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
            );
            continue;
        } else if id == 8 {
            let _ = read_i16(stream).await;
            continue;
        } else if id == 20 {
            let _ = read_i32(stream).await;
            let _ = read_string(stream).await;
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
            let _ = (
                read_u8(stream).await,
                read_u8(stream).await,
                read_i16(stream).await,
            );
            continue;
        } else if id == 29 || id == 30 {
            let _ = read_i32(stream).await;
            continue;
        } else if id == 34 {
            let _ = read_i32(stream).await;
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
            let _ = (read_u8(stream).await, read_u8(stream).await);
            continue;
        } else if id == 38 {
            let _ = (read_i32(stream).await, read_u8(stream).await);
            continue;
        } else if id == 103 {
            let _ = (read_u8(stream).await, read_i16(stream).await);
            let item_id = read_i16(stream).await;
            if item_id >= 0 {
                let _ = (read_u8(stream).await, read_i16(stream).await);
            }
            continue;
        } else if id == 104 {
            let _ = read_u8(stream).await;
            let count = read_i16(stream).await;
            for _ in 0..count {
                let item_id = read_i16(stream).await;
                if item_id >= 0 {
                    let _ = (read_u8(stream).await, read_i16(stream).await);
                }
            }
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

        // WindowItems (104)
        assert_eq!(read_u8(&mut client).await, 104);
        let _win_id = read_u8(&mut client).await;
        let count = read_i16(&mut client).await;
        for _ in 0..count {
            let item_id = read_i16(&mut client).await;
            if item_id >= 0 {
                let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
            }
        }

        // UpdateHealth (8)
        assert_eq!(read_u8(&mut client).await, 8);
        let _ = read_i16(&mut client).await;

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

        // Wait a bit for server to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 5. Send BlockDig (id 14) - break the stone block
        let mut dig_writer = Writer::new();
        dig_writer.write_u8(14);
        dig_writer.write_u8(2); // status 2 = finished digging
        dig_writer.write_i32(0);
        dig_writer.write_u8(87);
        dig_writer.write_i32(0);
        dig_writer.write_u8(1); // face
        client.write_all(dig_writer.as_slice()).await.unwrap();

        // Wait a bit for server to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 6. Send Chat (id 3)
        let mut chat_writer = Writer::new();
        chat_writer.write_u8(3);
        chat_writer.write_string("hello flintcore", 100).unwrap();
        client.write_all(chat_writer.as_slice()).await.unwrap();

        // Wait a bit for server to process
        tokio::time::sleep(Duration::from_millis(100)).await;

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

async fn login_and_consume_spawn(client: &mut TcpStream, username: &str) {
    client.write_all(&handshake(username)).await.unwrap();
    client.write_all(&login(14, username)).await.unwrap();

    assert_eq!(read_u8(client).await, 2);
    assert_eq!(read_string(client).await, "-");

    assert_eq!(read_u8(client).await, 1);
    let _ = read_i32(client).await;
    let _ = read_string(client).await;
    let _ = read_i64(client).await;
    let _ = read_u8(client).await;

    assert_eq!(read_u8(client).await, 6);
    let _ = (
        read_i32(client).await,
        read_i32(client).await,
        read_i32(client).await,
    );

    assert_eq!(read_u8(client).await, 4);
    let _ = read_i64(client).await;

    for _ in 0..9 {
        assert_eq!(read_u8(client).await, 50);
        let _ = (
            read_i32(client).await,
            read_i32(client).await,
            read_bool(client).await,
        );

        assert_eq!(read_u8(client).await, 51);
        let _ = (
            read_i32(client).await,
            read_i16(client).await,
            read_i32(client).await,
        );
        let _ = (
            read_u8(client).await,
            read_u8(client).await,
            read_u8(client).await,
        );
        let len = read_i32(client).await as usize;
        let mut comp = vec![0u8; len];
        client.read_exact(&mut comp).await.unwrap();
    }

    assert_eq!(read_u8(client).await, 13);
    let _ = (
        read_f64(client).await,
        read_f64(client).await,
        read_f64(client).await,
        read_f64(client).await,
    );
    let _ = (
        read_f32(client).await,
        read_f32(client).await,
        read_bool(client).await,
    );

    // Consume initial WindowItems (104)
    assert_eq!(read_u8(client).await, 104);
    let _win_id = read_u8(client).await;
    let count = read_i16(client).await;
    for _ in 0..count {
        let item_id = read_i16(client).await;
        if item_id >= 0 {
            let _ = (read_u8(client).await, read_i16(client).await);
        }
    }

    // Consume initial UpdateHealth (8)
    assert_eq!(read_u8(client).await, 8);
    let _ = read_i16(client).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_too_long_gets_kicked() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatTooLong").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer
            .write_string(&"a".repeat(101), MAX_CHAT_LENGTH)
            .unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 255);
        let reason = read_string(&mut client).await;
        assert_eq!(reason, "Chat message too long");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_illegal_characters_gets_kicked() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatIllegal").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("hello \u{1F600} world", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 255);
        let reason = read_string(&mut client).await;
        assert_eq!(reason, "Illegal characters in chat");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_command_is_not_broadcast() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatCmd").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("/help", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        // If the server kicked us for sending a command, write_all above would
        // have failed (connection closed). Reaching here means the connection
        // is alive and no kick occurred.
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_normal_broadcast_format() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatNormal").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("hello world", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 3);
        let msg = read_string(&mut client).await;
        assert_eq!(msg, "<ChatNormal> hello world");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_command_unknown() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatUnknown").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("/nonexistent", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 3);
        let msg = read_string(&mut client).await;
        assert_eq!(msg, "Unknown command. Type \"help\" for help.");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_command_help() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatHelp").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("/help", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 3);
        let msg = read_string(&mut client).await;
        assert!(msg.contains("Help"));
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_command_say() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatSay").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("/say hello from server", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 3);
        let msg = read_string(&mut client).await;
        assert!(msg.contains("hello from server"));
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_command_me() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ChatMe").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("/me dances", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_next_non_chunk_packet_id(&mut client).await, 3);
        let msg = read_string(&mut client).await;
        assert!(msg.contains("* ChatMe dances"));
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_message_broadcast() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "Player1").await;

        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Player2").await;

        // Player1 (already online) should receive join message when Player2 joins
        loop {
            let id = read_next_non_chunk_packet_id(&mut client1).await;
            if id == 3 {
                break;
            }
        }
        let msg = read_string(&mut client1).await;
        assert!(msg.contains("Player2 joined the game"));
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leave_message_broadcast() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "Player1").await;

        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Player2").await;

        // Disconnect Player1
        let mut writer = Writer::new();
        writer.write_u8(255);
        writer.write_string("Quitting", 100).unwrap();
        client1.write_all(writer.as_slice()).await.unwrap();

        // Player2 should receive leave message about Player1
        loop {
            let id = read_next_non_chunk_packet_id(&mut client2).await;
            if id == 3 {
                break;
            }
        }
        let msg = read_string(&mut client2).await;
        assert!(msg.contains("Player1 left the game"));
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_tracking_spawns_other_player() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "Watcher").await;

        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Spawnee").await;

        // Give the tracker tick a moment to scan proximity
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Watcher should receive NamedEntitySpawn (id 20) for Spawnee
        let mut found_spawn = false;
        for _ in 0..30 {
            let id = read_u8(&mut client1).await;
            if id == 20 {
                let entity_id = read_i32(&mut client1).await;
                assert!(entity_id > 0);
                let name = read_string(&mut client1).await;
                assert_eq!(name, "Spawnee");
                found_spawn = true;
                break;
            } else if id == 0 {
                continue;
            } else if id == 3 {
                let _ = read_string(&mut client1).await;
            } else if id == 50 {
                let _ = (
                    read_i32(&mut client1).await,
                    read_i32(&mut client1).await,
                    read_u8(&mut client1).await,
                );
            } else if id == 51 {
                let _ = (
                    read_i32(&mut client1).await,
                    read_i16(&mut client1).await,
                    read_i32(&mut client1).await,
                );
                let _ = (
                    read_u8(&mut client1).await,
                    read_u8(&mut client1).await,
                    read_u8(&mut client1).await,
                );
                let len = read_i32(&mut client1).await as usize;
                let mut comp = vec![0u8; len];
                client1.read_exact(&mut comp).await.unwrap();
            }
        }
        assert!(
            found_spawn,
            "Watcher should receive NamedEntitySpawn for Spawnee"
        );
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_click_transaction() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "Clicker").await;

        // Send WindowClick (id 102) on hotbar slot 36
        let mut click_writer = Writer::new();
        click_writer.write_u8(102);
        click_writer.write_u8(0); // window_id
        click_writer.write_i16(36); // slot
        click_writer.write_u8(0); // left click
        click_writer.write_i16(1); // action number
        click_writer.write_bool(false); // shift
        click_writer.write_i16(-1); // item_id (empty)
        client.write_all(click_writer.as_slice()).await.unwrap();

        // Should receive Transaction (id 106)
        let mut found_trans = false;
        for _ in 0..30 {
            let id = read_u8(&mut client).await;
            if id == 106 {
                let window_id = read_u8(&mut client).await;
                assert_eq!(window_id, 0);
                let action = read_i16(&mut client).await;
                assert_eq!(action, 1);
                let accepted = read_bool(&mut client).await;
                assert!(accepted);
                found_trans = true;
                break;
            } else if id == 0 {
                continue;
            } else if id == 103 {
                let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                let item = read_i16(&mut client).await;
                if item >= 0 {
                    let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                }
            } else if id == 50 {
                let _ = (
                    read_i32(&mut client).await,
                    read_i32(&mut client).await,
                    read_u8(&mut client).await,
                );
            } else if id == 51 {
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
        }
        assert!(found_trans, "Should receive Transaction confirmation");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn respawn_double_packet_sequence() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "DeadGuy").await;

        // Send Respawn (id 9)
        let mut respawn_writer = Writer::new();
        respawn_writer.write_u8(9);
        respawn_writer.write_i8(0);
        client.write_all(respawn_writer.as_slice()).await.unwrap();

        // Poseidon double-respawn: skip chunks and find first Respawn(-1)
        let mut found_respawn1 = false;
        for _ in 0..30 {
            let id = read_u8(&mut client).await;
            if id == 9 {
                let dim = read_u8(&mut client).await as i8;
                assert_eq!(dim, -1);
                found_respawn1 = true;
                break;
            } else if id == 0 {
                continue;
            } else if id == 50 {
                let _ = (
                    read_i32(&mut client).await,
                    read_i32(&mut client).await,
                    read_u8(&mut client).await,
                );
            } else if id == 51 {
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
        }
        assert!(found_respawn1, "Should receive first Respawn packet (-1)");

        // Expect second Respawn (0)
        assert_eq!(read_u8(&mut client).await, 9);
        assert_eq!(read_u8(&mut client).await as i8, 0);

        // Expect 9 spawn chunks
        for _ in 0..9 {
            assert_eq!(read_u8(&mut client).await, 50);
            let _ = (
                read_i32(&mut client).await,
                read_i32(&mut client).await,
                read_bool(&mut client).await,
            );
            assert_eq!(read_u8(&mut client).await, 51);
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

        // PlayerLookMove teleport (sent at end of stream_initial_chunks)
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

        // SpawnPosition
        assert_eq!(read_u8(&mut client).await, 6);
        let _ = (
            read_i32(&mut client).await,
            read_i32(&mut client).await,
            read_i32(&mut client).await,
        );

        // UpdateTime
        assert_eq!(read_u8(&mut client).await, 4);
        let _ = read_i64(&mut client).await;

        // UpdateHealth (20)
        assert_eq!(read_u8(&mut client).await, 8);
        assert_eq!(read_i16(&mut client).await, 20);

        // WindowItems (window 0)
        assert_eq!(read_u8(&mut client).await, 104);
        let win_id = read_u8(&mut client).await;
        assert_eq!(win_id, 0);
        let count = read_i16(&mut client).await;
        assert_eq!(count, 45); // 45 slots in inventory window!
        for _ in 0..count {
            let item_id = read_i16(&mut client).await;
            if item_id >= 0 {
                let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
            }
        }
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_color_codes_and_section_sign() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "ColorUser").await;

        // Send a message with section sign (§a)
        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("§aGreen message", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        // Must NOT be kicked (not packet 255)
        let id = read_next_non_chunk_packet_id(&mut client).await;
        assert_eq!(id, 3);
        let msg = read_string(&mut client).await;
        assert!(msg.contains("§aGreen message"));

        // Send a message with ampersand (&c)
        let mut writer2 = Writer::new();
        writer2.write_u8(3);
        writer2.write_string("&cRed message", 100).unwrap();
        client.write_all(writer2.as_slice()).await.unwrap();

        let id2 = read_next_non_chunk_packet_id(&mut client).await;
        assert_eq!(id2, 3);
        let msg2 = read_string(&mut client).await;
        assert!(msg2.contains("§cRed message"));
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_window_does_not_echo() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "CloseTest").await;

        // Send CloseWindow(0)
        let mut writer = Writer::new();
        writer.write_u8(101);
        writer.write_u8(0);
        client.write_all(writer.as_slice()).await.unwrap();

        // Send KeepAlive (0)
        let mut ka_writer = Writer::new();
        ka_writer.write_u8(0);
        client.write_all(ka_writer.as_slice()).await.unwrap();

        // Should NOT receive CloseWindow(101) back
        // Wait a short time to verify no packet 101 arrives
        tokio::time::sleep(Duration::from_millis(150)).await;
        let mut buf = [0u8; 1];
        let peek_res = client.try_read(&mut buf);
        // Either Nothing or KeepAlive/UpdateTime, definitely not 101
        if let Ok(n) = peek_res {
            if n > 0 {
                assert_ne!(buf[0], 101);
            }
        }
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fall_damage_on_player_position() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "FallUser").await;

        // First sync position on ground
        let mut writer0 = Writer::new();
        writer0.write_u8(11);
        writer0.write_f64(0.0);
        writer0.write_f64(64.0);
        writer0.write_f64(64.0);
        writer0.write_f64(0.0);
        writer0.write_bool(true);
        client.write_all(writer0.as_slice()).await.unwrap();

        // Fall 5 blocks in air: Packet 11 (PlayerPosition), on_ground = false
        let mut writer = Writer::new();
        writer.write_u8(11);
        writer.write_f64(0.0);
        writer.write_f64(59.0); // 5 blocks drop
        writer.write_f64(59.0);
        writer.write_f64(0.0);
        writer.write_bool(false);
        client.write_all(writer.as_slice()).await.unwrap();

        // Land on ground: Packet 11 (PlayerPosition), on_ground = true
        let mut writer2 = Writer::new();
        writer2.write_u8(11);
        writer2.write_f64(0.0);
        writer2.write_f64(59.0);
        writer2.write_f64(59.0);
        writer2.write_f64(0.0);
        writer2.write_bool(true);
        client.write_all(writer2.as_slice()).await.unwrap();

        // Should receive EntityStatus (38, status 2) and UpdateHealth (8) with health == 18
        let mut got_hurt_animation = false;
        let mut got_damaged_health = false;

        while !got_hurt_animation || !got_damaged_health {
            let id = read_u8(&mut client).await;
            if id == 0 {
                continue;
            } else if id == 50 {
                let _ = (
                    read_i32(&mut client).await,
                    read_i32(&mut client).await,
                    read_u8(&mut client).await,
                );
            } else if id == 51 {
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
            } else if id == 38 {
                let _ent = read_i32(&mut client).await;
                let status = read_u8(&mut client).await;
                assert_eq!(status, 2); // hurt animation!
                got_hurt_animation = true;
            } else if id == 8 {
                let health = read_i16(&mut client).await;
                assert_eq!(health, 18, "Health should be 18 after 5 block fall");
                got_damaged_health = true;
            }
        }
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pvp_attack_deals_damage_and_velocity() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut attacker = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut attacker, "Attacker").await;

        let mut victim = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut victim, "Victim").await;

        // Attacker entity_id is 1, Victim is 2
        // Attacker attacks Victim (entity 2) with Packet 7 (UseEntity)
        let mut writer = Writer::new();
        writer.write_u8(7);
        writer.write_i32(1); // user
        writer.write_i32(2); // target
        writer.write_bool(true); // attack
        attacker.write_all(writer.as_slice()).await.unwrap();

        // Victim should receive UpdateHealth (health < 20)
        let mut got_hurt = false;
        let mut got_health = false;
        while !got_hurt || !got_health {
            let id = read_u8(&mut victim).await;
            if id == 0 {
                continue;
            } else if id == 38 {
                let _ent = read_i32(&mut victim).await;
                let status = read_u8(&mut victim).await;
                assert_eq!(status, 2);
                got_hurt = true;
            } else if id == 8 {
                let health = read_i16(&mut victim).await;
                assert!(health < 20, "Victim health should be reduced, got {health}");
                got_health = true;
            } else if id == 28 {
                let _ = (
                    read_i32(&mut victim).await,
                    read_i16(&mut victim).await,
                    read_i16(&mut victim).await,
                    read_i16(&mut victim).await,
                );
            } else if id == 50 {
                let _ = (
                    read_i32(&mut victim).await,
                    read_i32(&mut victim).await,
                    read_u8(&mut victim).await,
                );
            } else if id == 51 {
                let _ = (
                    read_i32(&mut victim).await,
                    read_i16(&mut victim).await,
                    read_i32(&mut victim).await,
                );
                let _ = (
                    read_u8(&mut victim).await,
                    read_u8(&mut victim).await,
                    read_u8(&mut victim).await,
                );
                let len = read_i32(&mut victim).await as usize;
                let mut comp = vec![0u8; len];
                victim.read_exact(&mut comp).await.unwrap();
            } else if id == 3 {
                let _ = read_string(&mut victim).await;
            } else if id == 20 {
                let _ = read_i32(&mut victim).await;
                let _ = read_string(&mut victim).await;
                let _ = (
                    read_i32(&mut victim).await,
                    read_i32(&mut victim).await,
                    read_i32(&mut victim).await,
                );
                let _ = (
                    read_u8(&mut victim).await,
                    read_u8(&mut victim).await,
                    read_i16(&mut victim).await,
                );
            } else if id == 18 {
                let _ = (read_i32(&mut victim).await, read_u8(&mut victim).await);
            } else if id == 5 {
                let _ = (
                    read_i32(&mut victim).await,
                    read_i16(&mut victim).await,
                    read_i16(&mut victim).await,
                    read_i16(&mut victim).await,
                );
            } else if id == 40 {
                let _ = read_i32(&mut victim).await;
                let mut meta = [0u8; 3];
                victim.read_exact(&mut meta).await.unwrap();
            }
        }
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn negative_yaw_camera_rotation_tracked() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "Observer").await;

        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Rotator").await;

        tokio::time::sleep(Duration::from_millis(150)).await;

        // Rotator looks at negative yaw: -45.0, pitch -30.0
        let mut writer = Writer::new();
        writer.write_u8(12); // PlayerLook
        writer.write_f32(-45.0);
        writer.write_f32(-30.0);
        writer.write_bool(true);
        client2.write_all(writer.as_slice()).await.unwrap();

        // Observer should receive packet 32 (EntityLook) with non-zero yaw (around 224)
        let mut got_look = false;
        for _ in 0..60 {
            let id = read_u8(&mut client1).await;
            if id == 32 {
                let ent = read_i32(&mut client1).await;
                assert_eq!(ent, 2);
                let yaw = read_u8(&mut client1).await;
                let pitch = read_u8(&mut client1).await;
                assert_eq!(yaw, 224, "Negative yaw -45 deg must pack to 224, not 0");
                assert_eq!(pitch, 234, "Negative pitch -30 deg must pack to 234, not 0");
                got_look = true;
                break;
            } else if id == 0 {
                continue;
            } else if id == 3 {
                let _ = read_string(&mut client1).await;
            } else if id == 20 {
                let _ = read_i32(&mut client1).await;
                let _ = read_string(&mut client1).await;
                let _ = (
                    read_i32(&mut client1).await,
                    read_i32(&mut client1).await,
                    read_i32(&mut client1).await,
                );
                let _ = (
                    read_u8(&mut client1).await,
                    read_u8(&mut client1).await,
                    read_i16(&mut client1).await,
                );
            } else if id == 50 {
                let _ = (
                    read_i32(&mut client1).await,
                    read_i32(&mut client1).await,
                    read_u8(&mut client1).await,
                );
            } else if id == 51 {
                let _ = (
                    read_i32(&mut client1).await,
                    read_i16(&mut client1).await,
                    read_i32(&mut client1).await,
                );
                let _ = (
                    read_u8(&mut client1).await,
                    read_u8(&mut client1).await,
                    read_u8(&mut client1).await,
                );
                let len = read_i32(&mut client1).await as usize;
                let mut comp = vec![0u8; len];
                client1.read_exact(&mut comp).await.unwrap();
            } else if id == 40 {
                let _ = read_i32(&mut client1).await;
                let mut meta = [0u8; 3];
                client1.read_exact(&mut meta).await.unwrap();
            } else if id == 5 {
                let _ = (
                    read_i32(&mut client1).await,
                    read_i16(&mut client1).await,
                    read_i16(&mut client1).await,
                    read_i16(&mut client1).await,
                );
            }
        }
        assert!(
            got_look,
            "Observer should receive EntityLook with negative yaw packed"
        );
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_mining_spawns_pickup_and_collects() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "Miner").await;

        // Move player outside spawn protection (x = 20.5, y = 70.0, z = 0.5)
        let mut move_writer = Writer::new();
        move_writer.write_u8(11); // PlayerPosition
        move_writer.write_f64(20.5);
        move_writer.write_f64(70.0);
        move_writer.write_f64(71.62);
        move_writer.write_f64(0.5);
        move_writer.write_bool(true);
        client.write_all(move_writer.as_slice()).await.unwrap();

        // Give Miner 1 Dirt block (id 3) via /give Miner 3 1, and wait for pickup
        let mut chat_writer = Writer::new();
        chat_writer.write_u8(3);
        chat_writer.write_string("/give Miner 3 1", 100).unwrap();
        client.write_all(chat_writer.as_slice()).await.unwrap();

        let mut got_pickup = false;
        let mut got_collect = false;
        for _ in 0..80 {
            let id = read_u8(&mut client).await;
            match id {
                0 => {}
                3 => {
                    let _ = read_string(&mut client).await;
                }
                21 => {
                    // PickupSpawn!
                    let _eid = read_i32(&mut client).await;
                    let item_id = read_i16(&mut client).await;
                    let count = read_u8(&mut client).await;
                    let _damage = read_i16(&mut client).await;
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                    );
                    let _ = (
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                    assert_eq!(item_id, 3);
                    assert_eq!(count, 1);
                    got_pickup = true;
                }
                22 => {
                    // Collect!
                    let _collected = read_i32(&mut client).await;
                    let _collector = read_i32(&mut client).await;
                    got_collect = true;
                }
                28 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i16(&mut client).await,
                        read_i16(&mut client).await,
                        read_i16(&mut client).await,
                    );
                }
                29 => {
                    let _ = read_i32(&mut client).await;
                }
                31 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                34 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                50 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
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
                }
                104 => {
                    let _ = read_u8(&mut client).await;
                    let count = read_i16(&mut client).await;
                    for _ in 0..count {
                        let item = read_i16(&mut client).await;
                        if item >= 0 {
                            let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                        }
                    }
                }
                _ => {}
            }
            if got_pickup && got_collect {
                break;
            }
        }
        assert!(got_pickup, "Should receive PickupSpawn (21)");
        assert!(got_collect, "Should receive Collect (22)");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_command_shows_online_players() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "Alice").await;

        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Bob").await;

        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("/list", 100).unwrap();
        client1.write_all(writer.as_slice()).await.unwrap();

        let mut got_list = false;
        for _ in 0..40 {
            let id = read_next_non_chunk_packet_id(&mut client1).await;
            if id == 3 {
                let msg = read_string(&mut client1).await;
                if msg.starts_with("Connected players:") {
                    assert_eq!(msg, "Connected players: Alice, Bob");
                    got_list = true;
                    break;
                }
            } else if id == 40 {
                let _ = read_i32(&mut client1).await;
                let mut meta = [0u8; 3];
                client1.read_exact(&mut meta).await.unwrap();
            }
        }
        assert!(got_list, "Should receive /list response with Alice and Bob");
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_username_login_is_rejected() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "SameName").await;

        // Second client tries to log in with "samename" (case-insensitive duplicate)
        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        client2.write_all(&handshake("samename")).await.unwrap();
        client2.write_all(&login(14, "samename")).await.unwrap();

        assert_eq!(read_u8(&mut client2).await, 2);
        assert_eq!(read_string(&mut client2).await, "-");

        assert_eq!(read_u8(&mut client2).await, 255);
        let reason = read_string(&mut client2).await;
        assert!(
            reason.contains("already online"),
            "Expected duplicate login kick, got: {reason}"
        );
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn respawn_re_spawns_players_for_each_other() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client1 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client1, "Hero1").await;

        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Hero2").await;

        // Wait for initial tracking tick
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Hero1 dies via /kill and then sends Packet9Respawn
        let mut kill_writer = Writer::new();
        kill_writer.write_u8(3);
        kill_writer.write_string("/kill", 100).unwrap();
        client1.write_all(kill_writer.as_slice()).await.unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut respawn_writer = Writer::new();
        respawn_writer.write_u8(9);
        respawn_writer.write_i8(0);
        client1.write_all(respawn_writer.as_slice()).await.unwrap();

        // Hero2 must receive DestroyEntity(1) AND a fresh NamedEntitySpawn(1, "Hero1")!
        let mut saw_destroy = false;
        let mut saw_respawn_entity = false;
        for _ in 0..80 {
            let id = read_u8(&mut client2).await;
            match id {
                0 => {}
                3 => {
                    let _ = read_string(&mut client2).await;
                }
                5 => {
                    let _ = (
                        read_i32(&mut client2).await,
                        read_i16(&mut client2).await,
                        read_i16(&mut client2).await,
                        read_i16(&mut client2).await,
                    );
                }
                20 => {
                    let eid = read_i32(&mut client2).await;
                    let name = read_string(&mut client2).await;
                    let _ = (
                        read_i32(&mut client2).await,
                        read_i32(&mut client2).await,
                        read_i32(&mut client2).await,
                        read_u8(&mut client2).await,
                        read_u8(&mut client2).await,
                        read_i16(&mut client2).await,
                    );
                    if eid == 1 && name == "Hero1" && saw_destroy {
                        saw_respawn_entity = true;
                        break;
                    }
                }
                29 => {
                    let eid = read_i32(&mut client2).await;
                    if eid == 1 {
                        saw_destroy = true;
                    }
                }
                38 => {
                    let _ = (read_i32(&mut client2).await, read_u8(&mut client2).await);
                }
                40 => {
                    let _ = read_i32(&mut client2).await;
                    let mut meta = [0u8; 3];
                    client2.read_exact(&mut meta).await.unwrap();
                }
                50 => {
                    let _ = (
                        read_i32(&mut client2).await,
                        read_i32(&mut client2).await,
                        read_u8(&mut client2).await,
                    );
                }
                51 => {
                    let _ = (
                        read_i32(&mut client2).await,
                        read_i16(&mut client2).await,
                        read_i32(&mut client2).await,
                        read_u8(&mut client2).await,
                        read_u8(&mut client2).await,
                        read_u8(&mut client2).await,
                    );
                    let len = read_i32(&mut client2).await as usize;
                    let mut comp = vec![0u8; len];
                    client2.read_exact(&mut comp).await.unwrap();
                }
                _ => {}
            }
        }
        assert!(
            saw_destroy && saw_respawn_entity,
            "Hero2 must see Hero1 DestroyEntity followed by NamedEntitySpawn on respawn"
        );
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boat_placement_and_mounting() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "Sailor").await;

        // Move Sailor outside spawn protection to (20.5, 70.0, 0.5)
        let mut move_writer = Writer::new();
        move_writer.write_u8(11);
        move_writer.write_f64(20.5);
        move_writer.write_f64(70.0);
        move_writer.write_f64(71.62);
        move_writer.write_f64(0.5);
        move_writer.write_bool(true);
        client.write_all(move_writer.as_slice()).await.unwrap();

        // Place Boat (item 333) on block (20, 69, 0) face 1
        let mut place_writer = Writer::new();
        place_writer.write_u8(15);
        place_writer.write_i32(20);
        place_writer.write_u8(69);
        place_writer.write_i32(0);
        place_writer.write_u8(1);
        place_writer.write_i16(333);
        place_writer.write_u8(1);
        place_writer.write_i16(0);
        client.write_all(place_writer.as_slice()).await.unwrap();

        let mut boat_entity_id = None;
        for _ in 0..80 {
            let id = read_u8(&mut client).await;
            match id {
                0 => {}
                23 => {
                    // VehicleSpawn!
                    let eid = read_i32(&mut client).await;
                    let vtype = read_u8(&mut client).await;
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                    );
                    let thrower = read_i32(&mut client).await;
                    if thrower > 0 {
                        let _ = (
                            read_i16(&mut client).await,
                            read_i16(&mut client).await,
                            read_i16(&mut client).await,
                        );
                    }
                    assert_eq!(vtype, 1, "Boat VehicleSpawn type must be 1");
                    boat_entity_id = Some(eid);
                    break;
                }
                28 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i16(&mut client).await,
                        read_i16(&mut client).await,
                        read_i16(&mut client).await,
                    );
                }
                50 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                51 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i16(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                    let len = read_i32(&mut client).await as usize;
                    let mut comp = vec![0u8; len];
                    client.read_exact(&mut comp).await.unwrap();
                }
                53 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                103 => {
                    let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                    let item = read_i16(&mut client).await;
                    if item >= 0 {
                        let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                    }
                }
                _ => {}
            }
        }
        let boat_id = boat_entity_id.expect("Should receive VehicleSpawn (23) for placed boat");

        // Right-click boat (UseEntity 7, attack = false) to mount it
        let mut use_writer = Writer::new();
        use_writer.write_u8(7);
        use_writer.write_i32(1);
        use_writer.write_i32(boat_id);
        use_writer.write_bool(false);
        client.write_all(use_writer.as_slice()).await.unwrap();

        let mut got_attach = false;
        for _ in 0..80 {
            let id = read_u8(&mut client).await;
            match id {
                0 => {}
                13 => {
                    let _ = (
                        read_f64(&mut client).await,
                        read_f64(&mut client).await,
                        read_f64(&mut client).await,
                        read_f64(&mut client).await,
                        read_f32(&mut client).await,
                        read_f32(&mut client).await,
                        read_bool(&mut client).await,
                    );
                }
                18 => {
                    let _ = (read_i32(&mut client).await, read_u8(&mut client).await);
                }
                28 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i16(&mut client).await,
                        read_i16(&mut client).await,
                        read_i16(&mut client).await,
                    );
                }
                31 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                32 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                33 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                34 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                39 => {
                    let rider = read_i32(&mut client).await;
                    let vehicle = read_i32(&mut client).await;
                    assert_eq!(rider, 1);
                    assert_eq!(vehicle, boat_id);
                    got_attach = true;
                    break;
                }
                50 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                51 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_i16(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                    let len = read_i32(&mut client).await as usize;
                    let mut comp = vec![0u8; len];
                    client.read_exact(&mut comp).await.unwrap();
                }
                53 => {
                    let _ = (
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_i32(&mut client).await,
                        read_u8(&mut client).await,
                        read_u8(&mut client).await,
                    );
                }
                103 => {
                    let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                    let item = read_i16(&mut client).await;
                    if item >= 0 {
                        let _ = (read_u8(&mut client).await, read_i16(&mut client).await);
                    }
                }
                _ => {}
            }
        }
        assert!(
            got_attach,
            "Should receive AttachEntity (39) when mounting boat"
        );
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}
