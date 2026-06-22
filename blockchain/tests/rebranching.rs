use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_genesis::NetworkId;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_serde::Deserialize;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{generate_transactions, validator_key, REWARD_KEY},
};
use nimiq_transaction::Transaction;
use nimiq_transaction_builder::TransactionBuilder;

fn key_pair_with_funds() -> KeyPair {
    let priv_key = PrivateKey::deserialize_from_vec(&hex::decode(REWARD_KEY).unwrap()).unwrap();
    priv_key.into()
}

fn update_validator_reward_address_tx(
    new_reward_address: Address,
    validity_start_height: u32,
) -> Transaction {
    TransactionBuilder::new_update_validator(
        &key_pair_with_funds(),
        &validator_key(),
        None,
        None,
        Some(new_reward_address),
        None,
        Coin::ZERO,
        validity_start_height,
        NetworkId::UnitAlbatross,
    )
}

/// Regression test: `create_reward_transactions` must read reward recipients from the in-flight
/// write txn, not a fresh committed snapshot. During a rebranch all fork blocks are applied on one
/// uncommitted txn, so an `UpdateValidator` that changed a validator's reward_address earlier in
/// the same batch is only visible there. Reading committed state recomputed the OLD address while
/// the macro body (built over committed state by the producer) carries the NEW one, so the
/// rebranching node rejected (`InvalidRewardTransactions` -> `InvalidFork`) a macro block that
/// extending nodes accept -- a deterministic liveness split between two honest nodes.
#[test]
fn it_can_rebranch_with_reward_address_change() {
    let producer1 = TemporaryBlockProducer::new(); // superior fork carrying the UpdateValidator
    let producer2 = TemporaryBlockProducer::new(); // victim/inferior fork that rebranches
    let new_reward_address = Address([0x11u8; 20]);

    // Shared committed prefix through the first macro block (rewards empty there).
    loop {
        let b = producer1.next_block(vec![], false);
        let is_macro = b.is_macro();
        assert_eq!(producer2.push(b), Ok(PushResult::Extended));
        if is_macro {
            break;
        }
    }
    let m1 = producer1.blockchain.read().block_number();
    assert_eq!(Policy::batch_at(m1), 1);
    let m2 = Policy::macro_block_after(m1 + 1);
    assert_eq!(Policy::batch_at(m2), 2);

    // Fork in batch 2: producer2 builds an inferior (skip-block) chain; producer1's second block
    // carries the UpdateValidator that sets the NEW reward address.
    let superior_first = producer1.next_block(vec![], false);
    producer2.next_block(vec![], true);
    assert_eq!(producer2.push(superior_first), Ok(PushResult::Ignored));

    let update_tx = update_validator_reward_address_tx(
        new_reward_address.clone(),
        producer1.blockchain.read().block_number() + 1,
    );
    let superior_update = producer1.next_block_with_txs(vec![], false, vec![update_tx]);
    producer2.next_block(vec![], false);
    assert_eq!(producer2.push(superior_update), Ok(PushResult::Ignored));

    while producer1.blockchain.read().block_number() < m2 - 1 {
        let sup = producer1.next_block(vec![], false);
        producer2.next_block(vec![], false);
        assert_eq!(producer2.push(sup), Ok(PushResult::Ignored));
    }

    let macro_block = producer1.next_block(vec![], false);
    assert!(macro_block.is_macro());
    assert_eq!(producer1.blockchain.read().block_number(), m2);

    // The rebranching victim must accept the same macro block extending nodes accept. Before the
    // fix this returned `Err(InvalidFork)` because rewards were recomputed from stale committed
    // state (OLD reward address).
    let result = producer2.push(macro_block);
    assert_eq!(
        result,
        Ok(PushResult::Rebranched),
        "rebranching victim must accept the macro block extending nodes accept; got {result:?}"
    );

    let b1 = producer1.blockchain.read();
    let b2 = producer2.blockchain.read();
    assert_eq!(b1.state.head_hash, b2.state.head_hash);
    assert_eq!(b1.state.macro_head_hash, b2.state.macro_head_hash);
}

#[test]
fn it_can_rebranch_skip_block() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 1: easy rebranch (number denotes accumulated skip blocks)
    // [0] - [0] - [0] - [0]
    //          \- [1] - [1]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(vec![], false);
    let fork1 = temp_producer2.next_block(vec![], true);

    let inferior2 = temp_producer1.next_block(vec![], false);
    let fork2 = temp_producer2.next_block(vec![], false);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Ok(PushResult::Ignored));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));

    // Case 2: not obvious rebranch rebranch (number denotes accumulated skip block)
    // ... - [1] - [1] - [2]
    //          \- [2] - [2]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let inferior1 = temp_producer1.next_block(vec![], false);
    let fork1 = temp_producer2.next_block(vec![], true);

    let inferior2 = temp_producer1.next_block(vec![], true);
    let fork2 = temp_producer2.next_block(vec![], false);

    // Check that producer 2 ignores inferior chain.
    assert_eq!(temp_producer2.push(inferior1), Ok(PushResult::Ignored));
    assert_eq!(temp_producer2.push(inferior2), Ok(PushResult::Ignored));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork1), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Extended));
}

