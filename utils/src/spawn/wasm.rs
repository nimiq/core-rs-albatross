use std::future::Future;

use futures::FutureExt;

pub fn spawn<F>(future: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    // `spawn_local` only supports futures that return the unit type.
    wasm_bindgen_futures::spawn_local(future.map(|_| ()))
}
