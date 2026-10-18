//! Live-path acceptance harness: raw McRegion (`.mcr`) world comparison.
//!
//! Reference worlds live under `.references/generation` (override with
//! `FLINTCORE_REFERENCES`); raw files are intentionally not committed, so the
//! tests auto-skip when the directory is missing.
//!
//! Two kinds of checks live here:
//! * `golden_poseidon_manifest` — Java-free regression over the reference
//!   world itself (chunk list + per-chunk content hashes). Regenerate with
//!   `POSEIDON_LIVE_UPDATE=1` when the reference is intentionally replaced.
//! * `report_reference_vs_live` — informational diff between the Poseidon
//!   reference world and our current live output (FlintCore world). Never
//!   fails; used as the divergence tracker while the live path is ported.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flintcore::util::compress::{inflate_gzip, inflate_zlib};
use flintcore::world::storage::nbt::NbtTag;

/// A chunk decoded straight from a region file: blocks/meta/heightmap in
/// wire order, plus the persisted `TerrainPopulated` flag.
#[derive(Clone)]
struct RawChunk {
    blocks: Vec<u8>,
    data: Vec<u8>,
    height: Vec<u8>,
    populated: bool,
}

impl RawChunk {
    fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in self.blocks.iter().chain(&self.data).chain(&self.height) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
}

fn references_root() -> PathBuf {
    std::env::var("FLINTCORE_REFERENCES")
        .unwrap_or_else(|_| ".references/generation".into())
        .into()
}

fn read_world(dir: &Path) -> BTreeMap<(i32, i32), RawChunk> {
    let region_dir = dir.join("region");
    let mut out = BTreeMap::new();
    let entries = match fs::read_dir(&region_dir) {
        Ok(e) => e,
        Err(err) => panic!("cannot read {}: {err}", region_dir.display()),
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if !name.starts_with("r.") || !name.ends_with(".mcr") {
            continue;
        }
        read_region(&path, &mut out);
    }
    out
}

/// Read one region file. Chunk coordinates are taken from the stored NBT
/// (`xPos`/`zPos`), not from the header index, so a stray chunk cannot hide.
fn read_region(path: &Path, out: &mut BTreeMap<(i32, i32), RawChunk>) {
    let mut f = fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut header = [0u8; 4096];
    f.read_exact(&mut header).unwrap();

    for idx in 0..1024usize {
        let e = u32::from_be_bytes([
            header[idx * 4],
            header[idx * 4 + 1],
            header[idx * 4 + 2],
            header[idx * 4 + 3],
        ]);
        if e == 0 {
            continue;
        }
        let sector = (e >> 8) as u64;
        f.seek(SeekFrom::Start(sector * 4096)).unwrap();
        let mut lenb = [0u8; 4];
        f.read_exact(&mut lenb).unwrap();
        let len = u32::from_be_bytes(lenb) as usize;
        assert!(len >= 1, "bad chunk length in {}", path.display());
        let mut ver = [0u8; 1];
        f.read_exact(&mut ver).unwrap();
        let mut comp = vec![0u8; len - 1];
        f.read_exact(&mut comp).unwrap();
        let raw = match ver[0] {
            1 => inflate_gzip(&comp, 1 << 22).unwrap(),
            2 => inflate_zlib(&comp, 1 << 22).unwrap(),
            v => panic!("unknown region compression version {v} in {}", path.display()),
        };
        let (_name, nbt) = NbtTag::from_bytes(&raw).unwrap();
        let level = nbt
            .get("Level")
            .unwrap_or_else(|| panic!("chunk without Level in {}", path.display()));
        let chx = level.get_i32("xPos").unwrap();
        let chz = level.get_i32("zPos").unwrap();
        let blocks = level.get_byte_array("Blocks").unwrap().to_vec();
        let data = level.get_byte_array("Data").unwrap().to_vec();
        let height = level.get_byte_array("HeightMap").unwrap().to_vec();
        let populated = matches!(level.get("TerrainPopulated"), Some(NbtTag::Byte(v)) if *v != 0);
        assert_eq!(blocks.len(), 32768, "chunk ({chx},{chz}) bad Blocks length");
        assert_eq!(data.len(), 16384, "chunk ({chx},{chz}) bad Data length");
        assert_eq!(height.len(), 256, "chunk ({chx},{chz}) bad HeightMap length");
        out.insert(
            (chx, chz),
            RawChunk {
                blocks,
                data,
                height,
                populated,
            },
        );
    }
}

