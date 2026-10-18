use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::util::Result;
use crate::world::storage::nbt::NbtTag;
use crate::world::storage::region::RegionFile;

pub struct RegionFileCache {
    region_dir: PathBuf,
    regions: HashMap<(i32, i32), RegionFile>,
}

impl RegionFileCache {
    pub fn new(world_dir: &Path) -> Self {
        let region_dir = world_dir.join("region");
        Self {
            region_dir,
            regions: HashMap::new(),
        }
    }

    fn region_path(&self, rx: i32, rz: i32) -> PathBuf {
        self.region_dir.join(format!("r.{rx}.{rz}.mcr"))
    }

    pub fn get_or_open(&mut self, rx: i32, rz: i32) -> Result<&mut RegionFile> {
        let key = (rx, rz);
        if !self.regions.contains_key(&key) {
            if self.regions.len() >= 32 {
                self.close_all();
            }
            std::fs::create_dir_all(&self.region_dir)?;
            let path = self.region_path(rx, rz);
            let region = RegionFile::open(&path)?;
            self.regions.insert(key, region);
        }
        Ok(self.regions.get_mut(&key).unwrap())
    }

    pub fn has_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> bool {
        let rx = chunk_x >> 5;
        let rz = chunk_z >> 5;
        let path = self.region_path(rx, rz);
        if !path.exists() && !self.regions.contains_key(&(rx, rz)) {
            return false;
        }
        if let Ok(region) = self.get_or_open(rx, rz) {
            region.has_chunk(chunk_x & 31, chunk_z & 31)
        } else {
            false
        }
    }

    pub fn read_chunk(&mut self, chunk_x: i32, chunk_z: i32) -> Result<Option<NbtTag>> {
        let rx = chunk_x >> 5;
        let rz = chunk_z >> 5;
        let path = self.region_path(rx, rz);
        if !path.exists() && !self.regions.contains_key(&(rx, rz)) {
            return Ok(None);
        }
        let region = self.get_or_open(rx, rz)?;
        region.read_chunk(chunk_x & 31, chunk_z & 31)
    }

    pub fn write_chunk(&mut self, chunk_x: i32, chunk_z: i32, nbt: &NbtTag) -> Result<()> {
        let rx = chunk_x >> 5;
        let rz = chunk_z >> 5;
        let region = self.get_or_open(rx, rz)?;
        region.write_chunk(chunk_x & 31, chunk_z & 31, nbt)
    }

    pub fn flush_all(&mut self) -> Result<()> {
        for region in self.regions.values_mut() {
            region.sync()?;
        }
        Ok(())
    }

    pub fn close_all(&mut self) {
        let _ = self.flush_all();
        self.regions.clear();
        self.regions.shrink_to_fit();
    }
}
