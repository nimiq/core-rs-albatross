use std::{sync::Arc, task::Poll, time::Duration};

use futures::{future, Stream, StreamExt};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_interface::{network::Network as _, validator_record::ValidatorRecord};
use nimiq_network_libp2p::Network;
use nimiq_network_mock::{MockHub, MockNetwork, MockPeerId};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::validator::{
    build_validator, build_validators, build_validators_with_keys, pop_validator_for_slot,
    seeded_rng, validator_keys,
};
use nimiq_time::timeout;
use nimiq_utils::{
    spawn,
    tagged_signing::{TaggedKeyPair, TaggedSigned},
};
use nimiq_validator_network::record::RecordAnnouncement;
use parking_lot::RwLock;

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

#[test(tokio::test)]
async fn an_elected_validator_announces_its_record_at_epoch_start() {
    let mut hub = MockHub::default();
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

    // A peer advertising the validator service, listening for announcements.
    let observer = hub.new_network();
    let mut inbox = observer.receive_messages::<RecordAnnouncement<MockPeerId>>();

    let (validator, mut consensus) = build_validator::<MockNetwork>(
        0,
        Address::from(&validator_key),
        false,
        signing_key.clone(),
        voting_key,
        fee_key,
        genesis,
        &mut Some(hub),
    )
    .await;
    let validator_peer_id = consensus.network.get_local_peer_id();
    observer.dial_mock(&consensus.network);

    // The mock never reports the DHT as ready, so this cannot come from the DHT path.
    consensus.force_established();
    spawn(validator);

    let (announcement, from) = timeout(Duration::from_secs(10), inbox.next())
        .await
        .expect("no announcement")
        .unwrap();
    assert_eq!(from, validator_peer_id);
    let record = &announcement.record.record;
    assert_eq!(record.peer_id, validator_peer_id);
    assert_eq!(record.validator_address, Address::from(&validator_key));
    assert!(announcement.record.verify(&signing_key.public));
}

/// Waits for a skip block above `height`.
async fn wait_for_skip_block(
    blockchain: &RwLock<Blockchain>,
    events: &mut (impl Stream<Item = BlockchainEvent> + Unpin),
    height: u32,
) {
    loop {
        timeout(Duration::from_secs(120), events.next())
            .await
            .expect("no skip block");
        let head = blockchain.read().head().clone();
        if head.is_skip() && head.block_number() > height {
            return;
        }
    }
}

#[test(tokio::test)]
async fn a_validator_that_restarts_with_a_new_peer_id_is_heard_again_without_the_dht() {
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");
    let mut hub = Some(MockHub::default());
    let keys = validator_keys(4);
    let (mut validators, genesis) =
        build_validators_with_keys::<MockNetwork>(env, &[1, 2, 3, 4], &keys, &mut hub).await;

    let restarting = validators.pop().unwrap();
    let restarting_network = Arc::clone(&restarting.consensus.network);
    let others: Vec<Arc<MockNetwork>> = validators
        .iter()
        .map(|validator| Arc::clone(&validator.consensus.network))
        .collect();
    let blockchain = Arc::clone(&validators[0].blockchain);
    let mut events = blockchain.read().notifier_as_stream();

    // Validator 1 never produces, so each of its slots takes a skip block, which with four equal
    // validators needs the signatures of all three others.
    let _never_producing = validators.remove(1);
    for validator in validators {
        spawn(validator);
    }
    // Spawned directly so that it can be aborted, like a node that crashed.
    #[allow(clippy::disallowed_methods)]
    let old_instance = tokio::spawn(restarting);

    // Freeze time to trigger producer timeouts immediately.
    tokio::time::pause();
    wait_for_skip_block(&blockchain, &mut events, Policy::genesis_block_number()).await;

    // The validator comes back under a new peer ID. The mock holds no DHT records at all. It
    // syncs in real time: with time frozen, the chain would grow faster than it can catch up.
    tokio::time::resume();
    old_instance.abort();
    restarting_network.shutdown();
    let restart_height = blockchain.read().block_number();
    let restarted = &keys[3];
    let (validator, consensus) = build_validator::<MockNetwork>(
        5,
        restarted.address(),
        false,
        restarted.signing_key.clone(),
        restarted.voting_key.clone(),
        restarted.fee_key.clone(),
        genesis,
        &mut hub,
    )
    .await;
    for other in &others {
        consensus.network.dial_mock(other);
    }
    let mut consensus_events = consensus.subscribe_events();
    spawn(consensus);
    timeout(Duration::from_secs(60), consensus_events.next())
        .await
        .expect("the restarted validator did not sync");
    spawn(validator);

    // The next skip block needs the restarted validator's signature.
    tokio::time::pause();
    wait_for_skip_block(&blockchain, &mut events, restart_height + 1).await;
}

#[test(tokio::test)]
async fn a_validator_re_signs_its_record_past_a_newer_copy() {
    let mut hub = MockHub::default();
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

    let observer = hub.new_network();
    let mut inbox = observer.receive_messages::<RecordAnnouncement<MockPeerId>>();

    let (validator, mut consensus) = build_validator::<MockNetwork>(
        0,
        Address::from(&validator_key),
        false,
        signing_key.clone(),
        voting_key,
        fee_key,
        genesis,
        &mut Some(hub),
    )
    .await;
    let validator_peer_id = consensus.network.get_local_peer_id();
    observer.dial_mock(&consensus.network);
    consensus.force_established();
    spawn(validator);

    let (first, _) = timeout(Duration::from_secs(10), inbox.next())
        .await
        .expect("no announcement")
        .unwrap();

    // A copy of the validator's record from a previous incarnation whose clock was ahead, which
    // the observer presents as its own. Only the validator holds the key to sign past it.
    let ahead = first.record.record.timestamp + 1000;
    let copy = ValidatorRecord::new(
        observer.get_local_peer_id(),
        Address::from(&validator_key),
        ahead,
    );
    let signature = signing_key.tagged_sign(&copy);
    observer
        .message(
            RecordAnnouncement {
                record: TaggedSigned::new(copy, signature),
            },
            validator_peer_id,
        )
        .await
        .unwrap();

    let again = loop {
        let (announcement, _) = timeout(Duration::from_secs(10), inbox.next())
            .await
            .expect("no re-announcement")
            .unwrap();
        if announcement.record.record.timestamp > ahead {
            break announcement;
        }
    };
    assert_eq!(again.record.record.peer_id, validator_peer_id);
    assert!(again.record.verify(&signing_key.public));
}
