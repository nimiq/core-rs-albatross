use std::future::Future;

pub fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    tokio::task::spawn(future);
}

pub fn spawn_local<F: Future<Output = ()> + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    tokio::task::spawn_local(future);
}
