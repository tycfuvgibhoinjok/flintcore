//! Nether (`DIM-1`) parity harness against Project-Poseidon dumps.
//!
//! Reference dumps are produced by `tools/poseidon_dump/PoseidonHellGenerator.java`;
//! see `tests/golden/poseidon_nether_hashes.txt` for the JVM-free regression.
//! This test compares full 49,664-byte images and is skipped when the dumps
//! directory is absent.

use flintcore::config::WorldConfig;
use flintcore::world::chunk::{Chunk, ChunkPos, CHUNK_WIDTH};
use flintcore::world::world::World;
use std::path::{Path, PathBuf};

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

fn dumps_dir() -> PathBuf {
    std::env::var("POSEIDON_HELL_DUMPS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/hell_dumps"))
}

fn seeds() -> Vec<i64> {
    std::env::var("POSEIDON_SEEDS")
        .map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect())
        .unwrap_or_else(|_| vec![12345])
}

#[test]
fn poseidon_nether_dumps_match() {
    let dir = dumps_dir();
    if !dir.exists() {
        eprintln!("nether dumps dir missing, skipping byte-level check");
        return;
    }
    for seed in seeds() {
        let config = WorldConfig {
            seed,
            save_enabled: false,
            ..WorldConfig::default()
        };
        let mut world = World::new_nether(config);
        for cz in -1..=1 {
            for cx in -1..=1 {
                world.populate_chunk(ChunkPos::new(cx, cz));
            }
        }
        for cz in -1..=1 {
            for cx in -1..=1 {
                let path = dir.join(format!("chunk_{seed}_{cx}_{cz}.bin"));
                let Ok(java) = std::fs::read(&path) else {
                    eprintln!("missing {}", path.display());
                    continue;
                };
                let chunk = world.get_chunk(ChunkPos::new(cx, cz)).expect("chunk");
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
                        "nether mismatch seed {seed} chunk ({cx},{cz}): blocks={blocks} meta={meta} height={height}"
                    );
                }
            }
        }
    }
}

#[allow(dead_code)]
fn reference_path() -> &'static Path {
    Path::new("/tmp/hell_dumps")
}
