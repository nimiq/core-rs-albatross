pub use nimiq_test_log_proc_macro::test;

#[doc(hidden)]
pub fn initialize() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
}
