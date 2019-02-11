use std::cell::UnsafeCell;
use std::mem;

use database::Environment;
use database::lmdb::{LmdbEnvironment, open};


/// A wrapper for static variables that can be initialized at run-time
/// Invariants are checked dynamically, i.e. it will panic if you try to get a static reference
/// if the variable wasn't initialized yet or if you try to initialize it twice.
pub struct InitializedStatic<T> {
    inner: UnsafeCell<Option<T>>
}

impl<T> InitializedStatic<T> {
    pub fn new() -> InitializedStatic<T> {
        InitializedStatic{ inner: UnsafeCell::new(None) }
    }

    /// Initialize the static variable
    ///
    /// TODO: We either need to synchronize this, or mark it as unsafe.
    pub fn initialize(&self, x: T) {
        let inner = unsafe { self.inner.get().as_ref() }.unwrap();
        if inner.is_some() {
            panic!("InitializedStatic was already initialized");
        }
        unsafe { mem::replace(&mut *self.inner.get(), Some(x) ) };
    }

    pub fn get(&self) -> &T {
        let inner = unsafe { self.inner.get().as_ref() }.unwrap().as_ref();
        inner.expect("Static wasn't initialized yet")
    }
}

// This is needed for lazy_static
//
// Not sure if this is safe. We should probably synchronize initialize or set it to unsafe.
unsafe impl<T> std::marker::Sync for InitializedStatic<T> {}


pub type StaticEnvironment = InitializedStatic<Environment>;
lazy_static! {
    pub static ref ENV: InitializedStatic<Environment> = InitializedStatic::new();
}
