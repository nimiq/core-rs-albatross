use parking_lot::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Arc;
use std::time::Duration;

pub mod crc;
pub mod merkle;
pub mod db;
pub mod locking;
pub mod mnemonic;
pub mod bit_vec;
pub mod key_derivation;
pub mod services;
pub mod observer;

pub fn systemtime_to_timestamp(start : SystemTime) -> u64 {
    let since_the_epoch = start.duration_since(UNIX_EPOCH);
    if let Ok(duration) = since_the_epoch {
        return duration.as_secs() * 1000 + duration.subsec_nanos() as u64 / 1_000_000;
    }
    return 0;
}

pub fn timestamp_to_systemtime(start: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(start)
}
