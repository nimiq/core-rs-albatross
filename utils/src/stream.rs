use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use futures::{stream as inner, Stream};
use pin_project::pin_project;

use crate::WakerExt as _;

/// An unbounded queue of futures.
///
/// This is a wrapper around [`futures::stream::FuturesOrdered`] that takes
/// care of waking when a future is pushed. See its documentation for more
/// details.
#[pin_project]
pub struct FuturesOrdered<F: Future> {
    #[pin]
    inner: inner::FuturesOrdered<F>,
    waker: Option<Waker>,
}

impl<F: Future> Default for FuturesOrdered<F> {
    fn default() -> FuturesOrdered<F> {
        FuturesOrdered {
            inner: Default::default(),
            waker: None,
        }
    }
}

impl<F: Future> FuturesOrdered<F> {
    /// Constructs an empty queue of futures.
    ///
    /// See also [`futures::stream::FuturesOrdered::new`].
    pub fn new() -> FuturesOrdered<F> {
        Default::default()
    }
    /// Returns `true` if the queue contains no futures.
    ///
    /// See also [`futures::stream::FuturesOrdered::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
    /// Returns the number of futures in the queue.
    ///
    /// See also [`futures::stream::FuturesOrdered::is_empty`].
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    /// Push a future into the back of the queue.
    ///
    /// See also [`futures::stream::FuturesOrdered::push`].
    pub fn push_back(&mut self, future: F) {
        self.inner.push_back(future);
        self.waker.wake();
    }
}

impl<F: Future> FromIterator<F> for FuturesOrdered<F> {
    fn from_iter<I: IntoIterator<Item = F>>(iter: I) -> FuturesOrdered<F> {
        FuturesOrdered {
            inner: inner::FuturesOrdered::from_iter(iter),
            waker: None,
        }
    }
}

impl<F: Future> Stream for FuturesOrdered<F> {
    type Item = F::Output;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<F::Output>> {
        let this = self.project();
        this.waker.store_waker(cx);
        this.inner.poll_next(cx)
    }
}

/// An unbounded set of futures which may complete in any order.
///
/// This is a wrapper around [`futures::stream::FuturesUnordered`] that takes
/// care of waking when a future is pushed. See its documentation for more
/// details.
#[pin_project]
pub struct FuturesUnordered<F: Future> {
    #[pin]
    inner: inner::FuturesUnordered<F>,
    waker: Option<Waker>,
}

impl<F: Future> Default for FuturesUnordered<F> {
    fn default() -> FuturesUnordered<F> {
        FuturesUnordered {
            inner: Default::default(),
            waker: None,
        }
    }
}

impl<F: Future> FuturesUnordered<F> {
    /// Constructs an empty set of futures.
    ///
    /// See also [`futures::stream::FuturesUnordered::new`].
    pub fn new() -> FuturesUnordered<F> {
        Default::default()
    }
    /// Returns `true` if the set contains no futures.
    ///
    /// See also [`futures::stream::FuturesUnordered::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
    /// Returns the number of futures in the set.
    ///
    /// See also [`futures::stream::FuturesUnordered::is_empty`].
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    /// Push a future into the set.
    ///
    /// See also [`futures::stream::FuturesUnordered::push`].
    pub fn push(&mut self, future: F) {
        self.inner.push(future);
        self.waker.wake();
    }
}

impl<F: Future> FromIterator<F> for FuturesUnordered<F> {
    fn from_iter<I: IntoIterator<Item = F>>(iter: I) -> FuturesUnordered<F> {
        FuturesUnordered {
            inner: inner::FuturesUnordered::from_iter(iter),
            waker: None,
        }
    }
}

impl<F: Future> Stream for FuturesUnordered<F> {
    type Item = F::Output;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<F::Output>> {
        let this = self.project();
        this.waker.store_waker(cx);
        this.inner.poll_next(cx)
    }
}
