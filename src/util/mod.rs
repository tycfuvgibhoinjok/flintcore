pub mod bytes;
pub mod compress;
pub mod log;
pub mod math;
pub mod noise;
pub mod rng;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("incomplete packet data")]
    Incomplete,
    #[error("unknown packet id {0}")]
    UnknownPacketId(u8),
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
    #[error("invalid connection state: {0}")]
    InvalidState(&'static str),
    #[error("string length {len} exceeds maximum {max}")]
    StringTooLong { max: usize, len: usize },
    #[error("negative string length {0}")]
    NegativeStringLength(i16),
    #[error("config error: {0}")]
    Config(String),
    #[error("{0}")]
    Compression(&'static str),
    #[error("network error: {0}")]
    Network(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
