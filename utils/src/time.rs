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

// FIXME: This can be replaced with duration.as_millis() once it becomes part of stable:
// https://doc.rust-lang.org/std/time/struct.Duration.html#method.as_millis
pub fn duration_as_millis(duration: &Duration) -> u64 {
    duration.as_secs() * 1_000u64 + u64::from(duration.subsec_millis())
}
