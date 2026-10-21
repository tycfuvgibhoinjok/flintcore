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

mod common;
use common::*;

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
    /// The harness of the tests.
    ///
    /// The spawn of the player is not randomised, so the tests know the
    /// position of the player.  The reference randomises the spawn
    /// (`world-settings.randomize-spawn`) — the test of the randomisation
    /// uses the harness with it.
    async fn start() -> Self {
        Self::start_with(false).await
    }

    /// The harness with the randomisation of the spawn of the reference.
    async fn start_with_randomized_spawn() -> Self {
        Self::start_with(true).await
    }

    async fn start_with(randomize_spawn: bool) -> Self {
        let mut config = Config::default();
        config.world.save_enabled = false;
        config.world.randomize_spawn = randomize_spawn;
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

/// The spawn of the player is randomised by the reference.
///
/// Java `EntityPlayer.java:46-51`: the offset of the spawn is
/// `random.nextInt(20) - 10` on the X and the Z — the position of the player
/// is within ten blocks of the spawn of the world.  The option is enabled by
/// default (`PoseidonConfig.java:152`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_spawn_of_the_player_is_randomised() {
    let harness = Harness::start_with_randomized_spawn().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        client.write_all(&handshake("Player")).await.unwrap();
        client.write_all(&login(14, "Player")).await.unwrap();

        let handshake_reply = read_u8(&mut client).await;
        assert_eq!(handshake_reply, 2);
        assert_eq!(read_string(&mut client).await, "-");

        let login_reply = read_u8(&mut client).await;
        assert_eq!(login_reply, 1);
        let _entity_id = read_i32(&mut client).await;
        let _username = read_string(&mut client).await;
        let _seed = read_i64(&mut client).await;
        let _dimension = read_u8(&mut client).await;

        // The position of the spawn of the world: it is not randomised.
        assert_eq!(read_u8(&mut client).await, 6);
        let spawn_x = read_i32(&mut client).await;
        let spawn_y = read_i32(&mut client).await;
        let spawn_z = read_i32(&mut client).await;
        assert_eq!((spawn_x, spawn_y, spawn_z), (0, 64, 0));

        // The time of the world.
        assert_eq!(read_u8(&mut client).await, 4);
        let _ = read_i64(&mut client).await;

        // The nine chunks of the spawn.
        read_pre_chunks(&mut client).await;

        // The position of the player: the offset of the reference is within
        // ten blocks of the spawn.
        let teleport_id = read_u8(&mut client).await;
        assert_eq!(teleport_id, 13);
        let x = read_f64(&mut client).await;
        let _y = read_f64(&mut client).await;
        let stance = read_f64(&mut client).await;
        let z = read_f64(&mut client).await;
        assert!(
            (-9.5..=9.5).contains(&x),
            "the X of the player is within ten blocks of the spawn, got {x}"
        );
        assert!(
            (-9.5..=9.5).contains(&z),
            "the Z of the player is within ten blocks of the spawn, got {z}"
        );
        assert!(
            stance >= 64.0,
            "the height of the player is the height of the column of the spawn, got {stance}"
        );
    };

    timeout(Duration::from_secs(40), exchange).await.unwrap();
    harness.stop().await;
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

        // The 9 initial spawn chunks: the packets of the pre-chunk have a
        // normal priority, the data of the chunks has a low priority.
        read_pre_chunks(&mut client).await;

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

        // The data of the chunks has a low priority (`Packet.k`): it comes
        // after the other packets of the login.
        assert_eq!(read_u8(&mut client).await, 4); // UpdateTime
        let _ = read_i64(&mut client).await;
        read_window_items(&mut client).await;
        for _ in 0..9 {
            assert_eq!(read_u8(&mut client).await, 51); // MapChunk
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
        login_and_consume_spawn(&mut client, "Player3").await;

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

/// Reads the packets of the chunks (`Packet50PreChunk`).
///
/// The packets of the pre-chunk have a normal priority, so they are sent
/// before the other packets of the login.
async fn read_pre_chunks(client: &mut TcpStream) {
    for _ in 0..9 {
        assert_eq!(read_u8(client).await, 50, "PreChunk");
        let _ = (
            read_i32(client).await,
            read_i32(client).await,
            read_bool(client).await,
        );
    }
}

/// Reads the items of the inventory and the item in the hand.
async fn read_window_items(client: &mut TcpStream) {
    assert_eq!(read_u8(client).await, 104, "WindowItems");
    let _win_id = read_u8(client).await;
    let count = read_i16(client).await;
    for _ in 0..count {
        let item_id = read_i16(client).await;
        if item_id >= 0 {
            let _ = (read_u8(client).await, read_i16(client).await);
        }
    }
    assert_eq!(read_u8(client).await, 103, "SetSlot");
    let _ = read_u8(client).await;
    let _ = read_i16(client).await;
    let _item_id = read_i16(client).await;
}

async fn login_and_consume_spawn(client: &mut TcpStream, username: &str) {
    client.write_all(&handshake(username)).await.unwrap();
    client.write_all(&login(14, username)).await.unwrap();

    assert_eq!(read_u8(client).await, 2);
    assert_eq!(read_string(client).await, "-");

    // The packets of the login (`NetLoginHandler`): the login, the spawn
    // position, the time of the world, the position of the player, the
    // second packet of the time, the items of the inventory and the item in
    // hand.  The data of the chunks has a low priority (`Packet.k`), so it
    // comes at the end.
    let mut pre_chunks = 0;
    let mut chunk_data = 0;
    let mut times = 0;
    let mut spawn = false;
    let mut items = false;
    let mut slot = false;
    while pre_chunks < 9 || chunk_data < 9 || times < 2 || !spawn || !items || !slot {
        let id = read_u8(client).await;
        match id {
            1 => {
                let _ = read_i32(client).await;
                let _ = read_string(client).await;
                let _ = read_i64(client).await;
                let _ = read_u8(client).await;
            }
            4 => {
                let _ = read_i64(client).await;
                times += 1;
            }
            6 => {
                let _ = (
                    read_i32(client).await,
                    read_i32(client).await,
                    read_i32(client).await,
                );
            }
            13 => {
                let _ = (
                    read_f64(client).await,
                    read_f64(client).await,
                    read_f64(client).await,
                    read_f64(client).await,
                );
                let _ = (read_f32(client).await, read_f32(client).await);
                let _ = read_bool(client).await;
                spawn = true;
            }
            50 => {
                let _ = (
                    read_i32(client).await,
                    read_i32(client).await,
                    read_bool(client).await,
                );
                pre_chunks += 1;
            }
            51 => {
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
                chunk_data += 1;
            }
            103 => {
                let _ = read_u8(client).await;
                let _ = read_i16(client).await;
                let item_id = read_i16(client).await;
                if item_id >= 0 {
                    let _ = (read_u8(client).await, read_i16(client).await);
                }
                slot = true;
            }
            104 => {
                let _ = read_u8(client).await;
                let count = read_i16(client).await;
                for _ in 0..count {
                    let item_id = read_i16(client).await;
                    if item_id >= 0 {
                        let _ = (read_u8(client).await, read_i16(client).await);
                    }
                }
                items = true;
            }
            other => {
                // The packets of the world that the server sends between the
                // packets of the login.
                let _ = skip_packet_payload(client, other).await;
            }
        }
    }
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

        assert_eq!(read_packet_id(&mut client, &[255]).await, 255);
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

        assert_eq!(read_packet_id(&mut client, &[255]).await, 255);
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

        assert_eq!(read_packet_id(&mut client, &[3]).await, 3);
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

        assert_eq!(read_packet_id(&mut client, &[3]).await, 3);
        let msg = read_string(&mut client).await;
        // Java `ConsoleCommandHandler.java:288`.
        assert_eq!(msg, "Unknown console command. Type \"help\" for help.");
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

        // Java `ConsoleCommandHandler.java:355-374`: every line of the help
        // of the reference is a separate message.
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..20 {
            assert_eq!(read_packet_id(&mut client, &[3]).await, 3);
            lines.push(read_string(&mut client).await);
        }
        let help = lines.join("\n");
        assert!(help.contains("Console commands:"), "the help: {help}");
        assert!(help.contains("help  or  ?"), "the help: {help}");
        assert!(help.contains("say <message>"), "the help: {help}");
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

        assert_eq!(read_packet_id(&mut client, &[3]).await, 3);
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

        assert_eq!(read_packet_id(&mut client, &[3]).await, 3);
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
            let id = read_packet_id(&mut client1, &[3]).await;
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
            let id = read_packet_id(&mut client2, &[3]).await;
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

        // The watcher receives the spawn of the other player.
        let id = read_packet_id(&mut client1, &[20]).await;
        assert_eq!(id, 20);
        let entity_id = read_i32(&mut client1).await;
        assert!(entity_id > 0);
        let name = read_string(&mut client1).await;
        assert_eq!(name, "Spawnee");
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

        // The transaction of the window.
        let id = read_packet_id(&mut client, &[106]).await;
        assert_eq!(id, 106);
        let window_id = read_u8(&mut client).await;
        assert_eq!(window_id, 0);
        let action = read_i16(&mut client).await;
        assert_eq!(action, 1);
        let accepted = read_bool(&mut client).await;
        assert!(accepted);
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

        // The first packet of the respawn is the packet of the Nether
        // (`Packet9Respawn`).
        let id = read_packet_id(&mut client, &[9]).await;
        assert_eq!(id, 9);
        let dim = read_u8(&mut client).await as i8;
        assert_eq!(dim, -1);

        // Expect second Respawn (0)
        assert_eq!(read_u8(&mut client).await, 9);
        assert_eq!(read_u8(&mut client).await as i8, 0);

        // Expect 9 spawn chunks (the data of the chunks has a low priority
        // and comes at the end of the login).
        read_pre_chunks(&mut client).await;

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

        // The message with the ampersand: the reference does not change it
        // (the client does).
        let mut writer = Writer::new();
        writer.write_u8(3);
        writer.write_string("&cRed message", 100).unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        let id = read_packet_id(&mut client, &[3]).await;
        assert_eq!(id, 3);
        let msg = read_string(&mut client).await;
        assert_eq!(msg, "<ColorUser> &cRed message");

        // The character of the colour (`§`) is not in the file `font.txt` of
        // the reference (`FontAllowedCharacters`), so the player is kicked
        // with "Illegal characters in chat"
        // (`NetServerHandler.java:811-815`).
        let mut writer2 = Writer::new();
        writer2.write_u8(3);
        writer2.write_string("§aGreen message", 100).unwrap();
        client.write_all(writer2.as_slice()).await.unwrap();

        let id2 = read_packet_id(&mut client, &[255]).await;
        assert_eq!(id2, 255, "the character of the colour is not allowed");
        let reason = read_string(&mut client).await;
        assert_eq!(reason, "Illegal characters in chat");
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

        // The sixty ticks of the protection of the spawn
        // (`EntityPlayer.bM`): the damage of the fall is not applied while
        // the player is protected.
        wait_for_time_packets(&mut client, 4).await;

        // First sync position on ground
        let mut writer0 = Writer::new();
        writer0.write_u8(11);
        writer0.write_f64(0.0);
        writer0.write_f64(64.0);
        writer0.write_f64(65.62); // the stance: the height of the eyes
        writer0.write_f64(0.0);
        writer0.write_bool(true);
        client.write_all(writer0.as_slice()).await.unwrap();

        // Fall 5 blocks in air: Packet 11 (PlayerPosition), on_ground = false
        let mut writer = Writer::new();
        writer.write_u8(11);
        writer.write_f64(0.0);
        writer.write_f64(59.0); // 5 blocks drop
        writer.write_f64(60.62);
        writer.write_f64(0.0);
        writer.write_bool(false);
        client.write_all(writer.as_slice()).await.unwrap();

        // Land on ground: Packet 11 (PlayerPosition), on_ground = true
        let mut writer2 = Writer::new();
        writer2.write_u8(11);
        writer2.write_f64(0.0);
        writer2.write_f64(59.0);
        writer2.write_f64(60.62);
        writer2.write_f64(0.0);
        writer2.write_bool(true);
        client.write_all(writer2.as_slice()).await.unwrap();

        // The animation of the damage and the packet of the health.
        let id = read_packet_id(&mut client, &[38]).await;
        assert_eq!(id, 38, "the animation of the damage");
        let _ent = read_i32(&mut client).await;
        let status = read_u8(&mut client).await;
        assert_eq!(status, 2, "the animation of the damage");

        let id = read_packet_id(&mut client, &[8]).await;
        assert_eq!(id, 8, "the health of the player");
        let health = read_i16(&mut client).await;
        assert_eq!(health, 18, "the health after the fall of five blocks");
    };

    timeout(Duration::from_secs(40), exchange).await.unwrap();
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

        // The sixty ticks of the protection of the spawn
        // (`EntityPlayer.bM`): the damage of the player is not applied while
        // the player is protected.
        wait_for_time_packets(&mut victim, 4).await;

        // Attacker entity_id is 1, Victim is 2
        // Attacker attacks Victim (entity 2) with Packet 7 (UseEntity)
        let mut writer = Writer::new();
        writer.write_u8(7);
        writer.write_i32(1); // user
        writer.write_i32(2); // target
        writer.write_bool(true); // attack
        attacker.write_all(writer.as_slice()).await.unwrap();

        // The victim receives the animation of the damage.
        let id = read_packet_id(&mut victim, &[38]).await;
        assert_eq!(id, 38, "the animation of the damage");
        let _ent = read_i32(&mut victim).await;
        let status = read_u8(&mut victim).await;
        assert_eq!(status, 2, "the animation of the damage");

        // And the packet of the health.
        let id = read_packet_id(&mut victim, &[8]).await;
        assert_eq!(id, 8, "the health of the victim");
        let health = read_i16(&mut victim).await;
        assert!(
            health < 20,
            "the health of the victim is reduced, got {health}"
        );
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

        // The observer receives the rotation of the entity: the yaw of -45
        // degrees is packed to 224, the pitch of -30 degrees to 234.
        loop {
            let id = read_packet_id(&mut client1, &[32]).await;
            assert_eq!(id, 32);
            let ent = read_i32(&mut client1).await;
            let yaw = read_u8(&mut client1).await;
            let pitch = read_u8(&mut client1).await;
            if ent == 2 {
                assert_eq!(yaw, 224, "the yaw of -45 degrees is packed to 224");
                assert_eq!(pitch, 234, "the pitch of -30 degrees is packed to 234");
                break;
            }
        }
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

        // The item on the ground.
        let id = read_packet_id(&mut client, &[21]).await;
        assert_eq!(id, 21, "the item on the ground");
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
        assert_eq!(item_id, 3, "the block of the dirt");
        assert_eq!(count, 1, "the count of the item");

        // And its collect.
        let id = read_packet_id(&mut client, &[22]).await;
        assert_eq!(id, 22, "the collect of the item");
        let _collected = read_i32(&mut client).await;
        let _collector = read_i32(&mut client).await;
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

        // The list of the players.
        loop {
            let id = read_packet_id(&mut client1, &[3]).await;
            assert_eq!(id, 3);
            let msg = read_string(&mut client1).await;
            if msg.starts_with("Connected players:") {
                assert_eq!(msg, "Connected players: Alice, Bob");
                break;
            }
        }
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
async fn kicked_player_can_reconnect_with_the_same_name() {
    let harness = Harness::start().await;
    let exchange = async {
        let mut client = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client, "Kicked").await;

        // A too long chat message gets the player kicked.
        let mut writer = Writer::new();
        writer.write_u8(3);
        writer
            .write_string(&"a".repeat(101), MAX_CHAT_LENGTH)
            .unwrap();
        client.write_all(writer.as_slice()).await.unwrap();

        assert_eq!(read_packet_id(&mut client, &[255]).await, 255);
        let reason = read_string(&mut client).await;
        assert_eq!(reason, "Chat message too long");

        // The name is free right away: the same name may log in again.
        let mut client2 = TcpStream::connect(harness.address).await.unwrap();
        login_and_consume_spawn(&mut client2, "Kicked").await;
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

        // The second player sees the destroy of the entity and its spawn.
        let mut saw_destroy = false;
        loop {
            let id = read_packet_id(&mut client2, &[29, 20]).await;
            if id == 29 {
                let eid = read_i32(&mut client2).await;
                if eid == 1 {
                    saw_destroy = true;
                }
            } else {
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
                    break;
                }
            }
        }
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

        // The boat on the ground.
        let id = read_packet_id(&mut client, &[23]).await;
        assert_eq!(id, 23, "the spawn of the boat");
        let boat_id = read_i32(&mut client).await;
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
        assert_eq!(vtype, 1, "the type of the boat");

        // Right-click boat (UseEntity 7, attack = false) to mount it
        let mut use_writer = Writer::new();
        use_writer.write_u8(7);
        use_writer.write_i32(1);
        use_writer.write_i32(boat_id);
        use_writer.write_bool(false);
        client.write_all(use_writer.as_slice()).await.unwrap();

        // The mount of the boat.
        let id = read_packet_id(&mut client, &[39]).await;
        assert_eq!(id, 39, "the attach of the entity");
        let rider = read_i32(&mut client).await;
        let vehicle = read_i32(&mut client).await;
        assert_eq!(rider, 1);
        assert_eq!(vehicle, boat_id);
    };

    timeout(Duration::from_secs(5), exchange).await.unwrap();
    harness.stop().await;
}
