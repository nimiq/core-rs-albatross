use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::validator::{build_validator, seeded_rng};
use nimiq_time::timeout;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::spawn;

const NUM_SPAM_TXS: u64 = 1_600;
const NUM_SPAM_SENDERS: u64 = 4;
const NUM_PAYMENTS: u8 = 50;
const NUM_BLOCKS: usize = 3;

/// Free (value 0, fee 0) signaling staking transactions from never-funded throwaway keys are
/// admitted to the control pool. Control transactions are prioritized when a micro block is
/// produced, so without a cap on their share of the body they would crowd every fee-paying
/// regular transaction out of every block.
#[test(tokio::test(flavor = "multi_thread"))]
async fn control_spam_cannot_starve_regular_txs() {
    let network_id = NetworkId::UnitAlbatross;
    let hub = MockHub::default();
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    let voting_key = BlsKeyPair::generate(&mut seeded_rng(0));
    let validator_key = KeyPair::generate(&mut seeded_rng(0));
    let fee_key = KeyPair::generate(&mut seeded_rng(0));
    let signing_key = KeyPair::generate(&mut seeded_rng(0));
    let payer_key = KeyPair::generate(&mut seeded_rng(42));

    let genesis = GenesisBuilder::default()
        .with_network(network_id)
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
        .with_basic_account(
            Address::from(&payer_key),
            Coin::from_u64_unchecked(10_000_000),
        )
        .generate(env)
        .unwrap();

    let (validator, mut consensus) = build_validator::<Network>(
        0,
        Address::from(&validator_key),
        false,
        signing_key,
        voting_key,
        fee_key,
        genesis,
        &mut Some(hub),
    )
    .await;
    consensus.force_established();

    let mempool = Arc::clone(&validator.mempool_task.mempool);
    let blockchain = Arc::clone(&validator.blockchain);
    let validity_start_height = blockchain.read().block_number();

    // Flood the control pool with free signaling transactions. The senders are throwaway keys
    // that were never funded and hold no stake; rotating them sidesteps the per-sender limit.
    let spammers: Vec<KeyPair> = (0..NUM_SPAM_SENDERS)
        .map(|i| KeyPair::generate(&mut seeded_rng(100 + i)))
        .collect();
    for i in 0..NUM_SPAM_TXS {
        let tx = TransactionBuilder::new_set_active_stake(
            None,
            &spammers[(i % NUM_SPAM_SENDERS) as usize],
            Coin::from_u64_unchecked(i),
            Coin::ZERO,
            validity_start_height,
            network_id,
        )
        .unwrap();
        mempool
            .add_transaction(tx, None)
            .expect("free signaling transaction should be admitted");
    }

    // Queue fee-paying regular transactions from a funded account.
    for j in 0..NUM_PAYMENTS {
        let tx = TransactionBuilder::new_basic(
            &payer_key,
            Address::from([j; 20]),
            Coin::from_u64_unchecked(1),
            Coin::from_u64_unchecked(1_000),
            validity_start_height,
            network_id,
        )
        .unwrap();
        mempool.add_transaction(tx, None).unwrap();
    }

    let mut events = blockchain.read().notifier_as_stream();
    spawn(validator);

    let (mut regular, mut control) = (0usize, 0usize);
    for _ in 0..NUM_BLOCKS {
        let hash = loop {
            let event = timeout(Duration::from_secs(30), events.next())
                .await
                .expect("timed out waiting for a block")
                .expect("blockchain event stream ended");
            if let BlockchainEvent::Extended(hash) = event {
                break hash;
            }
        };
        let block = blockchain.read().get_block(&hash, true, None).unwrap();
        for tx in block.transactions().unwrap_or_default() {
            if tx.get_raw_transaction().recipient_type == AccountType::Staking {
                control += 1;
            } else {
                regular += 1;
            }
        }
    }

    assert!(
        regular >= NUM_PAYMENTS as usize,
        "regular txs starved: {regular} regular vs {control} control in {NUM_BLOCKS} micro blocks"
    );
    assert!(
        control > 0,
        "control transactions should still fill the remaining block space"
    );
}
