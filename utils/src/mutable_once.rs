use std::cell::UnsafeCell;
use std::mem;
use std::ops::Deref;

pub struct MutableOnce<T> {
    inner: UnsafeCell<T>,
}

impl<T> MutableOnce<T> {
    /// Initializes object with a default value.
    pub fn new(default: T) -> Self {
        MutableOnce {
            inner: UnsafeCell::new(default),
        }
    }

    /// This mutates the content of the cell
    /// and should only be called once, before any other reference can exist.
    /// This condition is not checked, thus this call is unsafe.
    pub unsafe fn replace(&self, value: T) {
        let _ = mem::replace(&mut *self.inner.get(), value);
    }

    fn get(&self) -> &T {
        unsafe { &*self.inner.get() }
    }
}

impl<T> Deref for MutableOnce<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T> AsRef<T> for MutableOnce<T> {
    fn as_ref(&self) -> &T {
        self.get()
    }
}

unsafe impl<T: Send> Send for MutableOnce<T> {}
unsafe impl<T: Send> Sync for MutableOnce<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutable_once_works() {
        let v = MutableOnce::new(1);
        assert_eq!(*v.as_ref(), 1);

        let v = MutableOnce::new(1);
        unsafe { v.replace(2) };
        assert_eq!(*v.as_ref(), 2);
    }
}
