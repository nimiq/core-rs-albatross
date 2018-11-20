use std::time::{SystemTime, UNIX_EPOCH};
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
pub mod timers;
pub mod version;
pub mod unique_ptr;

pub fn systemtime_to_timestamp(start : SystemTime) -> u64 {
    match start.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() * 1000 + duration.subsec_nanos() as u64 / 1_000_000,
        Err(_) => panic!("SystemTime before UNIX EPOCH!"),
    }
}

pub fn timestamp_to_systemtime(start: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(start)
}