fn build_manifest(dir: &Path) -> String {
    let world = read_world(dir);
    let mut text = String::new();
    let _ = writeln!(text, "# flintcore live-world manifest (blocks+data+heightmap fnv1a64)");
    let _ = writeln!(text, "# dir: {}", dir.display());
    let _ = writeln!(text, "# chunks: {}", world.len());
    for (&(cx, cz), chunk) in &world {
        let _ = writeln!(
            text,
            "{cx} {cz} {:016x} {}",
            chunk.hash(),
            if chunk.populated { 1 } else { 0 }
        );
    }
    text
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/poseidon_live_manifest.txt")
}

#[test]
fn golden_poseidon_manifest() {
    let dir = references_root().join("poseidon");
    if !dir.join("region").exists() {
        eprintln!("reference world {} not found, skipping", dir.display());
        return;
    }
    let level = flintcore::world::storage::LevelData::load_from_dir(&dir)
        .unwrap()
        .expect("reference level.dat");
    assert_eq!(level.seed, 222);
    assert_eq!((level.spawn_x, level.spawn_y, level.spawn_z), (-4, 64, -235));
    assert_eq!(level.time, 771);

    let player_dat = dir.join("players/13495f67-7e70-397d-a647-53d7b52b9b1a.dat");
    if player_dat.exists() {
        let compressed = fs::read(&player_dat).unwrap();
        let (_name, nbt) = NbtTag::from_gzip(&compressed).unwrap();
        if let Some(NbtTag::List(pos)) = nbt.get("Pos") {
            let coords: Vec<f64> = pos
                .iter()
                .map(|t| match t {
                    NbtTag::Double(v) => *v,
                    _ => panic!("expected Double in Pos"),
                })
                .collect();
            assert_eq!(coords, vec![18.503963472306527, 64.0, -277.28852702972694]);
        }
        if let Some(NbtTag::List(rot)) = nbt.get("Rotation") {
            let angles: Vec<f32> = rot
                .iter()
                .map(|t| match t {
                    NbtTag::Float(v) => *v,
                    _ => panic!("expected Float in Rotation"),
                })
                .collect();
            assert_eq!(angles, vec![-0.30029296875_f32, 84.44998931884766_f32]);
        }
    }

    let manifest = build_manifest(&dir);
    let golden = golden_path();

    if std::env::var("POSEIDON_LIVE_UPDATE").is_ok() {
        fs::write(&golden, &manifest).unwrap();
        eprintln!("updated {}", golden.display());
        return;
    }

    let expected = fs::read_to_string(&golden).unwrap_or_default();
    if expected.is_empty() {
        panic!(
            "missing golden {} — generate it with POSEIDON_LIVE_UPDATE=1",
            golden.display()
        );
    }
    if expected != manifest {
        let exp: Vec<&str> = expected.lines().filter(|l| !l.starts_with('#')).collect();
        let act: Vec<&str> = manifest.lines().filter(|l| !l.starts_with('#')).collect();
        let mut report = String::new();
        let _ = writeln!(report, "manifest lines: expected {}, actual {}", exp.len(), act.len());
        for (i, (e, a)) in exp.iter().zip(act.iter()).enumerate() {
            if e != a {
                let _ = writeln!(report, "first diff at line {i}: expected `{e}` actual `{a}`");
                break;
            }
        }
        panic!("poseidon manifest mismatch vs golden:\n{report}");
    }
}

struct DiffStats {
    shared: usize,
    blocks: u64,
    meta: u64,
    height: u64,
    populated_mismatch: usize,
    only_reference: Vec<(i32, i32)>,
    only_actual: Vec<(i32, i32)>,
}

