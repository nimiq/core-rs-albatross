// TODO: Documentation and move to nimiq_utils

use std::{pin::Pin, sync::Arc};

use futures::{
    task::{Context, Poll, Waker},
    Future,
};
use parking_lot::Mutex;

struct Inner {
    dropped: bool,
    wakers: Vec<Waker>,
}

impl Inner {
    fn dropped(&mut self) {
        self.dropped = true;
        for waker in &self.wakers {
            waker.wake_by_ref();
        }
    }
}

pub struct DropNotifier {
    inner: Arc<Mutex<Inner>>,
}

impl Default for DropNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl DropNotifier {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                dropped: false,
                wakers: vec![],
            })),
        }
    }

    pub fn listen(&self) -> DropListener {
        DropListener {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl std::ops::Drop for DropNotifier {
    fn drop(&mut self) {
        self.inner.lock().dropped();
    }
}

#[derive(Clone)]
pub struct DropListener {
    inner: Arc<Mutex<Inner>>,
}

impl Future for DropListener {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut inner = self.inner.lock();

        if inner.dropped {
            Poll::Ready(())
        } else {
            inner.wakers.push(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use tokio::time::{delay_for, timeout};

    #[tokio::test]
    async fn it_notifies_when_dropped() {
        let drop_notifier = DropNotifier::new();
        let drop_listener = drop_notifier.listen();

        // wait in background and drop value after some time.
        tokio::spawn(async move {
            delay_for(Duration::from_secs(1)).await;

            drop(drop_notifier);
        });

        // wait for future that waits for the notifier to be dropped.
        timeout(Duration::from_secs(2), drop_listener).await.unwrap();
    }
}