#[test]
fn micro_block_works_after_macro_block() {
    let genesis_block_number = Policy::genesis_block_number();
    let temp_producer = TemporaryBlockProducer::new();

    // apply an entire batch including macro block on view_number/round_number zero
    for _ in 0..Policy::blocks_per_batch() {
        temp_producer.next_block(vec![], false);
    }
    // make sure we are at the beginning of the batch and all block were applied
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + genesis_block_number
    );

    // Test if a micro block can be rebranched immediately after
    // a round_number 0 macro block

    // create a couple of skip blocks
    let block = temp_producer.next_block_no_push(vec![], true);
    let rebranch = temp_producer.next_block_no_push(vec![], true);
    // push first skip block
    temp_producer.push(block).unwrap();
    // make sure this was an extend
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + 1 + genesis_block_number
    );
    // and rebranch it to block the chain with only one skip block
    temp_producer.push(rebranch).unwrap();
    // make sure this was a rebranch
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() + 1 + genesis_block_number
    );

    // apply the rest of the batch including macro block on view_number/round_number one
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    // make sure we are at the beginning of the batch
    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() * 2 + genesis_block_number
    );

    // Test if a micro block can be rebranched immediately after
    // a round_number non 0 macro block

    // create blocks for a chain with accumulated skip blocks after the batch of 0, 1 and 2
    let block = temp_producer.next_block_no_push(vec![], true);
    let rebranch1 = temp_producer.next_block_no_push(vec![], true);
    // let rebranch2 = temp_producer.next_block_no_push(2, vec![]);
    // apply them each rebranching the previous one
    temp_producer.push(block).unwrap();
    temp_producer.push(rebranch1).unwrap();
    // temp_producer.push(rebranch2).unwrap();

    assert_eq!(
        temp_producer.blockchain.read().block_number(),
        Policy::blocks_per_batch() * 2 + 1 + genesis_block_number
    );
}

#[test]
fn it_can_rebranch_forks() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // Case 2: more difficult rebranch
    //              a     b     c     d
    // [0] - [0] - [0] - [0] - [0] - [0]
    //          \- [0] - [0] - [1] - [1]
    let block = temp_producer1.next_block(vec![], false);
    temp_producer2.push(block).unwrap();

    let fork1a = temp_producer1.next_block(vec![0x48], false);
    let fork2a = temp_producer2.next_block(vec![], false);

    let fork1b = temp_producer1.next_block(vec![], false);
    let fork2b = temp_producer2.next_block(vec![], false);

    let fork1c = temp_producer1.next_block(vec![], false);
    let fork2c = temp_producer2.next_block(vec![], true);

    let fork1d = temp_producer1.next_block(vec![], false);
    let fork2d = temp_producer2.next_block(vec![], false);

    // Check that each one accepts other fork.
    assert_eq!(temp_producer1.push(fork2a), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1a), Ok(PushResult::Forked));
    assert_eq!(temp_producer1.push(fork2b), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(fork1b), Ok(PushResult::Forked));

    // Check that producer 1 rebranches.
    assert_eq!(temp_producer1.push(fork2c), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1c), Ok(PushResult::Ignored));

    assert_eq!(temp_producer1.push(fork2d), Ok(PushResult::Extended));
    assert_eq!(temp_producer2.push(fork1d), Ok(PushResult::Ignored));
}

