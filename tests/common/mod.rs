//! The reading of the packets of the protocol for the live tests.
//!
//! Every packet the server sends is known, so the tests can skip the packets
//! they do not inspect and stay in sync with the stream.

#![allow(dead_code)]

use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

pub async fn read_u8(stream: &mut TcpStream) -> u8 {
    let mut byte = [0u8; 1];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut byte)).await {
        Ok(Ok(_)) => byte[0],
        _ => panic!("read_u8 timeout"),
    }
}

pub async fn read_i32(stream: &mut TcpStream) -> i32 {
    let mut bytes = [0u8; 4];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i32::from_be_bytes(bytes),
        _ => panic!("read_i32 timeout"),
    }
}

pub async fn read_i64(stream: &mut TcpStream) -> i64 {
    let mut bytes = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i64::from_be_bytes(bytes),
        _ => panic!("read_i64 timeout"),
    }
}

pub async fn read_i16(stream: &mut TcpStream) -> i16 {
    let mut bytes = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => i16::from_be_bytes(bytes),
        _ => panic!("read_i16 timeout"),
    }
}

pub async fn read_f64(stream: &mut TcpStream) -> f64 {
    let mut bytes = [0u8; 8];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => f64::from_be_bytes(bytes),
        _ => panic!("read_f64 timeout"),
    }
}

pub async fn read_f32(stream: &mut TcpStream) -> f32 {
    let mut bytes = [0u8; 4];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => f32::from_be_bytes(bytes),
        _ => panic!("read_f32 timeout"),
    }
}

pub async fn read_u16(stream: &mut TcpStream) -> u16 {
    let mut bytes = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => {}
        _ => panic!("read_u16 timeout"),
    }
    u16::from_be_bytes(bytes)
}

pub async fn read_bool(stream: &mut TcpStream) -> bool {
    read_u8(stream).await != 0
}

