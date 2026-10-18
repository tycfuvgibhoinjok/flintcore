use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::compress::{inflate_gzip, inflate_zlib};
use crate::util::{Error, Result};
use crate::world::storage::nbt::NbtTag;

pub const SECTOR_SIZE: usize = 4096;
pub const REGION_CHUNKS: usize = 1024; // 32 * 32

pub struct RegionFile {
    file: File,
    offsets: [u32; REGION_CHUNKS],
    timestamps: [u32; REGION_CHUNKS],
    sector_free: Vec<bool>,
}

impl RegionFile {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let metadata = file.metadata()?;
        let file_len = metadata.len();

        let mut offsets = [0u32; REGION_CHUNKS];
        let mut timestamps = [0u32; REGION_CHUNKS];

        // If file is newly created or smaller than header (8192 bytes), initialize with 0s
        if file_len < 8192 {
            file.seek(SeekFrom::Start(0))?;
            let zero_header = [0u8; 8192];
            file.write_all(&zero_header)?;
            file.flush()?;
        } else {
            // Read offsets table (sector 0: 4096 bytes)
            file.seek(SeekFrom::Start(0))?;
            let mut buf4 = [0u8; 4];
            for entry in &mut offsets {
                file.read_exact(&mut buf4)?;
                *entry = u32::from_be_bytes(buf4);
            }
            // Read timestamps table (sector 1: 4096 bytes)
            for entry in &mut timestamps {
                file.read_exact(&mut buf4)?;
                *entry = u32::from_be_bytes(buf4);
            }
        }

        let total_sectors = (file.metadata()?.len() as usize).div_ceil(SECTOR_SIZE);
        let mut sector_free = vec![true; total_sectors.max(2)];
        sector_free[0] = false; // Sector 0: offset table
        sector_free[1] = false; // Sector 1: timestamp table

        for &entry in &offsets {
            if entry != 0 {
                let sector_offset = (entry >> 8) as usize;
                let sector_count = (entry & 0xFF) as usize;
                if sector_offset + sector_count <= sector_free.len() {
                    for s in &mut sector_free[sector_offset..sector_offset + sector_count] {
                        *s = false;
                    }
                }
            }
        }

