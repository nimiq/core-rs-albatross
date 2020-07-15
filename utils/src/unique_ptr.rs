use std::cmp::Ordering;
use std::ops::Deref;

#[derive(Debug)]
pub struct UniquePtr<T>(*const T);

impl<T> UniquePtr<T> {
    pub fn new(v: &T) -> Self {
        UniquePtr(v as *const T)
    }
}

impl<T> Deref for UniquePtr<T> {
    type Target = T;

    fn deref(&self) -> &<Self as Deref>::Target {
        unsafe { &*self.0 }
    }
}

impl<T: PartialEq> PartialEq for UniquePtr<T> {
    fn eq(&self, other: &Self) -> bool {
        Deref::deref(self).eq(other)
    }
}

impl<T: PartialOrd> PartialOrd for UniquePtr<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Deref::deref(self).partial_cmp(other)
    }
}

impl<T: Ord> Ord for UniquePtr<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        Deref::deref(self).cmp(other)
    }
}

impl<T: Eq> Eq for UniquePtr<T> {}

impl<T> AsRef<T> for UniquePtr<T> {
    fn as_ref(&self) -> &T {
        self.deref()
    }
}
