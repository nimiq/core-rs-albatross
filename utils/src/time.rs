use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;

pub fn systemtime_to_timestamp(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() * 1000 + u64::from(duration.subsec_nanos()) / 1_000_000,
        Err(e) => panic!("SystemTime before UNIX EPOCH! Difference: {:?}", e.duration()),
    }
}

pub fn timestamp_to_systemtime(timestamp: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(timestamp)
}
