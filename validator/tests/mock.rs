use std::{sync::Arc, task::Poll, time::Duration};

use futures::{future, StreamExt};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_interface::network::Network as _;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::validator::{
    build_validator, build_validators, pop_validator_for_slot, seeded_rng,
};
use nimiq_time::timeout;
use nimiq_utils::spawn;

#[test(tokio::test)]
async fn one_validator_can_create_micro_blocks() {
    let hub = MockHub::default();
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    let voting_key = BlsKeyPair::generate(&mut seeded_rng(0));
    let validator_key = KeyPair::generate(&mut seeded_rng(0));
    let fee_key = KeyPair::generate(&mut seeded_rng(0));
    let signing_key = KeyPair::generate(&mut seeded_rng(0));
    let genesis = GenesisBuilder::default()
        .with_network(NetworkId::UnitAlbatross)
        .with_genesis_block_number(Policy::genesis_block_number())
        .with_genesis_validator(
            Address::from(&validator_key),
            signing_key.public,
            voting_key.public_key,
            Address::default(),
            None,
            None,
            false,
        )
        .generate(env)
        .unwrap();

    let (validator, mut consensus1) = build_validator::<Network>(
        0,
        Address::from(&validator_key),
        false,
        signing_key,
        voting_key,
        fee_key,
        genesis.clone(),
        &mut Some(hub),
    )
    .await;

    log::debug!("Establishing consensus...");
    consensus1.force_established();
    assert!(consensus1.is_established());

    let blockchain = Arc::clone(&validator.blockchain);

    log::debug!("Spawning validator...");
    spawn(validator);

    let events1 = blockchain.read().notifier_as_stream();
    events1.take(10).for_each(|_| future::ready(())).await;

    assert!(consensus1.blockchain.read().block_number() >= 10 + Policy::genesis_block_number());
}

#[test(tokio::test)]
async fn four_validators_can_create_micro_blocks() {
    let hub = MockHub::default();
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    let validators =
        build_validators::<MockNetwork>(env, &(1u64..=4u64).collect::<Vec<_>>(), &mut Some(hub))
            .await;

    let blockchain = Arc::clone(&validators.first().unwrap().blockchain);

    for validator in validators {
        spawn(validator);
    }

    // Take events until 30 blocks have been produced.
    let blockchain2 = Arc::clone(&blockchain);
    let stop_fut = future::poll_fn(move |_cx| {
        if blockchain2.read().block_number() < 30 + Policy::genesis_block_number() {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    });

    let events = blockchain.read().notifier_as_stream();
    timeout(
        Duration::from_secs(60),
        events.take_until(stop_fut).for_each(|e| {
            log::info!(?e, "EVENT");
            future::ready(())
        }),
    )
    .await
    .unwrap();

    assert!(blockchain.read().block_number() >= 30 + Policy::genesis_block_number());
}

#[test(tokio::test)]
async fn validator_advertises_its_claim_while_producing_blocks() {
    let mut hub = Some(MockHub::default());
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    let voting_key = BlsKeyPair::generate(&mut seeded_rng(0));
    let validator_key = KeyPair::generate(&mut seeded_rng(0));
    let fee_key = KeyPair::generate(&mut seeded_rng(0));
    let signing_key = KeyPair::generate(&mut seeded_rng(0));
    let validator_address = Address::from(&validator_key);
    let genesis = GenesisBuilder::default()
        .with_network(NetworkId::UnitAlbatross)
        .with_genesis_block_number(Policy::genesis_block_number())
        .with_genesis_validator(
            validator_address.clone(),
            signing_key.public,
            voting_key.public_key,
            Address::default(),
            None,
            None,
            false,
        )
        .generate(env)
        .unwrap();

    let (validator, mut consensus1) = build_validator::<MockNetwork>(
        1,
        validator_address.clone(),
        false,
        signing_key,
        voting_key,
        fee_key,
        genesis,
        &mut hub,
    )
    .await;
    let validator_peer_id = consensus1.network.peer_id();

    // Resolve from another network on the hub, since a network never resolves its own peer ID.
    let observer = hub.as_mut().unwrap().new_network_with_address(2);
    assert!(observer
        .get_validator_peer_ids(&validator_address)
        .is_empty());

    log::debug!("Establishing consensus...");
    consensus1.force_established();
    assert!(consensus1.is_established());

    let blockchain = Arc::clone(&validator.blockchain);

    log::debug!("Spawning validator...");
    spawn(validator);

    // The signer is installed on the first poll after syncing, so it is in place by the time the
    // validator has produced some blocks.
    let events1 = blockchain.read().notifier_as_stream();
    timeout(
        Duration::from_secs(60),
        events1.take(3).for_each(|_| future::ready(())),
    )
    .await
    .unwrap();

    assert_eq!(
        observer.get_validator_peer_ids(&validator_address),
        vec![validator_peer_id]
    );
}

#[test(tokio::test)]
async fn validators_can_do_skip_block() {
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    let mut validators =
        build_validators::<Network>(env, &(5u64..=10u64).collect::<Vec<_>>(), &mut None).await;

    // Disconnect the next block producer.
    let _validator = pop_validator_for_slot(
        &mut validators,
        1 + Policy::genesis_block_number(),
        1 + Policy::genesis_block_number(),
    );

    // Listen for blockchain events from the new block producer (after a skip block).
    let validator = validators.first().unwrap();
    let blockchain = Arc::clone(&validator.blockchain);
    let mut events = blockchain.read().notifier_as_stream();

    // Freeze time to immediately trigger the block producer timeout.
    tokio::time::pause();

    for validator in validators {
        spawn(validator);
    }

    // Wait for the new block producer to create a skip block.
    events.next().await;

    // Verify the skip block was produced:
    let block = blockchain.read().head().clone();

    assert!(block.is_skip());
    assert!(block.block_number() > Policy::genesis_block_number());
}
