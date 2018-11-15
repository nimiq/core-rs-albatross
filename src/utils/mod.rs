use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub mod crc;
pub mod merkle;
pub mod db;
pub mod locking;
pub mod mnemonic;
pub mod bit_vec;
pub mod key_derivation;
pub mod services;


pub fn get_current_time_millis() -> u64 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH);
    if let Ok(duration) = since_the_epoch {
        return duration.as_secs() * 1000 + duration.subsec_nanos() as u64 / 1_000_000;
    } else {
        return 0;
    }
}
