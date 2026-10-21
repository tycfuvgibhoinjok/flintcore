//! The scenarios of the blocks: the trace of the writes of the port against the
//! trace of the reference.
//!
//! The reference (`Project-Poseidon`, Beta 1.7.3) is instrumented with
//! `tools/poseidon_trace/reference-trace.patch` ✓ and the scenarios are run by
//! `tools/poseidon_blocks/PoseidonBlockTrace.java` ✓.  The port prints the same
//! lines from its own write pipeline ✓ (`FLINTCORE_TRACE` ✓).
//!
//! The golden files are regenerated with:
//!
//! ```sh
//! sh tools/poseidon_blocks/regenerate_golden.sh
//! ```

use flintcore::config::WorldConfig;
use flintcore::gameplay::blocks;
use flintcore::util::rng::JavaRandom;
use flintcore::world::chunk::ChunkPos;
use flintcore::world::world::World;
use std::path::Path;

/// The seed of the world of the scenarios.
const SEED: i64 = 42;
/// The number of the ticks of a scenario.
const TICKS: u32 = 20;

/// The number of the ticks of every scenario.
///
/// The lava spreads slower than the water (`BlockFlowing.c` ✓ — the delay of
/// the lava is 30 ticks ✓), so it needs more ticks ✓.
fn ticks_of(scenario: &str) -> u32 {
    match scenario {
        "lava_spread" => 90,
        "fire_spread" => 40,
        _ => TICKS,
    }
}
/// The window of the trace: the columns of the scenario.
const WINDOW: &str = "0,60,0,15,80,15";

/// Runs a scenario and returns the world.
fn run_scenario(scenario: &str) -> World {
    let config = WorldConfig {
        seed: SEED,
        save_enabled: false,
        // The random of the world: the same seed as the reference ✓ (the
        // harness of the reference takes the seed of the world ✓).
        random_seed: format!("fixed:{SEED}"),
        ..WorldConfig::default()
    };
    let mut world = World::new(config);
    // The trace of the scenario starts after the generation of the world ✓
    // (the reference arms the trace by hand ✓).
    flintcore::world::world::set_trace_armed(false);
    for cz in -1..=1 {
        for cx in -1..=1 {
            world.populate_chunk(ChunkPos::new(cx, cz));
        }
    }

    // The floor of the scenario: the same on both sides ✓.
    //
    // The physics is suppressed while the floor is made: the blocks of the
    // terrain that die would drop an item ✓ and the drop of the item needs the
    // server of CraftBukkit ✓ — the world of the generator has no server ✓.
    world.set_suppress_physics(true);
    // The trace covers the floor of the scenario ✓.
    flintcore::world::world::set_trace_armed(true);
    for x in 0..16 {
        for z in 0..16 {
            for y in 64..80 {
                world.java_set_type_id_and_data(x, y, z, 0, 0);
            }
            world.java_set_type_id_and_data(x, 63, z, 1, 0);
        }
    }
    world.set_suppress_physics(false);

    match scenario {
        "water_spread" => {
            world.java_set_type_id_and_data(8, 64, 8, 9, 0);
        }
        "lava_into_water" => {
            world.java_set_type_id_and_data(8, 64, 8, 11, 0);
            world.java_set_type_id_and_data(9, 64, 8, 9, 0);
        }
        "water_into_lava" => {
            world.java_set_type_id_and_data(8, 64, 8, 9, 0);
            world.java_set_type_id_and_data(9, 64, 8, 11, 0);
        }
        "sand_fall" => {
            world.java_set_type_id_and_data(8, 64, 8, 0, 0);
            for y in 65..=67 {
                world.java_set_type_id_and_data(8, y, 8, 12, 0);
            }
        }
        "lava_spread" => {
            world.java_set_type_id_and_data(8, 64, 8, 11, 0);
        }
        "fire_spread" => {
            // The planks of the floor and the fire above them.
            for x in 5..=11 {
                world.java_set_type_id_and_data(x, 64, 8, blocks::WOOD_PLANKS, 0);
            }
            world.java_set_type_id_and_data(8, 65, 8, blocks::FIRE, 0);
        }
        "redstone_torch" => {
            // The torch of the redstone on the floor and the wire next to it.
            world.java_set_type_id_and_data(8, 64, 8, blocks::REDSTONE_TORCH_OFF, 5);
            world.java_set_type_id_and_data(9, 64, 8, blocks::REDSTONE_WIRE, 0);
        }
        "piston_push" => {
            // The piston, the block above it and the lever that powers it.
            world.java_set_type_id_and_data(8, 64, 8, blocks::PISTON, 0);
            world.java_set_type_id_and_data(8, 65, 8, blocks::STONE, 0);
            world.java_set_type_id_and_data(9, 64, 8, blocks::LEVER, 9);
        }
        other => panic!("unknown scenario {other}"),
    }

    // The ticks of the world: the scheduled ticks and the random ticks
    // (`World.doTick()`).  The reference has no player in the harness, so the
    // random ticks do not run there ✓ — the same on both sides ✓.
    let mut entity_random = JavaRandom::new(SEED);
    for _ in 0..ticks_of(scenario) {
        // Java `World.doTick()`: the time of the world is advanced before the
        // scheduled ticks are processed ✓.
        world.advance_live_time();
        world.java_process_scheduled(false);
        world.tick_random_blocks(&[], &mut entity_random);
    }

    world
}

/// The scenarios whose trace must match the reference exactly ✓.
const SCENARIOS: &[&str] = &[
    "water_spread",
    "lava_into_water",
    "water_into_lava",
    "sand_fall",
    "lava_spread",
    "fire_spread",
    "piston_push",
];

/// The scenarios with a known divergence ✓.
///
/// The order of the notifications of the redstone wire ✓: the reference
/// iterates the set of the notifications in the order of `java.util.HashSet` ✓
/// (`BlockRedstoneWire.b` ✓ — the hash of `ChunkPosition` ✓), the port
/// iterates its own set ✓ ⇒ the same events in a different order ✓ (see
/// W-53 of the plan ✓).
const KNOWN_DIVERGENCES: &[&str] = &["redstone_torch"];

#[test]
fn the_scenarios_of_the_blocks_match_the_reference() {
    for scenario in SCENARIOS.iter().chain(KNOWN_DIVERGENCES.iter()) {
        let out = std::env::temp_dir().join(format!("flintcore_block_trace_{scenario}.txt"));
        let _ = std::fs::remove_file(&out);
        std::env::set_var("FLINTCORE_TRACE", WINDOW);
        std::env::set_var("FLINTCORE_TRACE_OUT", &out);

        let _world = run_scenario(scenario);

        let trace = std::fs::read_to_string(&out).expect("the trace of the port");
        let golden_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/poseidon_block_traces")
            .join(format!("{scenario}.txt"));
        let golden = std::fs::read_to_string(&golden_path).expect("the trace of the reference");

        assert_eq!(
            trace.lines().count(),
            golden.lines().count(),
            "the trace of the scenario {scenario} has a different number of events"
        );
        if KNOWN_DIVERGENCES.contains(scenario) {
            // The known divergence: the same events in a different order ✓.
            let mut actual: Vec<&str> = trace.lines().collect();
            let mut expected: Vec<&str> = golden.lines().collect();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(
                actual, expected,
                "the trace of the scenario {scenario} has other differences than the order"
            );
            continue;
        }
        for (index, (actual, expected)) in trace.lines().zip(golden.lines()).enumerate() {
            assert_eq!(
                actual,
                expected,
                "the trace of the scenario {scenario} differs at the line {}",
                index + 1
            );
        }
    }
}
