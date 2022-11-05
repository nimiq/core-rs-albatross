use futures::{future, StreamExt};
use tokio::time;

use nimiq_block::{MultiSignature, SignedSkipBlockInfo, SkipBlockInfo};
use nimiq_blockchain::{AbstractBlockchain, BlockchainEvent};
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair};
use nimiq_collections::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_handel::update::{LevelUpdate, LevelUpdateMessage};
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_test_log::test;
use nimiq_test_utils::{
    test_network::TestNetwork,
    validator::{
        build_validator, build_validators, pop_validator_for_slot, seeded_rng, validator_for_slot,
    },
};
use nimiq_validator::aggregation::skip_block::SignedSkipBlockMessage;
use nimiq_vrf::VrfSeed;
use std::sync::Arc;
use std::time::Duration;

#[test(tokio::test)]
async fn one_validator_can_create_micro_blocks() {
    let hub = MockHub::default();
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    let voting_key = BlsKeyPair::generate(&mut seeded_rng(0));
    let validator_key = KeyPair::generate(&mut seeded_rng(0));
    let fee_key = KeyPair::generate(&mut seeded_rng(0));
    let signing_key = KeyPair::generate(&mut seeded_rng(0));
    let genesis = GenesisBuilder::default()
        .with_genesis_validator(
            Address::from(&validator_key),
            signing_key.public,
            voting_key.public_key,
            Address::default(),
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
        false,
    )
    .await;

    log::debug!("Establishing consensus...");
    consensus1.force_established();
    assert_eq!(consensus1.is_established(), true);

    log::debug!("Spawning validator...");
    tokio::spawn(validator);

    let events1 = consensus1.blockchain.write().notifier.as_stream();
    events1.take(10).for_each(|_| future::ready(())).await;

    assert!(consensus1.blockchain.read().block_number() >= 10);
}

#[test(tokio::test)]
async fn four_validators_can_create_micro_blocks() {
    let hub = MockHub::default();
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    let validators = build_validators::<Network>(
        env,
        &(1u64..=4u64).collect::<Vec<_>>(),
        &mut Some(hub),
        false,
    )
    .await;

    let blockchain = Arc::clone(&validators.first().unwrap().consensus.blockchain);

    tokio::spawn(future::join_all(validators));

    let events = blockchain.write().notifier.as_stream();
    time::timeout(
        Duration::from_secs(60),
        events.take(30).for_each(|_| future::ready(())),
    )
    .await
    .unwrap();

    assert!(blockchain.read().block_number() >= 30);
}

#[test(tokio::test)]
async fn four_validators_can_do_skip_block() {
    let hub = MockHub::default();
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    let mut validators = build_validators::<Network>(
        env,
        &(5u64..=8u64).collect::<Vec<_>>(),
        &mut Some(hub),
        false,
    )
    .await;

    // Disconnect the next block producer.
    let validator = pop_validator_for_slot(&mut validators, 1, 1);
    validator.consensus.network.disconnect().await;
    drop(validator);
    log::info!("Peer disconnection");

    // Listen for blockchain events from the new block producer (after a skip block).
    let validator = validators.first().unwrap();
    let blockchain = Arc::clone(&validator.consensus.blockchain);
    let mut events = blockchain.write().notifier.as_stream();

    // Freeze time to immediately trigger the block producer timeout.
    // time::pause();

    tokio::spawn(future::join_all(validators));

    // Wait for the new block producer to create a block.
    events.next().await;

    assert!(blockchain.read().block_number() >= 1);
}

fn create_skip_block_update(
    block_number: u32,
    prev_seed: VrfSeed,
    key_pair: BlsKeyPair,
    validator_id: u16,
    slots: &Vec<u16>,
) -> LevelUpdateMessage<SignedSkipBlockMessage, SkipBlockInfo> {
    // create skip block data according to parameters
    let skip_block_info = SkipBlockInfo {
        block_number,
        vrf_entropy: prev_seed.entropy(),
    };

    // get a single signature for this skip block data
    let signed_skip_block_info = SignedSkipBlockInfo::from_message(
        skip_block_info.clone(),
        &key_pair.secret_key,
        validator_id,
    );

    // multiply with number of slots to get a signature representing all the slots of this public_key
    let signature = AggregateSignature::from_signatures(&[signed_skip_block_info
        .signature
        .multiply(slots.len() as u16)]);

    // compute the signers bitset (which is just all the slots)
    let mut signers = BitSet::new();
    for slot in slots {
        signers.insert(*slot as usize);
    }

    // the contribution is composed of the signers bitset with the signature already multiplied by the number of slots.
    let contribution = SignedSkipBlockMessage {
        proof: MultiSignature::new(signature, signers),
    };

    LevelUpdate::new(
        contribution.clone(),
        Some(contribution),
        1,
        validator_id as usize,
    )
    .with_tag(skip_block_info)
}

