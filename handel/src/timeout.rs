/// A timeout strategy defines after which period a level timeouts
///
/// TODO: This is not used currently


use std::time::Duration;
use std::ops::Range;

use futures::{Stream, StreamExt, stream};
use tokio::time::throttle;

pub trait TimeoutStrategy {
    type Timeouts: Stream<Item=usize>;

    fn timeouts(&self, num_levels: usize) -> Self::Timeouts;
}


#[derive(Clone, Debug)]
pub struct LinearTimeout {
    period: Duration,
}

impl LinearTimeout {
    pub fn new(period: Duration) -> Self {
        LinearTimeout {
            period
        }
    }
}

impl Default for LinearTimeout {
    fn default() -> Self {
        Self::new(Duration::from_millis(500))
    }
}

impl TimeoutStrategy for LinearTimeout {
    type Timeouts = Box<dyn Stream<Item=usize> + Send + Unpin>;

    fn timeouts(&self, num_levels: usize) -> Self::Timeouts {
        debug!("Creating timeout stream: period={:?}, levels={}", self.period, num_levels);
        let stream = stream::iter::<Range<usize>>(0..num_levels)
            .inspect(|level| debug!("Timeout for level {}", level));
        Box::new(throttle(self.period.clone(), stream))
    }
}
