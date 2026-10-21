//! The import of the worlds of the port that used the regions of Anvil.
//!
//! The reference stores the chunks in the files
//! `<world>/<x & 63>/<z & 63>/c.<x>.<z>.dat` (`ChunkLoader`), the port used
//! the regions of Anvil before.  The import reads the regions of the old
//! format and writes the chunks in the format of the reference; the chunks
//! that already exist in the new format are not touched.

use std::path::Path;

use crate::util::Result;
use crate::world::storage::chunk_file::ChunkFileCache;
use crate::world::storage::region::RegionFile;

/// Imports the chunks of the regions of Anvil into the format of the
/// reference.
///
/// The method returns the number of the imported chunks.
pub fn import_regions(world_dir: &Path, storage: &mut ChunkFileCache) -> Result<usize> {
    let region_dir = world_dir.join("region");
    if !region_dir.is_dir() {
        return Ok(0);
    }
    let mut names: Vec<std::path::PathBuf> = match std::fs::read_dir(&region_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension().is_some_and(|extension| extension == "mcr")
                    || path.extension().is_some_and(|extension| extension == "mca")
            })
            .collect(),
        Err(_) => return Ok(0),
    };
    names.sort();

    let mut imported = 0;
    for path in names {
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        // The name of the region is `r.<x>.<z>`.
        let mut parts = name.split('.');
        if parts.next() != Some("r") {
            continue;
        }
        let (Some(region_x), Some(region_z)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(region_x), Ok(region_z)) = (region_x.parse::<i32>(), region_z.parse::<i32>())
        else {
            continue;
        };
        let Ok(mut region) = RegionFile::open(&path) else {
            continue;
        };
        for local_z in 0..32 {
            for local_x in 0..32 {
                if !region.has_chunk(local_x, local_z) {
                    continue;
                }
                let chunk_x = region_x * 32 + local_x;
                let chunk_z = region_z * 32 + local_z;
                if storage.has_chunk(chunk_x, chunk_z) {
                    continue;
                }
                let Ok(Some(nbt)) = region.read_chunk(local_x, local_z) else {
                    continue;
                };
                storage.write_chunk(chunk_x, chunk_z, &nbt)?;
                imported += 1;
            }
        }
    }
    Ok(imported)
}
