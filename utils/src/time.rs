use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use atomic::{Atomic, Ordering};

/// Time with fixed offset from wall-clock, in milliseconds
#[derive(Debug, Default)]
pub struct OffsetTime {
    offset: Atomic<i64>,
}

impl OffsetTime {
    pub fn new() -> Self {
        OffsetTime::with_offset(0)
    }

    pub fn with_offset(offset: i64) -> Self {
        OffsetTime { offset: Atomic::new(offset) }
    }

    pub fn set_offset(&self, new_offset: i64) {
        self.offset.store(new_offset, Ordering::Relaxed);
    }

    pub fn now(&self) -> u64 {
        let offset = self.offset.load(Ordering::Relaxed);
        let abs_offset = offset.abs() as u64;
        let system_time = if offset > 0 {
            SystemTime::now() + Duration::from_millis(abs_offset)
        } else {
            SystemTime::now() - Duration::from_millis(abs_offset)
        };

        systemtime_to_timestamp(system_time)
    }
}

pub fn systemtime_to_timestamp(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() * 1000 + u64::from(duration.subsec_nanos()) / 1_000_000,
        Err(e) => panic!("SystemTime before UNIX EPOCH! Difference: {:?}", e.duration()),
    }
}

pub fn timestamp_to_systemtime(timestamp: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(timestamp)
}
