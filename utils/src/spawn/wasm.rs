use std::future::Future;

pub fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    wasm_bindgen_futures::spawn_local(future);
}

pub fn spawn_local<F: Future<Output = ()> + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    wasm_bindgen_futures::spawn_local(future);
}

/// Runs `f` inline, as there are no threads to offload it to. This blocks the event loop.
pub async fn spawn_blocking<R: Send + 'static, F: FnOnce() -> R + Send + 'static>(f: F) -> R {
    f()
}