#[ignore]
#[test(tokio::test)]
async fn validator_can_catch_up() {
    // remove first block producer in order to trigger a skip block. Never connect him again
    // remove the second block producer to trigger another skip block after the first one (which we want someone to catch up to). Never connect him again
    // third block producer needs to be disconnected as well and then reconnected to catch up to the second's skip blocks while not having seen the first one,
    // resulting in him producing the first block.
    let hub = MockHub::default();
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    // In total 8 validator are registered. after 3 validators are taken offline the remaining 5 should not be able to progress on their own
    let mut validators = build_validators::<Network>(
        env,
        &(9u64..=16u64).collect::<Vec<_>>(),
        &mut Some(hub),
        false,
    )
    .await;
    // Maintain a collection of the corresponding networks.

    let networks: Vec<Arc<Network>> = validators
        .iter()
        .map(|v| v.consensus.network.clone())
        .collect();

    // Disconnect the block producers for the next 3 skip blocks. remember the one which is supposed to actually create the block (3rd skip block)
    let (validator, _) = {
        let validator = validator_for_slot(&mut validators, 1, 1);
        validator.consensus.network.disconnect().await;
        let id1 = validator.validator_slot_band();
        let validator = validator_for_slot(&mut validators, 2, 2);
        validator.consensus.network.disconnect().await;
        let id2 = validator.validator_slot_band();
        assert_ne!(id2, id1);

        // ideally we would remove the validators from the vec for them to not even execute.
        // However the implementation does still progress their chains and since they have registered listeners, they would panic.
        // that is confusing, thus they are allowed to execute (with no validator network connection)
        // validators.retain(|v| {
        //     v.validator_address() != id1 && v.validator_address() != id2
        // });

        let validator = validator_for_slot(&validators, 3, 3);
        validator.consensus.network.disconnect().await;
        assert_ne!(id1, validator.validator_slot_band());
        assert_ne!(id2, validator.validator_slot_band());
        (validator, validator.consensus.network.clone())
    };
    // assert_eq!(validators.len(), 7);

    let blockchain = validator.consensus.blockchain.clone();
    // Listen for blockchain events from the block producer (after two skip blocks).
    let mut events = blockchain.write().notifier.as_stream();

    let (start, end) = blockchain.read().current_validators().unwrap().validators
        [validator.validator_slot_band() as usize]
        .slot_range;

    let slots = (start..end).collect();

    // Manually construct a skip block for the validator
    let vc = create_skip_block_update(
        1,
        blockchain.read().head().seed().clone(),
        validator.voting_key(),
        validator.validator_slot_band(),
        &slots,
    );

    // let the validators run.
    tokio::spawn(future::join_all(validators));

    // while waiting for them to run into the block producer timeout (10s)
    time::sleep(Duration::from_secs(11)).await;
    // At which point the prepared skip block message is broadcast
    // (only a subset of the validators will accept it as it send as level 1 message)
    for network in &networks {
        for peer_id in network.get_peers() {
            network
                .message::<LevelUpdateMessage<SignedSkipBlockMessage, SkipBlockInfo>>(
                    vc.clone(),
                    peer_id,
                )
                .await
                .unwrap();
        }
    }

    // wait enough time to complete the skip block aggregation (it really does not matter how long, as long as the vc completes)
    time::sleep(Duration::from_secs(8)).await;

    // reconnect a validator (who has not seen the proof for the skip block)
    log::warn!("connecting networks");
    Network::connect_networks(&networks, 9u64).await;

    // Wait for the new block producer to create a blockchainEvent (which is always an extended event for block 1) and keep the hash
    if let Some(BlockchainEvent::Extended(hash)) = events.next().await {
        // retrieve the block for height 1
        if let Some(block) = blockchain.read().get_block_at(1, false, None) {
            // the hash needs to be the one the extended event returned.
            // (the chain itself i.e blockchain.header_hash() might have already progressed further)
            assert_eq!(block.header().hash(), hash);
            // now in that case the validator producing this block has progressed the 2nd skip block without having seen the first skip block.
            return;
        }
    }

    assert!(false);
}