#[test]
fn rebranched_blocks_preserve_history_chain_info() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();
    let funded_key_pair = key_pair_with_funds();

    let shared = temp_producer1.next_block(vec![], false);
    assert_eq!(temp_producer2.push(shared), Ok(PushResult::Extended));

    let fork1_txs = generate_transactions(
        &funded_key_pair,
        temp_producer2.blockchain.read().block_number() + 1,
        NetworkId::UnitAlbatross,
        1,
        1,
    );
    let fork1 = temp_producer2.next_block_with_txs(vec![0x48], false, fork1_txs);
    let inferior = temp_producer1.next_block(vec![], false);

    assert_eq!(temp_producer1.push(fork1.clone()), Ok(PushResult::Forked));
    assert_eq!(temp_producer2.push(inferior), Ok(PushResult::Forked));

    let fork2_txs = generate_transactions(
        &funded_key_pair,
        temp_producer2.blockchain.read().block_number() + 1,
        NetworkId::UnitAlbatross,
        1,
        2,
    );
    let fork2 = temp_producer2.next_block_with_txs(vec![], false, fork2_txs);

    assert_eq!(
        temp_producer1.push(fork2.clone()),
        Ok(PushResult::Rebranched)
    );

    let source_chain = temp_producer2.blockchain.read();
    let source_fork1 = source_chain
        .chain_store
        .get_chain_info(&fork1.hash(), false, None)
        .unwrap();
    let source_fork2 = source_chain
        .chain_store
        .get_chain_info(&fork2.hash(), false, None)
        .unwrap();

    assert!(source_fork1.history_tree_len > 0);
    assert!(source_fork1.cum_hist_tx_size > 0);
    assert!(source_fork2.history_tree_len > 0);
    assert!(source_fork2.cum_hist_tx_size > 0);

    let rebranched_chain = temp_producer1.blockchain.read();
    let adopted_fork1 = rebranched_chain
        .chain_store
        .get_chain_info(&fork1.hash(), false, None)
        .unwrap();
    let adopted_fork2 = rebranched_chain
        .chain_store
        .get_chain_info(&fork2.hash(), false, None)
        .unwrap();

    assert_eq!(
        adopted_fork1.history_tree_len,
        source_fork1.history_tree_len
    );
    assert_eq!(
        adopted_fork1.cum_hist_tx_size,
        source_fork1.cum_hist_tx_size
    );
    assert_eq!(
        adopted_fork2.history_tree_len,
        source_fork2.history_tree_len
    );
    assert_eq!(
        adopted_fork2.cum_hist_tx_size,
        source_fork2.cum_hist_tx_size
    );
    assert_eq!(
        rebranched_chain.state.main_chain.history_tree_len,
        source_fork2.history_tree_len
    );
    assert_eq!(
        rebranched_chain.state.main_chain.cum_hist_tx_size,
        source_fork2.cum_hist_tx_size
    );
}

#[test]
fn it_can_rebranch_at_macro_block() {
    // Build forks using two producers.
    let temp_producer1 = TemporaryBlockProducer::new();
    let temp_producer2 = TemporaryBlockProducer::new();

    // The numbers in [X/Y] represent block_number (X) and view_number (Y):
    //
    // [0/0] ... [1/0] - [1/0]
    //                \- [1/1]

    let mut block;
    loop {
        block = temp_producer1.next_block(vec![], false);
        temp_producer2.push(block.clone()).unwrap();
        if block.is_macro() {
            break;
        }
    }

    let fork1 = temp_producer1.next_block(vec![], false);
    let fork2 = temp_producer2.next_block(vec![], true);

    assert_eq!(temp_producer1.push(fork2), Ok(PushResult::Rebranched));
    assert_eq!(temp_producer2.push(fork1), Ok(PushResult::Ignored));
}

#[test]
fn it_can_rebranch_to_inferior_macro_block() {
    // Build forks using two producers.
    let producer1 = TemporaryBlockProducer::new();
    let producer2 = TemporaryBlockProducer::new();

    // (1 denotes a skip block)
    // [0] - [0] - ... - [0] - [macro 0]
    //    \- [1] - ... - [0]

    // Do one iteration first to create fork
    let inferior = producer1.next_block(vec![], false);
    producer2.next_block(vec![], true);
    assert_eq!(producer2.push(inferior), Ok(PushResult::Ignored));

    // Complete a batch
    for _ in 1..Policy::blocks_per_batch() - 1 {
        let inferior = producer1.next_block(vec![], false);
        producer2.next_block(vec![], false);
        assert_eq!(producer2.push(inferior), Ok(PushResult::Ignored));
    }

    let macro_block = producer1.next_block(vec![], false);
    assert!(macro_block.is_macro());

    // Check that producer 2 rebranches.
    assert_eq!(producer2.push(macro_block), Ok(PushResult::Rebranched));

    // Push one additional block and check that producer 2 accepts it.
    let block = producer1.next_block(vec![], false);
    assert_eq!(producer2.push(block), Ok(PushResult::Extended));

    // Check that both chains are in an identical state.
    let blockchain1 = producer1.blockchain.read();
    let blockchain2 = producer2.blockchain.read();
    assert_eq!(blockchain1.state.head_hash, blockchain2.state.head_hash);
    assert_eq!(
        blockchain1.state.macro_head_hash,
        blockchain2.state.macro_head_hash
    );
    assert_eq!(
        blockchain1.state.election_head_hash,
        blockchain2.state.election_head_hash
    );
    assert_eq!(
        blockchain1.state.current_slots,
        blockchain2.state.current_slots
    );
    assert_eq!(
        blockchain1.state.previous_slots,
        blockchain2.state.previous_slots
    );
}
