use std::sync::Arc;

use futures::{future, StreamExt};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_database::volatile::VolatileDatabase;
use nimiq_network_libp2p::Network;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::validator::build_validators;

#[test(tokio::test(flavor = "multi_thread"))]
#[ignore]
async fn four_validators_can_create_an_epoch() {
    let env = VolatileDatabase::new(20).expect("Could not open a volatile database");

    let validators =
        build_validators::<Network>(env, &(1u64..=4u64).collect::<Vec<_>>(), &mut None, false)
            .await;

    let blockchain = Arc::clone(&validators.first().unwrap().blockchain);

    tokio::spawn(future::join_all(validators));

    let events = blockchain.read().notifier_as_stream();

    events.take(130).for_each(|_| future::ready(())).await;

    assert!(blockchain.read().block_number() >= 130 + Policy::genesis_block_number());
}
