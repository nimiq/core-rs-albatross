use std::time::SystemTime;
use std::time::Duration;
use crate::utils::systemtime_to_timestamp;
use atomic::Atomic;
use atomic::Ordering;

#[derive(Debug)]
pub struct NetworkTime {
    offset: Atomic<i64>
}

impl NetworkTime {
    pub fn new() -> Self {
        NetworkTime::with_offset(0)
    }

    pub fn with_offset(offset: i64) -> Self {
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