fn compare_worlds(
    reference: &BTreeMap<(i32, i32), RawChunk>,
    actual: &BTreeMap<(i32, i32), RawChunk>,
) -> DiffStats {
    let mut stats = DiffStats {
        shared: 0,
        blocks: 0,
        meta: 0,
        height: 0,
        populated_mismatch: 0,
        only_reference: Vec::new(),
        only_actual: Vec::new(),
    };
    for pos in reference.keys() {
        if !actual.contains_key(pos) {
            stats.only_reference.push(*pos);
        }
    }
    for pos in actual.keys() {
        if !reference.contains_key(pos) {
            stats.only_actual.push(*pos);
        }
    }
    for (pos, r) in reference {
        let Some(a) = actual.get(pos) else { continue };
        stats.shared += 1;
        if r.populated != a.populated {
            stats.populated_mismatch += 1;
        }
        for i in 0..32768 {
            if r.blocks[i] != a.blocks[i] {
                stats.blocks += 1;
            }
        }
        for i in 0..16384 {
            if r.data[i] != a.data[i] {
                stats.meta += 1;
            }
        }
        for i in 0..256 {
            if r.height[i] != a.height[i] {
                stats.height += 1;
            }
        }
    }
    stats
}

/// Informational: shows how far our current live world (flintcore) is from the
/// Poseidon reference. Never fails; look at the printed report.
#[test]
fn report_reference_vs_live() {
    let root = references_root();
    let reference = root.join("poseidon");
    let actual = root.join("flintcore");
    if !reference.join("region").exists() || !actual.join("region").exists() {
        eprintln!("reference worlds not found, skipping");
        return;
    }
    let reference_chunks = read_world(&reference);
    let actual_chunks = read_world(&actual);

    // The reference spawn-search side chunks (see live-path notes): the only
    // reference chunks with TerrainPopulated=0 outside the prep square.
    let unpopulated: Vec<(i32, i32)> = reference_chunks
        .iter()
        .filter(|(_, c)| !c.populated)
        .map(|(p, _)| *p)
        .collect();
    println!(
        "reference: {} chunks ({} unpopulated), live: {} chunks",
        reference_chunks.len(),
        unpopulated.len(),
        actual_chunks.len()
    );
    println!("reference unpopulated chunks: {unpopulated:?}");

    let stats = compare_worlds(&reference_chunks, &actual_chunks);
    println!(
        "shared {}: block diffs {}, meta diffs {}, height diffs {}, populated mismatches {}",
        stats.shared, stats.blocks, stats.meta, stats.height, stats.populated_mismatch
    );
    println!(
        "only in reference: {} {:?}",
        stats.only_reference.len(),
        &stats.only_reference[..stats.only_reference.len().min(24)]
    );
    println!(
        "only in live: {} {:?}",
        stats.only_actual.len(),
        &stats.only_actual[..stats.only_actual.len().min(24)]
    );
    println!(
        "reference seed/spawn: {:?}",
        flintcore::world::storage::LevelData::load_from_dir(&reference).unwrap()
    );
}

// ---------------------------------------------------------------------------
// Scripted live replay (fitting driver for the Poseidon reference session).
// ---------------------------------------------------------------------------

const REFERENCE_SEED: i64 = 222;

/// The chunks the spawn search generated as a side effect before the spawn
/// prep (forensically established from the reference world: the only
/// candidates outside `prep ∪ (z=-3 row) ∪ (x=11 column)`).
const SEARCH_SIDE_CHUNKS: &[(i32, i32)] = &[
    (-8, 0),
    (-7, 0),
    (-7, -1),
    (-7, -2),
    (-5, -1),
    (-4, -1),
    (-4, -2),
    (-2, -2),
    (-1, -1),
    (-1, -2),
    (0, 0),
];

