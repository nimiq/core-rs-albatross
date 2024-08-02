use std::future::Future;

pub fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    wasm_bindgen_futures::spawn_local(future);
}

pub fn spawn_local<F: Future<Output = ()> + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    wasm_bindgen_futures::spawn_local(future);
}
