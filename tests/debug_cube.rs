use flintcore::config::WorldConfig;
use flintcore::world::chunk::ChunkPos;
use flintcore::world::world::World;

/// Debug helper: prints a cube of block ids / metadata from the generated world.
/// Environment: DEBUG_SEED, DEBUG_X0, DEBUG_X1, DEBUG_Y0, DEBUG_Y1, DEBUG_Z0, DEBUG_Z1.
#[test]
fn debug_cube() {
    let seed: i64 = std::env::var("DEBUG_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let x0: i32 = std::env::var("DEBUG_X0")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let x1: i32 = std::env::var("DEBUG_X1")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let y0: i32 = std::env::var("DEBUG_Y0")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let y1: i32 = std::env::var("DEBUG_Y1")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(127);
    let z0: i32 = std::env::var("DEBUG_Z0")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let z1: i32 = std::env::var("DEBUG_Z1")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);

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

    for x in x0..=x1 {
        for z in z0..=z1 {
            let mut ids = Vec::new();
            let mut metas = Vec::new();
            for y in y0..=y1 {
                ids.push(format!("{:3}", world.get_block(x, y, z)));
                metas.push(format!("{:2}", world.get_data(x, y, z)));
            }
            println!("x={x} z={z} ids: {}", ids.join(" "));
            println!("x={x} z={z} met: {}", metas.join(" "));
        }
    }
}