fn parse_point(text: &str) -> Option<(f64, f64)> {
    let (a, b) = text.trim().split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// `line:x1,z1,x2,z2,step` — a straight walk sampled every `step` blocks.
fn parse_line(spec: &str) -> Option<Vec<(f64, f64)>> {
    let nums: Vec<f64> = spec
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<Vec<f64>>>()?;
    if nums.len() != 5 {
        return None;
    }
    let (x1, z1, x2, z2, step) = (nums[0], nums[1], nums[2], nums[3], nums[4]);
    let dist = ((x2 - x1).powi(2) + (z2 - z1).powi(2)).sqrt();
    let n = (dist / step).ceil() as usize;
    let mut out = Vec::with_capacity(n);
    for i in 1..=n {
        let t = i as f64 / n as f64;
        out.push((x1 + (x2 - x1) * t, z1 + (z2 - z1) * t));
    }
    Some(out)
}

fn sample_first<T: Copy>(items: &[T], n: usize) -> Vec<T> {
    items.iter().take(n).copied().collect()
}

/// Prints the worst-diff chunks and sample block coordinates to make the
/// fitting loop workable.
fn print_diff_detail(
    reference: &BTreeMap<(i32, i32), RawChunk>,
    actual: &BTreeMap<(i32, i32), RawChunk>,
) {
    let mut per_chunk: Vec<((i32, i32), u64, u64, u64)> = Vec::new();
    for (pos, r) in reference {
        let Some(a) = actual.get(pos) else { continue };
        let mut bl = 0u64;
        let mut mt = 0u64;
        let mut hg = 0u64;
        for i in 0..32768 {
            if r.blocks[i] != a.blocks[i] {
                bl += 1;
            }
        }
        for i in 0..16384 {
            if r.data[i] != a.data[i] {
                mt += 1;
            }
        }
        for i in 0..256 {
            if r.height[i] != a.height[i] {
                hg += 1;
            }
        }
        if bl + mt + hg > 0 {
            per_chunk.push((*pos, bl, mt, hg));
        }
    }
    per_chunk.sort_by_key(|&(_, bl, mt, hg)| std::cmp::Reverse(bl + mt + hg));
    println!("REPLAY: chunks with diffs: {}", per_chunk.len());
    for &(pos, bl, mt, hg) in per_chunk.iter().take(12) {
        println!("REPLAY:   chunk {pos:?}: blocks {bl}, meta {mt}, height {hg}");
    }

    // Row histogram: total diffs per cz, with the x-chunks involved.
    let mut rows: BTreeMap<i32, (u32, u64, Vec<i32>)> = BTreeMap::new();
    for &((cx, cz), bl, mt, hg) in &per_chunk {
        let e = rows.entry(cz).or_insert((0, 0, Vec::new()));
        e.0 += 1;
        e.1 += bl + mt + hg;
        e.2.push(cx);
    }
    println!("REPLAY: per-row diffs (cz: chunks, diffs, cx list):");
    for (cz, (n, d, xs)) in &rows {
        println!("REPLAY:   cz={cz}: {n} chunks, {d} diffs, cx={xs:?}");
    }

    // Global classification of block diffs: (ref id, got id) counts.
    let mut global: BTreeMap<(u8, u8), u64> = BTreeMap::new();
    for (pos, r) in reference {
        if let Some(a) = actual.get(pos) {
            for i in 0..32768 {
                if r.blocks[i] != a.blocks[i] {
                    *global.entry((r.blocks[i], a.blocks[i])).or_insert(0) += 1;
                }
            }
        }
    }
    let mut gv: Vec<((u8, u8), u64)> = global.into_iter().collect();
    gv.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    println!("REPLAY: global ref->got block pairs (top 30):");
    for ((rf, gt), n) in gv.iter().take(30) {
        println!("REPLAY:   ref {rf} -> got {gt}: {n}");
    }

    // Breakdown of ref 11 -> got 10 by cz
    let mut lava_cz: BTreeMap<i32, u64> = BTreeMap::new();
    for (pos, r) in reference {
        if let Some(a) = actual.get(pos) {
            for i in 0..32768 {
                if r.blocks[i] == 11 && a.blocks[i] == 10 {
                    *lava_cz.entry(pos.1).or_insert(0) += 1;
                }
            }
        }
    }
    println!("REPLAY: (ref 11 -> got 10) by cz:");
    for (cz, n) in &lava_cz {
        println!("REPLAY:   cz={cz}: {n}");
    }

    if let Some(&(pos, ..)) = per_chunk.first() {
        let r = &reference[&pos];
        let a = &actual[&pos];
        // Local-axis histograms of block diffs.
        let mut by_x = [0u32; 16];
        let mut by_z = [0u32; 16];
        for i in 0..32768 {
            if r.blocks[i] != a.blocks[i] {
                by_x[(i >> 11) & 15] += 1;
                by_z[(i >> 7) & 15] += 1;
            }
        }
        println!("REPLAY: worst chunk {pos:?} block diffs by local x: {:?}", by_x);
        println!("REPLAY: worst chunk {pos:?} block diffs by local z: {:?}", by_z);
        let mut shown = 0;
        for i in (0..32768).step_by(7) {
            if r.blocks[i] != a.blocks[i] {
                // wire index -> x,y,z
                let x = i >> 11;
                let z = (i >> 7) & 15;
                let y = i & 127;
                let wx = pos.0 * 16 + x as i32;
                let wz = pos.1 * 16 + z as i32;
                println!(
                    "REPLAY:     ({wx},{y},{wz}): ref {} got {}",
                    r.blocks[i], a.blocks[i]
                );
                shown += 1;
                if shown >= 16 {
                    break;
                }
            }
        }
    }

    // Optional deep dive on one chunk: FLINTCORE_LIVE_SHOW_CHUNK=cx,cz
    if let Ok(spec) = std::env::var("FLINTCORE_LIVE_SHOW_CHUNK") {
        let nums: Vec<i32> = spec
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if nums.len() == 2 {
            let pos = (nums[0], nums[1]);
            if let (Some(r), Some(a)) = (reference.get(&pos), actual.get(&pos)) {
                let mut pairs: BTreeMap<(u8, u8), u32> = BTreeMap::new();
                let mut by_y = [0u32; 128];
                for i in 0..32768 {
                    if r.blocks[i] != a.blocks[i] {
                        *pairs.entry((r.blocks[i], a.blocks[i])).or_insert(0) += 1;
                        by_y[i & 127] += 1;
                    }
                }
                println!("REPLAY: show chunk {pos:?}: top ref->got pairs:");
                let mut pv: Vec<_> = pairs.into_iter().collect();
                pv.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
                for ((rf, gt), n) in pv.iter().take(20) {
                    println!("REPLAY:   ref {rf} -> got {gt}: {n}");
                }
                let ys: Vec<(usize, u32)> = by_y
                    .iter()
                    .enumerate()
                    .filter(|(_, &n)| n > 0)
                    .map(|(y, &n)| (y, n))
                    .collect();
                println!("REPLAY:   diff y levels: {ys:?}");
                let mut shown = 0;
                for i in (0..32768).step_by(11) {
                    if r.blocks[i] != a.blocks[i] {
                        let x = i >> 11;
                        let z = (i >> 7) & 15;
                        let y = i & 127;
                        println!(
                            "REPLAY:     ({},{y},{}): ref {} got {}",
                            pos.0 * 16 + x as i32,
                            pos.1 * 16 + z as i32,
                            r.blocks[i],
                            a.blocks[i]
                        );
                        shown += 1;
                        if shown >= 24 {
                            break;
                        }
                    }
                }
            } else {
                println!("REPLAY: show chunk {pos:?}: missing in one of the worlds");
            }
        }
    }
}

/// Replay the Poseidon reference session through the live path.
///
/// Gate: `FLINTCORE_LIVE_REPLAY=1`. Optional environment:
/// * `FLINTCORE_LIVE_START=<x,z>` — join position (default spawn + 0.5).
/// * `FLINTCORE_LIVE_ROUTE=<x,z;x,z;...>` — walk samples (default: none).
/// * `FLINTCORE_LIVE_NO_UNLOAD=1` — do not run the per-tick unload pass.
/// * `FLINTCORE_LIVE_EXPECT=exact` — fail unless the worlds match.
#[test]
fn replay_poseidon_live() {
    if std::env::var("FLINTCORE_LIVE_REPLAY").is_err() {
        eprintln!("FLINTCORE_LIVE_REPLAY not set; skipping live replay");
        return;
    }
    let root = references_root();
    let reference_dir = root.join("poseidon");
    if !reference_dir.join("region").exists() {
        eprintln!("reference world {} not found, skipping", reference_dir.display());
        return;
    }
    let level = flintcore::world::storage::LevelData::load_from_dir(&reference_dir)
        .unwrap()
        .expect("reference level.dat");
    assert_eq!(level.seed, REFERENCE_SEED, "unexpected reference seed");
    println!(
        "reference spawn: ({}, {}, {})",
        level.spawn_x, level.spawn_y, level.spawn_z
    );

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/live_replay_world");
    let _ = fs::remove_dir_all(&out_dir);

    let config = flintcore::config::WorldConfig {
        seed: level.seed,
        level_name: out_dir.to_string_lossy().into_owned(),
        save_enabled: true,
        view_distance: 10,
        spawn_x: level.spawn_x,
        spawn_y: level.spawn_y,
        spawn_z: level.spawn_z,
        ..Default::default()
    };
    let mut world = flintcore::world::world::World::new(config);
    world.live_enable();

    // Adopt the spawn-search side effects: by the time the spawn prep runs,
    // these chunks are already loaded (search candidates).
    for &(cx, cz) in SEARCH_SIDE_CHUNKS {
        world.server_get_chunk_at(flintcore::world::chunk::ChunkPos::new(cx, cz));
    }

    world.live_prepare_spawn_region();
    println!("after prep: {} chunks loaded", world.chunk_count());

    let start = std::env::var("FLINTCORE_LIVE_START")
        .ok()
        .and_then(|s| parse_point(&s))
        .unwrap_or((level.spawn_x as f64 + 0.5, level.spawn_z as f64 + 0.5));
    world.live_player_join(1, start.0, start.1);
    println!("join at ({}, {})", start.0, start.1);

    let route = std::env::var("FLINTCORE_LIVE_ROUTE").unwrap_or_default();
    let path_spec = std::env::var("FLINTCORE_LIVE_PATH").unwrap_or_default();
    let mut walk: Vec<(f64, f64)> = Vec::new();
    if let Some(line) = path_spec.strip_prefix("line:") {
        walk = parse_line(line).expect("bad FLINTCORE_LIVE_PATH line spec");
    }
    for part in route.split(';') {
        if part.trim().is_empty() {
            continue;
        }
        walk.push(parse_point(part).expect("bad FLINTCORE_LIVE_ROUTE point"));
    }
    if walk.is_empty() {
        // Optimal fitted frame route for the seed 222 reference world:
        // loads the southern rows -28 and -29 through the exact visible frame
        // sequence (-1, -17) -> (-1, -18) -> (0, -19) -> (1, -18).
        walk = vec![
            (-8.0, -260.0),
            (-8.0, -280.0),
            (4.0, -295.0),
            (18.503963472306527, -277.28852702972694),
        ];
    }

    let no_unload = std::env::var("FLINTCORE_LIVE_NO_UNLOAD").is_ok();
    let trace_frames = std::env::var("FLINTCORE_LIVE_TRACE_FRAMES").is_ok();
    let mut last_anchor = world.live_anchor(1);
    for p in &walk {
        world.live_player_move(1, p.0, p.1);
        world.live_tick_ex(!no_unload);
        if trace_frames {
            let anchor = world.live_anchor(1);
            if anchor != last_anchor {
                if let Some((ax, az)) = anchor {
                    println!(
                        "frame: pos ({ax}, {az}) chunk ({}, {})",
                        (ax as i32) >> 4,
                        (az as i32) >> 4
                    );
                }
                last_anchor = anchor;
            }
        }
    }
    println!("after walk ({} samples): {} chunks loaded", walk.len(), world.chunk_count());

    // The reference session was saved at world time 771; scheduled block
    // ticks (fluids, sand/gravel) were processed once per `doTick`.
    let target_time = std::env::var("FLINTCORE_LIVE_TICKS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(771);
    while world.live_time() < target_time {
        world.live_tick_ex(!no_unload);
    }
    println!("after ticks: time {}, {} chunks loaded", world.live_time(), world.chunk_count());

    world.live_player_quit(1);

    world.save_all().unwrap();

    let reference = read_world(&reference_dir);
    let actual = read_world(&out_dir);
    if Path::new("/tmp/opencode/poseidon_live_222/world/region").exists() {
        let java222 = read_world(Path::new("/tmp/opencode/poseidon_live_222/world"));
        let vs_rust = compare_worlds(&java222, &actual);
        println!(
            "JAVA222 vs RUST: ref {} chunks, live {} chunks, shared {}, blocks {}, meta {}, height {}, pop {}",
            java222.len(),
            actual.len(),
            vs_rust.shared,
            vs_rust.blocks,
            vs_rust.meta,
            vs_rust.height,
            vs_rust.populated_mismatch
        );
        assert_eq!(vs_rust.only_reference.len(), 0);
        assert_eq!(vs_rust.only_actual.len(), 0);
        assert_eq!(vs_rust.populated_mismatch, 0);
        assert_eq!(vs_rust.blocks, 0);
        assert_eq!(vs_rust.meta, 0);
        assert_eq!(vs_rust.height, 0);
    }
    let stats = compare_worlds(&reference, &actual);
    println!(
        "REPLAY: ref {} chunks, live {} chunks, shared {}",
        reference.len(),
        actual.len(),
        stats.shared
    );
    println!(
        "REPLAY: block diffs {}, meta diffs {}, height diffs {}, populated mismatches {}",
        stats.blocks, stats.meta, stats.height, stats.populated_mismatch
    );
    println!(
        "REPLAY: only in reference {}: {:?}",
        stats.only_reference.len(),
        sample_first(&stats.only_reference, 32)
    );
    println!(
        "REPLAY: only in live {}: {:?}",
        stats.only_actual.len(),
        sample_first(&stats.only_actual, 32)
    );
    if stats.blocks + stats.meta + stats.height > 0 {
        print_diff_detail(&reference, &actual);
    }

    if std::env::var("FLINTCORE_LIVE_EXPECT").as_deref() == Ok("exact") {
        assert_eq!(stats.only_reference.len(), 0, "chunks missing from the replay");
        assert_eq!(stats.only_actual.len(), 0, "extra chunks in the replay");
        assert_eq!(stats.populated_mismatch, 0, "populated-flag mismatches");
        assert_eq!(stats.blocks, 0, "block diffs");
        assert_eq!(stats.meta, 0, "meta diffs");
        assert_eq!(stats.height, 0, "height diffs");
    }
}

#[test]
fn controlled_live_spawn_parity() {
    let out_dir = Path::new("/tmp/opencode/flintcore_live_test");
    let _ = fs::remove_dir_all(out_dir);
    let config = flintcore::config::WorldConfig {
        seed: 42,
        level_name: out_dir.to_string_lossy().into_owned(),
        save_enabled: true,
        view_distance: 10,
        ..Default::default()
    };
    let mut world = flintcore::world::world::World::new(config);
    world.live_enable();
    let spawn = world.live_find_spawn(12345);
    println!("FlintCore spawn found at: ({}, 64, {})", spawn.0, spawn.1);
    assert_eq!(spawn, (-45, -3), "spawn location parity with Java PoseidonLive");

    let java_dir = Path::new("/tmp/opencode/poseidon_live_test/world");
    if !java_dir.join("region").exists() {
        return;
    }

    world.live_prepare_spawn_region();
    world.live_player_join(1, spawn.0 as f64 + 0.5, spawn.1 as f64 + 0.5);
    for &(wx, wz) in &[(-50.0, -20.0), (-50.0, -40.0), (-35.0, -55.0), (-20.0, -40.0)] {
        world.live_player_move(1, wx, wz);
        world.live_tick_ex(true);
    }
    while world.live_time() < 300 {
        world.live_tick_ex(true);
    }
    println!("FlintCore after 300 ticks: {} chunks loaded", world.chunk_count());
    world.live_player_quit(1);
    for _ in 0..200 {
        world.live_unload_chunks();
    }
    world.save_all().unwrap();

    let reference = read_world(java_dir);
    let actual = read_world(out_dir);
    let stats = compare_worlds(&reference, &actual);
    println!(
        "CONTROLLED: ref {} chunks, live {} chunks, shared {}",
        reference.len(),
        actual.len(),
        stats.shared
    );
    println!(
        "CONTROLLED: block diffs {}, meta diffs {}, height diffs {}, populated mismatches {}",
        stats.blocks, stats.meta, stats.height, stats.populated_mismatch
    );
    if stats.blocks + stats.meta + stats.height > 0 {
        print_diff_detail(&reference, &actual);
    }
    assert_eq!(stats.only_reference.len(), 0);
    assert_eq!(stats.only_actual.len(), 0);
    assert_eq!(stats.populated_mismatch, 0);
    assert_eq!(stats.blocks, 0);
    assert_eq!(stats.meta, 0);
    assert_eq!(stats.height, 0);
}
