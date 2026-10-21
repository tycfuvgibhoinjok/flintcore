//! The trace of the block writes of the port against the trace of the reference.
//!
//! The reference (`Project-Poseidon`, Beta 1.7.3) is instrumented with
//! `tools/poseidon_trace/reference-trace.patch` ✓: every block write of the
//! reference is printed with the coordinates, the old and the new value ✓.
//! The port prints the same lines from its own write pipeline
//! (`FLINTCORE_TRACE` ✓ in `src/world/world.rs` ✓).
//!
//! The golden file is the trace of the reference:
//!
//! ```sh
//! sh tools/poseidon_trace/regenerate_golden.sh
//! ```

use flintcore::config::WorldConfig;
use flintcore::world::chunk::ChunkPos;
use flintcore::world::world::World;
use std::path::Path;

/// The seed of the world of the trace.
const SEED: i64 = 42;
/// The window of the trace: the columns of the region, from the floor to the
/// top of the surface.
const WINDOW: &str = "0,60,0,15,80,15";

#[test]
fn the_trace_of_the_blocks_matches_the_reference() {
    let out = std::env::temp_dir().join("flintcore_block_trace.txt");
    let _ = std::fs::remove_file(&out);
    std::env::set_var("FLINTCORE_TRACE", WINDOW);
    std::env::set_var("FLINTCORE_TRACE_OUT", &out);

    let config = WorldConfig {
        seed: SEED,
        save_enabled: false,
        ..WorldConfig::default()
    };
    let mut world = World::new(config);
    for cz in -1..=1 {
        for cx in -1..=1 {
            world.populate_chunk(ChunkPos::new(cx, cz));
        }
    }

    let trace = std::fs::read_to_string(&out).expect("the trace of the port");
    let golden_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/poseidon_block_trace.txt");
    let golden = std::fs::read_to_string(&golden_path).expect("the trace of the reference");

    assert_eq!(
        trace.lines().count(),
        golden.lines().count(),
        "the trace of the port has a different number of events"
    );
    for (index, (actual, expected)) in trace.lines().zip(golden.lines()).enumerate() {
        assert_eq!(
            actual,
            expected,
            "the trace of the port differs from the reference at the line {}",
            index + 1
        );
    }
}
