//! A replayed oracle update is refused, on a real chain.
//!
//! An oracle update carries no nonce: the owner's signature covers the transaction, and the oracle
//! appends whatever an owner-signed update holds, the same root again included. What stops an
//! applied update from being written a second time is the chain refusing a transaction it has
//! already included within the validity window, so these tests pin that refusal for the oracle.

use nimiq_account::{Account, OracleContract};
use nimiq_block::BlockError;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError};
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::bridge::{blake2b, BridgeChain};
use nimiq_transaction::{
    account::{htlc_contract::AnyHash, oracle_contract::IncomingOracleTransactionData},
    Transaction,
};

const HASH_COUNT: u16 = 4;

fn oracle(chain: &BridgeChain) -> OracleContract {
    match chain
        .node
        .blockchain
        .read()
        .get_account_if_complete(&chain.oracle)
    {
        Some(Account::Oracle(oracle)) => oracle,
        other => panic!("expected an oracle contract, found {other:?}"),
    }
}

fn root(label: &str) -> AnyHash {
    blake2b(label.as_bytes())
}

/// The update as anyone who saw it can resubmit it: the exact bytes it was included with.
fn replay_of(update: &Transaction) -> Transaction {
    let bytes = update.serialize_to_vec();
    let replay = Transaction::deserialize_from_vec(&bytes).unwrap();
    assert_eq!(replay.serialize_to_vec(), bytes);
    replay
}

/// Asserts that `replay` is, on its own, still a valid transaction that the oracle's owner signed,
/// so that only the duplicate check stands between it and the oracle.
fn assert_valid_on_its_own(chain: &BridgeChain, replay: &Transaction) {
    let owner = oracle(chain).owner;
    let blockchain = chain.node.blockchain.read();
    assert_eq!(
        replay
            .clone()
            .verify(blockchain.network_id(), blockchain.protocol_version()),
        Ok(())
    );
    let IncomingOracleTransactionData::Update { proof, .. } =
        IncomingOracleTransactionData::parse(replay).unwrap()
    else {
        panic!("expected an update");
    };
    assert!(proof.is_signed_by(&owner));
}

#[test]
fn an_applied_update_resubmitted_byte_for_byte_is_refused() {
    let chain = BridgeChain::new(HASH_COUNT);
    let first = root("first");
    let update = chain.update(vec![first.clone()]);
    chain.push_block(vec![update.clone()]);

    let applied = oracle(&chain);
    assert_eq!(applied.latest_index, Some(0));
    assert_eq!(applied.hashes[0], first.zero_of_same_type().digest(&first));

    let replay = replay_of(&update);
    assert_valid_on_its_own(&chain, &replay);

    let head = chain.node.blockchain.read().head_hash();
    let block = chain.produce_block(vec![replay]);
    assert_eq!(chain.node.push(block), Err(PushError::DuplicateTransaction));

    assert_eq!(chain.node.blockchain.read().head_hash(), head);
    assert_eq!(
        oracle(&chain).serialize_to_vec(),
        applied.serialize_to_vec()
    );
}

#[test]
fn an_applied_update_stays_refused_after_later_updates() {
    let chain = BridgeChain::new(HASH_COUNT);
    let update = chain.update(vec![root("first")]);
    chain.push_block(vec![update.clone()]);

    // Later updates move the ring on, and an empty block puts more distance between the update
    // and its replay, still inside the update's validity window.
    chain.push_block(vec![chain.update(vec![root("second")])]);
    chain.push_block(vec![]);
    chain.push_block(vec![chain.update(vec![root("third")])]);
    assert!(
        chain.node.blockchain.read().block_number()
            < update.validity_start_height + Policy::transaction_validity_window_blocks()
    );

    let before = oracle(&chain);
    assert_eq!(before.latest_index, Some(2));

    let replay = replay_of(&update);
    assert_valid_on_its_own(&chain, &replay);

    let block = chain.produce_block(vec![replay]);
    assert_eq!(chain.node.push(block), Err(PushError::DuplicateTransaction));
    assert_eq!(oracle(&chain).serialize_to_vec(), before.serialize_to_vec());
}

#[test]
fn an_update_included_twice_in_one_block_is_refused() {
    let chain = BridgeChain::new(HASH_COUNT);
    let update = chain.update(vec![root("first")]);
    let before = oracle(&chain);

    let block = chain.produce_block(vec![update.clone(), replay_of(&update)]);
    assert_eq!(
        chain.node.push(block),
        Err(PushError::InvalidBlock(BlockError::DuplicateTransaction))
    );
    assert_eq!(oracle(&chain).serialize_to_vec(), before.serialize_to_vec());
}
