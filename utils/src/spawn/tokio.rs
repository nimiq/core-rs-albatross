use std::future::Future;

pub fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    tokio::task::spawn(future);
}

pub fn spawn_local<F: Future<Output = ()> + 'static>(future: F) {
    #[allow(clippy::disallowed_methods)]
    tokio::task::spawn_local(future);
}

/// Runs `f` on a thread where blocking is acceptable and returns its result.
///
/// Unlike `tokio::task::spawn_blocking`, `f` only starts once the returned future is first polled.
/// Panics in `f` are propagated to the caller.
pub async fn spawn_blocking<R: Send + 'static, F: FnOnce() -> R + Send + 'static>(f: F) -> R {
    #[allow(clippy::disallowed_methods)]
    let result = tokio::task::spawn_blocking(f).await;
    match result {
        Ok(result) => result,
        Err(error) => match error.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(error) => panic!("blocking task failed: {error}"),
        },
    }
}