/// Reads a string of the modified UTF-8 (the title of the window).
///
/// Java `DataOutput.writeUTF`: the length is the number of the bytes.
pub async fn read_modified_utf8_string(stream: &mut TcpStream) -> String {
    let len = read_u16(stream).await as usize;
    let mut bytes = vec![0u8; len];
    match tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes)).await {
        Ok(Ok(_)) => {}
        _ => panic!("read_modified_utf8_string timeout"),
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub async fn read_string(stream: &mut TcpStream) -> String {
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

/// Reads the metadata of an entity: the types of the values.
pub async fn read_metadata(stream: &mut TcpStream) {
    loop {
        let header = read_u8(stream).await;
        if header == 127 {
            break;
        }
        match (header >> 5) & 7 {
            0 => {
                let _ = read_u8(stream).await;
            }
            1 => {
                let _ = read_i16(stream).await;
            }
            2 | 3 => {
                let _ = read_i32(stream).await;
            }
            4 => {
                let len = read_i16(stream).await as usize;
                for _ in 0..len {
                    let _ = read_i16(stream).await;
                }
            }
            5 => {
                let item_id = read_i16(stream).await;
                if item_id >= 0 {
                    let _ = read_u8(stream).await;
                    let _ = read_i16(stream).await;
                }
            }
            6 => {
                for _ in 0..6 {
                    let _ = read_i32(stream).await;
                }
            }
            _ => break,
        }
    }
}

/// Skips the payload of a packet of the world.
///
/// Every packet the server sends is known, so the tests can skip the packets
/// they do not inspect and stay in sync with the stream.  Returns `true`
/// when the packet is known and its payload is consumed.
pub async fn skip_packet_payload(stream: &mut TcpStream, id: u8) -> bool {
    match id {
        0 => {}
        1 => {
            // The login.
            let _ = read_i32(stream).await;
            let _ = read_string(stream).await;
            let _ = read_i64(stream).await;
            let _ = read_u8(stream).await;
        }
        2 => {
            // The handshake.
            let _ = read_string(stream).await;
        }
        3 => {
            // The chat.
            let _ = read_string(stream).await;
        }
        4 => {
            // The time of the world.
            let _ = read_i64(stream).await;
        }
        5 => {
            // The equipment of the entity.
            let _ = (
                read_i32(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
            );
        }
        6 => {
            // The position of the spawn.
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
        }
        7 => {
            // The use of the entity.
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_bool(stream).await,
            );
        }
        8 => {
            // The health.
            let _ = read_i16(stream).await;
        }
        9 => {
            // The respawn.
            let _ = read_u8(stream).await;
        }
        10 => {
            // The flying.
            let _ = read_bool(stream).await;
        }
        13 => {
            // The position and the rotation of the player.
            let _ = (
                read_f64(stream).await,
                read_f64(stream).await,
                read_f64(stream).await,
                read_f64(stream).await,
            );
            let _ = (read_f32(stream).await, read_f32(stream).await);
            let _ = read_bool(stream).await;
        }
        17 => {
            // The bed.
            let _ = (
                read_i32(stream).await,
                read_u8(stream).await,
                read_i32(stream).await,
                read_u8(stream).await,
                read_i32(stream).await,
            );
        }
        18 => {
            // The animation of the arm.
            let _ = (read_i32(stream).await, read_u8(stream).await);
        }
        20 => {
            // The spawn of the player.
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
        }
        21 => {
            // The spawn of the item.
            let _ = read_i32(stream).await;
            let _ = (
                read_i16(stream).await,
                read_u8(stream).await,
                read_i16(stream).await,
            );
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
            let _ = (
                read_u8(stream).await,
                read_u8(stream).await,
                read_u8(stream).await,
            );
        }
        22 => {
            // The collect of the item.
            let _ = (read_i32(stream).await, read_i32(stream).await);
        }
        23 => {
            // The spawn of the vehicle.
            let _ = read_i32(stream).await;
            let _ = read_u8(stream).await;
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
            let thrower = read_i32(stream).await;
            if thrower > 0 {
                let _ = (
                    read_i16(stream).await,
                    read_i16(stream).await,
                    read_i16(stream).await,
                );
            }
        }
        24 => {
            // The spawn of the mob.
            let _ = read_i32(stream).await;
            let _ = read_u8(stream).await;
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
            let _ = (read_u8(stream).await, read_u8(stream).await);
            read_metadata(stream).await;
        }
        25 => {
            // The spawn of the painting.
            let _ = read_i32(stream).await;
            let _ = read_string(stream).await;
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
        }
        28 => {
            // The speed of the entity.
            let _ = (
                read_i32(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
            );
        }
        29 | 30 => {
            // The destroy of the entity, the entity itself.
            let _ = read_i32(stream).await;
        }
        31 => {
            // The relative move of the entity.
            let _ = read_i32(stream).await;
            let _ = (
                read_u8(stream).await,
                read_u8(stream).await,
                read_u8(stream).await,
            );
        }
        32 => {
            // The rotation of the entity.
            let _ = read_i32(stream).await;
            let _ = (read_u8(stream).await, read_u8(stream).await);
        }
        33 => {
            // The relative move and the rotation of the entity.
            let _ = read_i32(stream).await;
            let _ = (
                read_u8(stream).await,
                read_u8(stream).await,
                read_u8(stream).await,
            );
            let _ = (read_u8(stream).await, read_u8(stream).await);
        }
        34 => {
            // The teleport of the entity.
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
            let _ = (read_u8(stream).await, read_u8(stream).await);
        }
        38 => {
            // The status of the entity.
            let _ = (read_i32(stream).await, read_u8(stream).await);
        }
        39 => {
            // The attach of the entity.
            let _ = (read_i32(stream).await, read_i32(stream).await);
        }
        40 => {
            // The metadata of the entity.
            let _ = read_i32(stream).await;
            read_metadata(stream).await;
        }
        50 => {
            // The pre-chunk.
            let _ = (
                read_i32(stream).await,
                read_i32(stream).await,
                read_bool(stream).await,
            );
        }
        51 => {
            // The data of the chunk.
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
        }
        52 => {
            // The change of many blocks.
            let _ = (read_i32(stream).await, read_i32(stream).await);
            let count = read_i16(stream).await as usize;
            for _ in 0..count {
                let _ = read_u16(stream).await;
            }
            let mut blocks = vec![0u8; count * 2];
            stream.read_exact(&mut blocks).await.unwrap();
        }
        53 => {
            // The change of the block.
            let _ = (
                read_i32(stream).await,
                read_u8(stream).await,
                read_i32(stream).await,
                read_u8(stream).await,
                read_u8(stream).await,
            );
        }
        54 => {
            // The note block.
            let _ = (
                read_i32(stream).await,
                read_i16(stream).await,
                read_i32(stream).await,
                read_u8(stream).await,
                read_u8(stream).await,
            );
        }
        60 => {
            // The explosion.
            let _ = (
                read_f64(stream).await,
                read_f64(stream).await,
                read_f64(stream).await,
                read_f32(stream).await,
            );
            let count = read_i32(stream).await as usize;
            let mut records = vec![0u8; count * 3];
            stream.read_exact(&mut records).await.unwrap();
        }
        70 => {
            // The bed.
            let _ = read_u8(stream).await;
        }
        71 => {
            // The weather.
            let _ = (
                read_i32(stream).await,
                read_u8(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
                read_i32(stream).await,
            );
        }
        100 => {
            // The window.
            let _ = read_u8(stream).await;
            let _ = read_u8(stream).await;
            let _ = read_modified_utf8_string(stream).await;
            let _ = read_u8(stream).await;
        }
        101 => {
            // The close of the window.
            let _ = read_u8(stream).await;
        }
        103 => {
            // The slot of the window.
            let _ = read_u8(stream).await;
            let _ = read_i16(stream).await;
            let item_id = read_i16(stream).await;
            if item_id >= 0 {
                let _ = (read_u8(stream).await, read_i16(stream).await);
            }
        }
        104 => {
            // The items of the window.
            let _ = read_u8(stream).await;
            let count = read_i16(stream).await;
            for _ in 0..count {
                let item_id = read_i16(stream).await;
                if item_id >= 0 {
                    let _ = (read_u8(stream).await, read_i16(stream).await);
                }
            }
        }
        105 => {
            // The bar of the craft.
            let _ = (
                read_u8(stream).await,
                read_i16(stream).await,
                read_i16(stream).await,
            );
        }
        106 => {
            // The transaction of the window.
            let _ = (
                read_u8(stream).await,
                read_i16(stream).await,
                read_bool(stream).await,
            );
        }
        130 => {
            // The sign: the four lines of the text.
            let _ = (
                read_i32(stream).await,
                read_i16(stream).await,
                read_i32(stream).await,
            );
            for _ in 0..4 {
                let _ = read_string(stream).await;
            }
        }
        200 => {
            // The statistic.
            let _ = (read_i32(stream).await, read_u8(stream).await);
        }
        255 => {
            // The kick.
            let _ = read_string(stream).await;
        }
        _ => return false,
    }
    true
}

/// Reads the next packet of the client whose id is in the list of the
/// inspected packets.
///
/// The payload of the other packets is consumed, so the stream of the packets
/// stays in sync.
pub async fn read_packet_id(stream: &mut TcpStream, inspect: &[u8]) -> u8 {
    loop {
        let id = read_u8(stream).await;
        if inspect.contains(&id) {
            return id;
        }
        assert!(skip_packet_payload(stream, id).await, "unknown packet {id}");
    }
}

/// Waits for the given number of the packets of the time of the world.
///
/// The server sends the time of the world to the players every twenty ticks
/// (`MinecraftServer.tick`), so four packets are eighty ticks — more than
/// the sixty ticks of the protection of the spawn of the reference
/// (`EntityPlayer.bM`).
pub async fn wait_for_time_packets(client: &mut TcpStream, count: usize) {
    for _ in 0..count {
        let id = read_packet_id(client, &[4]).await;
        assert_eq!(id, 4, "the time of the world");
        let _ = read_i64(client).await;
    }
}
