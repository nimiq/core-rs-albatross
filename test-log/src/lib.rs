use log::level_filters::LevelFilter;
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_log::TargetsExt;
use nimiq_primitives::policy::{Policy, TEST_POLICY};
pub use nimiq_test_log_proc_macro::test;
use parking_lot::Once;
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

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

        let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
        let genesis_block = network_info.genesis_block();

        // Run tests with different policy values:
        let mut policy_config = TEST_POLICY;
        // The genesis block number must be set accordingly
        policy_config.genesis_block_number = genesis_block.block_number();

        let _ = Policy::get_or_init(policy_config);
    });
}
