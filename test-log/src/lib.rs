use log::level_filters::LevelFilter;
use nimiq_log::TargetsExt;
use nimiq_primitives::policy::Policy;
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
                    .with_target("r1cs", LevelFilter::WARN)
                    .with_env(),
            )
            .init();
        // Run tests with different policy values:
        // Shorter epochs and shorter batches
        let policy = Policy {
            blocks_per_batch: 32,
            batches_per_epoch: 4,
            tendermint_timeout_init: 1000,
            tendermint_timeout_delta: 1000,
        };
        let _ = Policy::get_or_init(policy);
    });
}
