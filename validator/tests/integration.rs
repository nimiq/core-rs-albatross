use std::sync::Arc;

use futures::{future, StreamExt};
use nimiq_blockchain::AbstractBlockchain;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_network_libp2p::Network;
use nimiq_test_utils::validator::build_validators;
use tracing_core::LevelFilter;
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn four_validators_can_create_an_epoch() {
    tracing_subscriber::registry()
        .with(
            Targets::new()
                .with_default(LevelFilter::INFO)
                .with_target("nimiq_validator", LevelFilter::DEBUG)
                .with_target("nimiq_network_libp2p", LevelFilter::INFO)
                .with_target("nimiq_handel", LevelFilter::INFO)
                .with_target("nimiq_tendermint", LevelFilter::DEBUG)
                .with_target("nimiq_blockchain", LevelFilter::DEBUG)
                .with_target("nimiq_block", LevelFilter::DEBUG),
        )
        .with(tracing_subscriber::fmt::layer().with_test_writer())
        .init();

    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    let validators = build_validators::<Network>(env, 4, &mut None).await;

    let blockchain = Arc::clone(&validators.first().unwrap().consensus.blockchain);

    tokio::spawn(future::join_all(validators));

    let events = blockchain.write().notifier.as_stream();

    events.take(130).for_each(|_| future::ready(())).await;

    assert!(blockchain.read().block_number() >= 130);
    assert_eq!(blockchain.read().view_number(), 0);
}
