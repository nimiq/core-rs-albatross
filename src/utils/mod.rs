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
pub mod iterators;
pub mod mutable_once;

pub fn systemtime_to_timestamp(start : SystemTime) -> u64 {
    match start.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() * 1000 + duration.subsec_nanos() as u64 / 1_000_000,
        Err(_) => panic!("SystemTime before UNIX EPOCH!"),
    }
}

pub fn timestamp_to_systemtime(start: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(start)
}

// FIXME: This can be replaced with duration.as_millis() once it becomes part of stable:
// https://doc.rust-lang.org/std/time/struct.Duration.html#method.as_millis
pub fn duration_as_millis(duration: &Duration) -> u64 {
    duration.as_secs() * 1_000u64 + duration.subsec_millis() as u64
}
