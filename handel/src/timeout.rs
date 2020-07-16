use std::ops::Range;
/// A timeout strategy defines after which period a level timeouts
///
/// TODO: This is not used currently
use std::time::Duration;

use futures::{stream, Stream};
use tokio::util::StreamExt;

pub trait TimeoutStrategy {
    type Timeouts: Stream<Item = usize, Error = ()>;

    fn timeouts(&self, num_levels: usize) -> Self::Timeouts;
}

#[derive(Clone, Debug)]
pub struct LinearTimeout {
    period: Duration,
}

impl LinearTimeout {
    pub fn new(period: Duration) -> Self {
        LinearTimeout { period }
    }
}

impl Default for LinearTimeout {
    fn default() -> Self {
        Self::new(Duration::from_millis(500))
    }
}

impl TimeoutStrategy for LinearTimeout {
    type Timeouts = Box<dyn Stream<Item = usize, Error = ()> + Send>;

    fn timeouts(&self, num_levels: usize) -> Self::Timeouts {
        debug!(
            "Creating timeout stream: period={:?}, levels={}",
            self.period, num_levels
        );
        Box::new(
            stream::iter_ok::<Range<usize>, ()>(0..num_levels)
                .throttle(self.period.clone())
                .map(|level| {
                    debug!("Timeout for level {}", level);
                    level
                })
                .map_err(|e| {
                    warn!("Throttle error: {:?}", e);
                }),
        )
    }
}
