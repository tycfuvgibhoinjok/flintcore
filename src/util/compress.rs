use std::cell::RefCell;

use libdeflater::{CompressionLvl, Compressor, Decompressor};

use crate::util::{Error, Result};

pub const CHUNK_DEFLATE_LEVEL: u8 = 6;
pub const SMALL_DEFLATE_LEVEL: u8 = 1;

thread_local! {
    static ZLIB_WORKERS: RefCell<Option<(u8, Compressor, Vec<u8>)>> = const { RefCell::new(None) };
    static ZLIB_INFLATE_WORKERS: RefCell<(Decompressor, Vec<u8>)> = RefCell::new((
        Decompressor::new(),
        Vec::with_capacity(96 * 1024),
    ));
}

fn compression_level(level: u8) -> CompressionLvl {
    CompressionLvl::new(level as i32).unwrap_or_else(|_| CompressionLvl::default())
}

pub fn deflate_zlib(input: &[u8], level: u8) -> Result<Vec<u8>> {
    ZLIB_WORKERS.with(|cell| {
        let mut guard = cell.borrow_mut();
        if guard.as_ref().map(|(l, _, _)| *l) != Some(level) {
            *guard = Some((
                level,
                Compressor::new(compression_level(level)),
                Vec::with_capacity(82000),
            ));
        }
        let (_, compressor, scratch) = guard.as_mut().unwrap();
        let bound = compressor.zlib_compress_bound(input.len());
        if scratch.len() < bound {
            scratch.resize(bound, 0);
        }
        let written = compressor
            .zlib_compress(input, scratch)
            .map_err(|_| Error::Compression("zlib compression failed"))?;
        if scratch.capacity() > 131_072 && bound <= 90_000 {
            scratch.truncate(90_000);
            scratch.shrink_to_fit();
        }
        // Copy exact slice so capacity == written (~3 KB instead of 82 KB)
        Ok(scratch[..written].to_vec())
    })
}

pub fn inflate_zlib(input: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    ZLIB_INFLATE_WORKERS.with(|cell| {
        let mut guard = cell.borrow_mut();
        let (decompressor, scratch) = &mut *guard;
        let cap = expected_len.max(1);
        if scratch.len() < cap {
            scratch.resize(cap, 0);
        }
        let written = decompressor
            .zlib_decompress(input, &mut scratch[..cap])
            .map_err(|_| Error::Compression("zlib decompression failed"))?;
        let out = scratch[..written].to_vec();
        if scratch.capacity() > 256 * 1024 {
            scratch.truncate(96 * 1024);
            scratch.shrink_to_fit();
        }
        Ok(out)
    })
}

pub fn deflate_gzip(input: &[u8], level: u8) -> Result<Vec<u8>> {
    let mut compressor = Compressor::new(compression_level(level));
    let bound = compressor.gzip_compress_bound(input.len());
    let mut output = vec![0u8; bound];
    let written = compressor
        .gzip_compress(input, &mut output)
        .map_err(|_| Error::Compression("gzip compression failed"))?;
    Ok(output[..written].to_vec())
}

pub fn inflate_gzip(input: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    let mut output = vec![0u8; expected_len.max(1)];
    let written = Decompressor::new()
        .gzip_decompress(input, &mut output)
        .map_err(|_| Error::Compression("gzip decompression failed"))?;
    Ok(output[..written].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_compressible_payload() {
        let input: Vec<u8> = (0..81920).map(|index| (index / 512) as u8).collect();
        let compressed = deflate_zlib(&input, CHUNK_DEFLATE_LEVEL).unwrap();
        assert!(compressed.len() < input.len());
        assert_eq!(compressed.capacity(), compressed.len());
        assert_eq!(compressed[0], 0x78);
        let restored = inflate_zlib(&compressed, input.len()).unwrap();
        assert_eq!(restored, input);
    }

    #[test]
    fn rejects_truncated_stream() {
        let compressed = deflate_zlib(b"flintcore", SMALL_DEFLATE_LEVEL).unwrap();
        assert!(inflate_zlib(&compressed[..compressed.len() / 2], 64).is_err());
    }

    #[test]
    fn round_trips_gzip_payload() {
        let input = b"Hello, Beta 1.7.3 level.dat world!";
        let compressed = deflate_gzip(input, CHUNK_DEFLATE_LEVEL).unwrap();
        assert_eq!(compressed[0], 0x1f);
        assert_eq!(compressed[1], 0x8b);
        let restored = inflate_gzip(&compressed, input.len()).unwrap();
        assert_eq!(restored, input);
    }
}
