//! Java-free regression test: regenerates a 3x3 populated region for each seed
//! and compares an FNV-1a 64 hash of the full dump image (blocks | meta | height)
//! against the golden hashes committed in `tests/golden/poseidon_region_hashes.txt`.
//!
//! The golden file is derived from Project-Poseidon (Java) dumps — see the header
//! of the file for the regeneration command. This test needs no JVM and no dumps
//! at run time; it pins the Java-verified behaviour against future changes.

use flintcore::config::WorldConfig;
use flintcore::world::chunk::{Chunk, ChunkPos, CHUNK_WIDTH};
use flintcore::world::world::World;
use std::collections::HashMap;
use std::path::Path;

const SEEDS: &[i64] = &[
    12345, 0, 9999, 54321, -12345, 1337, 42, 1, 7, 100, 777, 2024, 31337, 65536, 777777, -42, -999,
    123456789,
];

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Rebuilds the 49664-byte Poseidon dump image of a chunk:
/// 32768 block bytes (wire index `(x << 11) | (z << 7) | y`), then 16384 meta
/// bytes (low nibble = even wire cell, high nibble = odd wire cell), then 256
/// height bytes (`(z << 4) | x`).
fn build_dump_image(chunk: &Chunk) -> Vec<u8> {
    let mut img = Vec::with_capacity(32768 + 16384 + 256);
    for wire in 0..32768usize {
        img.push(chunk.get_block(wire >> 11, wire & 127, (wire >> 7) & 15));
    }
    for wire in (0..32768usize).step_by(2) {
        let lo = chunk.get_metadata(wire >> 11, wire & 127, (wire >> 7) & 15) & 15;
        let hi_wire = wire + 1;
        let hi = chunk.get_metadata(hi_wire >> 11, hi_wire & 127, (hi_wire >> 7) & 15) & 15;
        img.push(lo | (hi << 4));
    }
    for z in 0..CHUNK_WIDTH {
        for x in 0..CHUNK_WIDTH {
            img.push(chunk.get_height(x, z));
        }
    }
    assert_eq!(img.len(), 32768 + 16384 + 256);
    img
}

#[test]
fn poseidon_golden_hashes() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/poseidon_region_hashes.txt");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let mut golden: HashMap<(i64, i32, i32), u64> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let seed: i64 = it.next().unwrap().parse().unwrap();
        let cx: i32 = it.next().unwrap().parse().unwrap();
        let cz: i32 = it.next().unwrap().parse().unwrap();
        let hash = u64::from_str_radix(it.next().unwrap(), 16).unwrap();
        golden.insert((seed, cx, cz), hash);
    }
    assert!(!golden.is_empty(), "golden hash file is empty");

    let mut checked = 0usize;
    for &seed in SEEDS {
        let config = WorldConfig {
            seed,
            save_enabled: false,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        for cz in -1..=1 {
            for cx in -1..=1 {
                world.populate_chunk(ChunkPos::new(cx, cz));
            }
        }

        for cz in -1..=1 {
            for cx in -1..=1 {
                let Some(&expected) = golden.get(&(seed, cx, cz)) else {
                    continue;
                };
                let chunk = world
                    .get_chunk(ChunkPos::new(cx, cz))
                    .expect("generated chunk missing");
                let actual = fnv1a64(&build_dump_image(chunk));
                assert_eq!(
                    actual, expected,
                    "golden hash mismatch for seed {seed} chunk ({cx},{cz})"
                );
                checked += 1;
            }
        }
    }

    assert_eq!(
        checked,
        golden.len(),
        "some golden entries were never checked (seed list out of sync?)"
    );
}

/// Extra sanity: the dump-image builder must reproduce the Java dump bytes
/// exactly. Skipped silently when the dumps directory is unavailable.
#[test]
fn dump_image_matches_java_dumps() {
    let dir = std::path::PathBuf::from("/tmp/poseidon_dumps");
    if !dir.exists() {
        eprintln!("dumps dir missing, skipping byte-level check");
        return;
    }
    for &seed in &SEEDS[..4] {
        let config = WorldConfig {
            seed,
            save_enabled: false,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);
        for cz in -1..=1 {
            for cx in -1..=1 {
                world.populate_chunk(ChunkPos::new(cx, cz));
            }
        }
        for cz in -1..=1 {
            for cx in -1..=1 {
                let path = dir.join(format!("chunk_{seed}_{cx}_{cz}.bin"));
                let Ok(java) = std::fs::read(&path) else {
                    continue;
                };
                let chunk = world.get_chunk(ChunkPos::new(cx, cz)).expect("chunk");
                let img = build_dump_image(chunk);
                assert_eq!(
                    img.len(),
                    java.len(),
                    "dump image length mismatch for seed {seed} chunk ({cx},{cz})"
                );
                assert!(
                    img == java,
                    "dump image bytes mismatch for seed {seed} chunk ({cx},{cz})"
                );
            }
        }
    }
}
