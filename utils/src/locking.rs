use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use parking_lot::{Mutex, MutexGuard};

#[derive(Debug)]
pub struct MultiLock<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> MultiLock<T> {
    /// Creates a new `MultiLock` protecting the provided data.
    ///
    /// A cloneable handle to the lock is returned. Such handles can then be sent to separate
    /// tasks to be managed there.
    pub fn new(t: T) -> Self {
        let inner = Arc::new(Mutex::new(t));

        MultiLock { inner }
    }

    /// Attempt to acquire this lock, returning `NotReady` if it can't be
    /// acquired.
    ///
    /// This function will acquire the lock in a nonblocking fashion, returning
    /// immediately if the lock is already held. If the lock is successfully
    /// acquired then `Async::Ready` is returned with a value that represents
    /// the locked value (and can be used to access the protected data). The
    /// lock is unlocked when the returned `MutexGuard` is dropped.
    ///
    /// If the lock is already held then this function will return
    /// `Async::NotReady`. In this case the current task will also be scheduled
    /// to receive a notification when the lock would otherwise become
    /// available.
    pub fn poll_lock(&self) -> Poll<MutexGuard<T>> {
        match self.inner.try_lock() {
            None => Poll::Pending,
            Some(lock) => Poll::Ready(lock),
        }
    }

    /// Perform a "blocking lock" of this lock, consuming this lock handle and
    /// returning a future to the acquired lock.
    ///
    /// This function consumes the `MultiLock<T>` and returns a sentinel future,
    /// `MultiLockAcquire<T>`. The returned future will resolve to
    /// `MultiLockAcquired<T>` which represents a locked lock similarly to
    /// `MutexGuard<T>`.
    ///
    /// Note that the returned future will never resolve to an error.
    pub fn lock(self) -> MultiLockAcquire<T> {
        MultiLockAcquire {
            inner: Some(self),
        }
    }

    unsafe fn force_unlock(&self) {
        self.inner.force_unlock()
    }
}

impl<T> Clone for MultiLock<T> {
    #[inline]
    fn clone(&self) -> Self {
        MultiLock {
            inner: self.inner.clone()
        }
    }
}

/// Future returned by `BiLock::lock` which will resolve when the lock is
/// acquired.
#[derive(Debug)]
pub struct MultiLockAcquire<T> {
    inner: Option<MultiLock<T>>,
}

impl<T> Future for MultiLockAcquire<T> {
    type Output = MultiLockAcquired<T>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_ref().expect("cannot poll after Ready").poll_lock() {
            Poll::Ready(r) => { mem::forget(r); },
            Poll::Pending => return Poll::Pending,
        }
        Poll::Ready(MultiLockAcquired { inner: self.inner.take() })
    }
}

/// Resolved value of the `MultiLockAcquire<T>` future.
///
/// This value, like `MutexGuard<T>`, is a sentinel to the value `T` through
/// implementations of `Deref` and `DerefMut`. When dropped will unlock the
/// lock, and the original unlocked `MultiLock<T>` can be recovered through the
/// `unlock` method.
#[derive(Debug)]
pub struct MultiLockAcquired<T> {
    inner: Option<MultiLock<T>>,
}

impl<T> MultiLockAcquired<T> {
    /// Recovers the original `MultiLock<T>`, unlocking this lock.
    pub fn unlock(mut self) -> MultiLock<T> {
        let multi_lock = self.inner.take().unwrap();

        unsafe { multi_lock.force_unlock() };

        multi_lock
    }
}

//impl<T> Deref for MultiLockAcquired<T> {
//    type Target = T;
//    fn deref(&self) -> &T {
//        // TODO
//    }
//}
//
//impl<T> DerefMut for MultiLockAcquired<T> {
//    fn deref_mut(&mut self) -> &mut T {
//        // TODO
//    }
//}
