use parking_lot::Once;

pub use nimiq_test_log_proc_macro::test;

static INITIALIZE: Once = Once::new();

#[doc(hidden)]
pub fn initialize() {
    INITIALIZE.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .init();
    });
}
