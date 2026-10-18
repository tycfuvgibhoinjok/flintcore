use flintcore::config::WorldConfig;
use flintcore::world::chunk::{ChunkPos, CHUNK_HEIGHT, CHUNK_WIDTH};
use flintcore::world::world::World;
use std::fs;
use std::path::Path;

#[derive(Default, Debug)]
struct MismatchStats {
    blocks: i32,
    meta: i32,
    height: i32,
}

fn check_chunk_match(world: &World, cx: i32, cz: i32, dump_path: &Path) -> MismatchStats {
    let data = fs::read(dump_path).expect("failed to read poseidon dump");
    assert_eq!(data.len(), 32768 + 16384 + 256);

    let pos = ChunkPos::new(cx, cz);
    let chunk = world.get_chunk(pos).expect("chunk not found in world");

    let expected_blocks = &data[0..32768];
    let expected_meta = &data[32768..32768 + 16384];
    let expected_height = &data[32768 + 16384..];

    let mut stats = MismatchStats::default();

    for x in 0..CHUNK_WIDTH {
        for z in 0..CHUNK_WIDTH {
            for y in 0..CHUNK_HEIGHT {
                let wire_idx = (x << 11) | (z << 7) | y;
                let exp_id = expected_blocks[wire_idx];
                let act_id = chunk.get_block(x, y, z);

                let meta_idx = wire_idx >> 1;
                let exp_meta = if (wire_idx & 1) == 0 {
                    expected_meta[meta_idx] & 0x0F
                } else {
                    (expected_meta[meta_idx] >> 4) & 0x0F
                };
                let act_meta = chunk.get_metadata(x, y, z);

                if exp_id != act_id {
                    if stats.blocks < 10 {
                        println!(
                            "Block mismatch at chunk ({cx},{cz}) local ({x},{y},{z}) world ({},{},{}): expected id={exp_id}, actual id={act_id}",
                            cx * 16 + x as i32,
                            y,
                            cz * 16 + z as i32
                        );
                    }
                    stats.blocks += 1;
                } else if exp_meta != act_meta {
                    if stats.meta < 10 {
                        println!(
                            "Meta mismatch at chunk ({cx},{cz}) local ({x},{y},{z}) world ({},{},{}): block={act_id}, expected meta={exp_meta}, actual meta={act_meta}",
                            cx * 16 + x as i32,
                            y,
                            cz * 16 + z as i32
                        );
                    }
                    stats.meta += 1;
                }
            }

            let height_idx = (z << 4) | x;
            let exp_height = expected_height[height_idx];
            let act_height = chunk.get_height(x, z);
            if exp_height != act_height {
                if stats.height < 10 {
                    println!(
                        "Height mismatch at chunk ({cx},{cz}) local ({x},{z}) world ({},{}): expected={exp_height}, actual={act_height}",
                        cx * 16 + x as i32,
                        cz * 16 + z as i32
                    );
                }
                stats.height += 1;
            }
        }
    }

    println!(
        "Chunk ({cx},{cz}): block mismatches = {}, meta mismatches = {}, height mismatches = {}",
        stats.blocks, stats.meta, stats.height
    );
    stats
}

#[test]
fn test_compare_poseidon_chunks() {
    let seeds: Vec<i64> = std::env::var("POSEIDON_SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_else(|| {
            vec![
                12345, 0, 9999, 54321, -12345, 1337, 42, 1, 7, 100, 777, 2024, 31337, 65536,
                777777, -42, -999, 123456789,
            ]
        });
    let dump_dir = std::env::var("POSEIDON_DUMPS").unwrap_or_else(|_| "/tmp/poseidon_dumps".into());
    let dump_dir = Path::new(&dump_dir);
    if !dump_dir.exists() {
        eprintln!("dump dir {} not found, skipping", dump_dir.display());
        return;
    }

    let mut total = MismatchStats::default();

    for &seed in &seeds {
        println!("=== Testing Seed {seed} ===");
        let config = WorldConfig {
            seed,
            save_enabled: false,
            ..WorldConfig::default()
        };
        let mut world = World::new(config);

        // Pre-generate and populate in the exact same order as the Java harness:
        // populate cx from -1 to 1, cz from -1 to 1.
        for cz in -1..=1 {
            for cx in -1..=1 {
                world.populate_chunk(ChunkPos::new(cx, cz));
            }
        }

        for cz in -1..=1 {
            for cx in -1..=1 {
                let path = dump_dir.join(format!("chunk_{seed}_{cx}_{cz}.bin"));
                if path.exists() {
                    let stats = check_chunk_match(&world, cx, cz, &path);
                    total.blocks += stats.blocks;
                    total.meta += stats.meta;
                    total.height += stats.height;
                }
            }
        }
    }

    println!(
        "TOTAL: block mismatches = {}, meta mismatches = {}, height mismatches = {}",
        total.blocks, total.meta, total.height
    );
    assert_eq!(total.blocks, 0, "Total block mismatches across region");
    assert_eq!(total.meta, 0, "Total meta mismatches across region");
    assert_eq!(total.height, 0, "Total height mismatches across region");
}