        Ok(Self {
            file,
            offsets,
            timestamps,
            sector_free,
        })
    }

    pub fn has_chunk(&self, local_x: i32, local_z: i32) -> bool {
        if !(0..32).contains(&local_x) || !(0..32).contains(&local_z) {
            return false;
        }
        let index = (local_x & 31) + (local_z & 31) * 32;
        self.offsets[index as usize] != 0
    }

    pub fn read_chunk(&mut self, local_x: i32, local_z: i32) -> Result<Option<NbtTag>> {
        if !(0..32).contains(&local_x) || !(0..32).contains(&local_z) {
            return Ok(None);
        }
        let index = ((local_x & 31) + (local_z & 31) * 32) as usize;
        let entry = self.offsets[index];
        if entry == 0 {
            return Ok(None);
        }

        let sector_offset = (entry >> 8) as usize;
        let sector_count = (entry & 0xFF) as usize;

        if sector_offset + sector_count > self.sector_free.len() {
            return Err(Error::Malformed(
                "region chunk sector points beyond file length",
            ));
        }

        self.file
            .seek(SeekFrom::Start((sector_offset * SECTOR_SIZE) as u64))?;

        let mut len_buf = [0u8; 4];
        self.file.read_exact(&mut len_buf)?;
        let length = u32::from_be_bytes(len_buf) as usize;

        if length > sector_count * SECTOR_SIZE || length < 1 {
            return Err(Error::Malformed("invalid chunk length in region file"));
        }

        let mut version_buf = [0u8; 1];
        self.file.read_exact(&mut version_buf)?;
        let version = version_buf[0];

        let compressed_len = length - 1;
        let mut compressed = vec![0u8; compressed_len];
        self.file.read_exact(&mut compressed)?;

        let decompressed = match version {
            1 => inflate_gzip(&compressed, 256 * 1024)?,
            2 => inflate_zlib(&compressed, 256 * 1024)?,
            _ => return Err(Error::Malformed("unknown region chunk compression version")),
        };

        let (_name, nbt) = NbtTag::from_bytes(&decompressed)?;
        Ok(Some(nbt))
    }

    pub fn write_chunk(&mut self, local_x: i32, local_z: i32, nbt: &NbtTag) -> Result<()> {
        if !(0..32).contains(&local_x) || !(0..32).contains(&local_z) {
            return Err(Error::Malformed(
                "chunk coordinates out of bounds for region file",
            ));
        }

        let compressed = nbt.to_zlib("")?;
        let payload_len = compressed.len();
        // 4 bytes length + 1 byte compression version (2) + payload
        let total_chunk_len = payload_len + 5;
        let sectors_needed = total_chunk_len.div_ceil(SECTOR_SIZE);

        if sectors_needed >= 256 {
            return Err(Error::Malformed(
                "chunk data too large for region sector allocation",
            ));
        }

        let index = ((local_x & 31) + (local_z & 31) * 32) as usize;
        let old_entry = self.offsets[index];
        let old_offset = (old_entry >> 8) as usize;
        let old_count = (old_entry & 0xFF) as usize;

        let target_sector = if old_offset != 0 && old_count == sectors_needed {
            // Rewrite in place
            old_offset
        } else {
            // Free old sectors
            if old_offset != 0 {
                for s in old_offset..old_offset + old_count {
                    if s < self.sector_free.len() {
                        self.sector_free[s] = true;
                    }
                }
            }
            // Find contiguous run of sectors_needed
            let mut found_run = None;
            let mut run_len = 0;
            let mut run_start = 0;

            for (i, &free) in self.sector_free.iter().enumerate().skip(2) {
                if free {
                    if run_len == 0 {
                        run_start = i;
                    }
                    run_len += 1;
                    if run_len == sectors_needed {
                        found_run = Some(run_start);
                        break;
                    }
                } else {
                    run_len = 0;
                }
            }

            match found_run {
                Some(start) => {
                    for s in start..start + sectors_needed {
                        self.sector_free[s] = false;
                    }
                    start
                }
                None => {
                    // Grow file
                    let start = self.sector_free.len();
                    for _ in 0..sectors_needed {
                        self.sector_free.push(false);
                    }
                    start
                }
            }
        };

        // Write chunk data at target_sector
        self.file
            .seek(SeekFrom::Start((target_sector * SECTOR_SIZE) as u64))?;
        // Length field is payload_len + 1 (the version byte)
        let length_field = (payload_len + 1) as u32;
        self.file.write_all(&length_field.to_be_bytes())?;
        self.file.write_all(&[2u8])?; // Version 2 = ZLIB
        self.file.write_all(&compressed)?;

        // Sector padding
        let written = 4 + 1 + payload_len;
        let padding_needed = (sectors_needed * SECTOR_SIZE) - written;
        if padding_needed > 0 {
            let pad = vec![0u8; padding_needed];
            self.file.write_all(&pad)?;
        }

        // Update offset and timestamp
        let new_entry = ((target_sector as u32) << 8) | (sectors_needed as u32);
        self.offsets[index] = new_entry;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.timestamps[index] = now;

        // Write entry to offset table
        self.file.seek(SeekFrom::Start((index * 4) as u64))?;
        self.file.write_all(&new_entry.to_be_bytes())?;

        // Write entry to timestamp table
        self.file.seek(SeekFrom::Start((4096 + index * 4) as u64))?;
        self.file.write_all(&now.to_be_bytes())?;

        self.file.flush()?;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_file_round_trip() {
        let temp_dir = std::env::temp_dir().join(format!("flintcore_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let region_path = temp_dir.join("r.0.0.mcr");

        {
            let mut region = RegionFile::open(&region_path).unwrap();
            assert!(!region.has_chunk(0, 0));

            let mut chunk_nbt = NbtTag::compound();
            let mut level = NbtTag::compound();
            level.insert("xPos", NbtTag::Int(0));
            level.insert("zPos", NbtTag::Int(0));
            level.insert("Blocks", NbtTag::ByteArray(vec![4; 32768]));
            chunk_nbt.insert("Level", level);

            region.write_chunk(0, 0, &chunk_nbt).unwrap();
            assert!(region.has_chunk(0, 0));

            let loaded = region.read_chunk(0, 0).unwrap().unwrap();
            let level_loaded = loaded.get_compound("Level").unwrap();
            assert_eq!(level_loaded.get("xPos"), Some(&NbtTag::Int(0)));
            assert_eq!(level_loaded.get("zPos"), Some(&NbtTag::Int(0)));
        }

        // Re-open from disk
        {
            let mut region = RegionFile::open(&region_path).unwrap();
            assert!(region.has_chunk(0, 0));
            let loaded = region.read_chunk(0, 0).unwrap().unwrap();
            let level_loaded = loaded.get_compound("Level").unwrap();
            assert_eq!(level_loaded.get("xPos"), Some(&NbtTag::Int(0)));
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
