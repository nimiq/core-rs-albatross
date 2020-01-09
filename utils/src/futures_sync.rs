use std::sync::Arc;

use futures::FutureExt;
use parking_lot::Mutex;
use tokio::sync::watch::{channel, Sender};
use futures::future::BoxFuture;

pub struct WaitGroup {
    state: Arc<Mutex<WaitGroupInner>>,
}

struct WaitGroupInner {
    count: usize,
    tx: Option<Sender<usize>>,
}

impl WaitGroup {
    pub fn new() -> WaitGroup {
        WaitGroup {
            state: Arc::new(Mutex::new(WaitGroupInner {
                count: 1,
                tx: None,
            }))
        }
    }

    pub fn wait(self) -> BoxFuture<'static, ()> {
        let mut state = self.state.lock();
        let (tx, rx) = channel(state.count);
        state.tx = Some(tx);
        drop(state);
        async move {
            while let Some(count) = rx.clone().recv().await {
                if count <= 1 {
                    return;
                }
            }
        }.boxed()
    }
}

impl Clone for WaitGroup {
    fn clone(&self) -> Self {
        self.state.lock().count += 1;
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl Drop for WaitGroup {
    fn drop(&mut self) {
        let mut state = self.state.lock();
        state.count -= 1;
        if let Some(tx) = &state.tx {
            if tx.broadcast(state.count).is_err() {
                // The receiver future was dropped
                state.tx = None;
            }
        }
        drop(state);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use super::WaitGroup;

    #[tokio::test]
    async fn waitgroup_can_wait_on_nothing() {
        let wg = WaitGroup::new();
        wg.wait().await;
    }

    #[tokio::test]
    async fn waitgroup_wait_blocks() {
        let lock = Arc::new(Mutex::new(0));
        let lock2 = Arc::clone(&lock);
        let wg = WaitGroup::new();
        let wg2 = wg.clone();
        tokio::spawn(async move {
            wg.wait().await;
            assert_eq!(*lock2.lock().unwrap(), 1);
        });
        drop(wg2);
        *lock.lock().unwrap() = 1;
    }
}
