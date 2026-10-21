//! The storage of the chunks of the reference.
//!
//! Java `ChunkLoader`:
//! - the file of the chunk is `<world>/<x & 63 in base 36>/<z & 63 in
//!   base 36>/c.<x in base 36>.<z in base 36>.dat` ✓,
//! - the file holds the NBT of the chunk, compressed by gzip ✓,
//! - the file is written as `tmp_chunk.dat` in the world directory and
//!   then renamed to its place ✓.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::util::Result;
use crate::world::storage::nbt::NbtTag;

/// Java `Integer.toString(int, int)`: the number in the base thirty six.
///
/// The negative numbers keep the sign, like in Java.
pub fn java_base36(value: i32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let negative = value < 0;
    let mut rest = value.unsigned_abs();
    let mut out = Vec::new();
    while rest > 0 {
        out.push(DIGITS[(rest % 36) as usize]);
        rest /= 36;
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).expect("the digits of the base thirty six are ASCII")
}

/// The cache of the files of the chunks.
pub struct ChunkFileCache {
    world_dir: PathBuf,
    /// The chunks that were written and are not saved on the disk yet.
    dirty: HashMap<(i32, i32), ()>,
}

impl ChunkFileCache {
    pub fn new(world_dir: &Path) -> Self {
        Self {
            world_dir: world_dir.to_path_buf(),
            dirty: HashMap::new(),
        }
    }

    /// Java `ChunkLoader.a(int, int)`: the file of the chunk.
    pub fn chunk_path(&self, chunk_x: i32, chunk_z: i32) -> PathBuf {
        let name = format!("c.{}.{}.dat", java_base36(chunk_x), java_base36(chunk_z));
        self.world_dir
            .join(java_base36(chunk_x & 63))
            .join(java_base36(chunk_z & 63))
            .join(name)
    }

    pub fn has_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> bool {
        self.chunk_path(chunk_x, chunk_z).exists()
    }

    pub fn read_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> Result<Option<NbtTag>> {
        let path = self.chunk_path(chunk_x, chunk_z);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let (_name, nbt) = NbtTag::from_gzip(&bytes)?;
        Ok(Some(nbt))
    }

    /// Java `ChunkLoader.a(World, Chunk)`: the file is written as
    /// `tmp_chunk.dat` and then renamed to its place.
    pub fn write_chunk(&mut self, chunk_x: i32, chunk_z: i32, nbt: &NbtTag) -> Result<()> {
        let path = self.chunk_path(chunk_x, chunk_z);
        let directory = path
            .parent()
            .expect("the file of the chunk has a directory")
            .to_path_buf();
        std::fs::create_dir_all(&directory)?;

        let bytes = nbt.to_gzip("")?;
        let temporary = self.world_dir.join("tmp_chunk.dat");
        std::fs::write(&temporary, &bytes)?;
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::rename(&temporary, &path)?;
        self.dirty.insert((chunk_x, chunk_z), ());
        Ok(())
    }

    pub fn flush_all(&mut self) -> Result<()> {
        self.dirty.clear();
        Ok(())
    }

    pub fn close_all(&mut self) {
        let _ = self.flush_all();
        self.dirty.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_of_the_reference_matches_the_base_of_java() {
        assert_eq!(java_base36(0), "0");
        assert_eq!(java_base36(1), "1");
        assert_eq!(java_base36(35), "z");
        assert_eq!(java_base36(36), "10");
        assert_eq!(java_base36(63), "1r");
        assert_eq!(java_base36(-1), "-1");
        assert_eq!(java_base36(-36), "-10");
    }

    #[test]
    fn the_file_of_the_chunk_has_the_name_of_the_reference() {
        let cache = ChunkFileCache::new(Path::new("/tmp/world"));
        assert_eq!(
            cache.chunk_path(0, 0),
            PathBuf::from("/tmp/world/0/0/c.0.0.dat")
        );
        assert_eq!(
            cache.chunk_path(-1, 35),
            PathBuf::from("/tmp/world/1r/z/c.-1.z.dat")
        );
        // Sixty four in the base thirty six is "1s".
        assert_eq!(
            cache.chunk_path(64, 64),
            PathBuf::from("/tmp/world/0/0/c.1s.1s.dat")
        );
    }
}
