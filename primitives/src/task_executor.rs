use std::{future::Future, pin::Pin};

/// This a trait used to abstract the task executor that is used in several modules.
/// For instance, it can be used to execute tasks with tokio or wasm-bindgen if needed
/// This is the same approach that is used within some components of libp2p
pub trait TaskExecutor: Clone {
    /// Run the given future in the background until it ends.
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>);
}

impl<F: Fn(Pin<Box<dyn Future<Output = ()> + Send>>)> TaskExecutor for F
where
    F: Clone,
{
    fn exec(&self, f: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self(f)
    }
}
