use log::level_filters::LevelFilter;
use nimiq_log::TargetsExt;
use parking_lot::Once;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub use nimiq_test_log_proc_macro::test;

static INITIALIZE: Once = Once::new();

#[doc(hidden)]
pub fn initialize() {
    INITIALIZE.call_once(|| {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_test_writer())
            .with(
                Targets::new()
                    .with_default(LevelFilter::INFO)
                    .with_nimiq_targets(LevelFilter::DEBUG)
                    .with_env(),
            )
            .init();
    });
}
