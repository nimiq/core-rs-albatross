use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Hash)]
pub struct UniqueId(usize);

static GLOBAL_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl UniqueId {
    pub fn new() -> Self {
        UniqueId(GLOBAL_COUNTER.fetch_add(1, Ordering::Release))
    }
}

impl fmt::Display for UniqueId {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0)
    }
}