use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct NetworkTime;

impl NetworkTime {
    pub fn now(&self) -> u32 {
        // TODO actually compute network adjusted time.

        return match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs() as u32,
            Err(_) => panic!("SystemTime before UNIX EPOCH!"),
        };
    }
}
