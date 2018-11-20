use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;
use num_traits::real::Real;
use crate::utils::systemtime_to_timestamp;
use atomic::Atomic;
use atomic::Ordering;

#[derive(Debug)]
pub struct NetworkTime {
    offset: Atomic<i64>
}

impl NetworkTime {
    pub fn new(offset: i64) -> Self {
        NetworkTime {
            offset: Atomic::new(offset)
        }
    }

    pub fn set_offset(&self, new_offset: i64) {
        self.offset.store(new_offset, Ordering::Relaxed);
    }

    pub fn now(&self) -> u64 {
        let offset = self.offset.load(Ordering::Relaxed);
        let abs_offset = offset.abs() as u64;
        let system_time = if offset > 0 {
            SystemTime::now() + Duration::from_secs(abs_offset)
        } else {
            SystemTime::now() - Duration::from_secs(abs_offset)
        };

        return systemtime_to_timestamp(system_time);
    }
}
