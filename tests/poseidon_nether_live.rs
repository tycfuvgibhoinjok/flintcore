//! Nether live-order parity against Project-Poseidon.
//!
//! The Nether generator is stateful: `ChunkProviderHell.getChunkAt(populate)`
//! continues the `Random` left behind by the last `getOrCreateChunk` call, so
//! the chunk load order changes decoration output. This harness loads a fixed
//! deterministic order through `ChunkProviderServer.getChunkAt` on both sides
//! and compares every resulting chunk image.
//!
//! Reference dumps: `tools/poseidon_dump/PoseidonHellLive.java`.
//! Skipped when `/tmp/hell_live_dumps` is absent.

use flintcore::config::WorldConfig;
use flintcore::world::chunk::{Chunk, ChunkPos, CHUNK_WIDTH};
use flintcore::world::world::World;
use std::path::Path;

/// 121 chunks in `(-5..=5)^2`, shuffled with `random.Random(7)`.
const LOAD_ORDER: &[(i32, i32)] = &[
    (4, 2),
    (4, -4),
    (3, 3),
    (-4, 2),
    (0, -4),
    (-2, -5),
    (0, 4),
    (-3, -2),
    (3, -1),
    (-4, -2),
    (-5, -5),
    (-4, -5),
    (-3, 4),
    (-2, 0),
    (5, 0),
    (-2, 5),
    (0, 0),
    (4, 1),
    (-2, -4),
    (-1, 3),
    (2, -1),
    (-1, -1),
    (4, 4),
    (-5, -3),
    (3, 4),
    (1, 0),
    (-5, 4),
    (-2, -3),
    (5, -2),
    (0, 2),
    (-5, 3),
    (5, 5),
    (-3, -5),
    (4, -2),
    (-4, 5),
    (0, 5),
    (-4, -1),
    (5, -5),
    (-5, 1),
    (5, -3),
    (-4, 1),
    (2, 4),
    (0, -2),
    (-3, 2),
    (3, 2),
    (5, -4),
    (4, -3),
    (-5, -2),
    (-3, 0),
    (-2, -2),
    (2, 2),
    (-4, 0),
    (0, -1),
    (-5, -1),
    (-4, 4),
    (5, 2),
    (-5, 2),
    (4, 5),
    (5, 1),
    (2, 0),
    (-1, 0),
    (-1, 2),
    (2, -3),
    (2, -2),
    (5, 3),
    (3, 0),
    (-1, -3),
    (-3, 3),
    (-2, 4),
    (-3, 5),
    (-2, -1),
    (-3, -3),
    (-3, -4),
    (-4, -3),
    (2, 5),
    (1, -2),
    (-4, 3),
    (2, 3),
    (-2, 1),
    (2, -4),
    (-1, 4),
    (-1, -2),
    (1, -4),
    (0, 1),
    (0, -5),
    (1, 3),
    (1, 5),
    (3, 5),
    (-2, 3),
    (2, 1),
    (4, 3),
    (-5, 5),
    (0, 3),
    (-2, 2),
    (1, -3),
    (-1, -4),
    (1, 1),
    (5, 4),
    (5, -1),
    (-1, 1),
    (-1, 5),
    (3, -3),
    (3, -5),
    (4, -1),
    (-5, 0),
    (-5, -4),
    (-1, -5),
    (0, -3),
    (4, 0),
    (2, -5),
    (3, 1),
    (-3, -1),
    (-4, -4),
    (-3, 1),
    (1, 4),
    (4, -5),
    (1, -5),
    (1, 2),
    (1, -1),
    (3, -4),
    (3, -2),
];

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
    img
}

#[test]
fn poseidon_nether_live_order_matches() {
    let dir = std::env::var("POSEIDON_HELL_LIVE_DUMPS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/hell_live_dumps"));
    if !dir.exists() {
        eprintln!("nether live dumps missing, skipping: {}", dir.display());
        return;
    }

    let seed = 42i64;
    let config = WorldConfig {
        seed,
        save_enabled: false,
        ..WorldConfig::default()
    };
    let mut world = World::new_nether(config);
    world.live_enable();
    for &(cx, cz) in LOAD_ORDER {
        world.server_get_chunk_at(ChunkPos::new(cx, cz));
    }

    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.starts_with("chunk_") || !name.ends_with(".bin") {
            continue;
        }
        let stem = name.trim_start_matches("chunk_").trim_end_matches(".bin");
        let mut parts = stem.split('_');
        let cx: i32 = parts.next().unwrap().parse().unwrap();
        let cz: i32 = parts.next().unwrap().parse().unwrap();
        let java = std::fs::read(&path).unwrap();
        let chunk = world
            .get_chunk(ChunkPos::new(cx, cz))
            .unwrap_or_else(|| panic!("missing rust chunk ({cx},{cz})"));
        let img = build_dump_image(chunk);
        if img != java {
            let mut blocks = 0;
            let mut meta = 0;
            let mut height = 0;
            for i in 0..32768 {
                if img[i] != java[i] {
                    blocks += 1;
                }
            }
            for i in 32768..32768 + 16384 {
                if img[i] != java[i] {
                    meta += 1;
                }
            }
            for i in 32768 + 16384..img.len() {
                if img[i] != java[i] {
                    height += 1;
                }
            }
            panic!(
                "nether live mismatch chunk ({cx},{cz}): blocks={blocks} meta={meta} height={height}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 121, "expected 121 loaded chunks");
}

#[allow(dead_code)]
fn dump_dir() -> &'static Path {
    Path::new("/tmp/hell_live_dumps")
}
