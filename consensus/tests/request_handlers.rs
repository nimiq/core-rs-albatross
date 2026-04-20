use std::sync::Arc;

use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_consensus::messages::{
    RequestTransactionReceiptsByAddress, RequestTransactionsProof, RequestTrieProof,
    ResponseTransactionProofError, ResponseTrieProofError,
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_network_interface::request::Handle;
use nimiq_network_mock::{MockNetwork, MockPeerId};
use nimiq_primitives::{key_nibbles::KeyNibbles, networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

fn blockchain_without_history_index() -> Arc<RwLock<Blockchain>> {
    Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig {
                index_history: false,
                ..Default::default()
            },
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ))
}

fn blockchain_with_accounts() -> Arc<RwLock<Blockchain>> {
    Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ))
}

#[test]
fn transactions_proof_request_without_index_returns_error_for_block_lookup_path() {
    let blockchain = blockchain_without_history_index();

    let response =
        <RequestTransactionsProof as Handle<MockNetwork, Arc<RwLock<Blockchain>>>>::handle(
            &RequestTransactionsProof {
                hashes: vec![Blake2bHash::default()],
                block_number: Some(Policy::genesis_block_number()),
            },
            MockPeerId(0),
            &blockchain,
        );

    assert!(matches!(
        response,
        Err(ResponseTransactionProofError::Other)
    ));
}

#[test]
fn transactions_proof_request_without_index_returns_error_for_hash_lookup_path() {
    let blockchain = blockchain_without_history_index();

    let response =
        <RequestTransactionsProof as Handle<MockNetwork, Arc<RwLock<Blockchain>>>>::handle(
            &RequestTransactionsProof {
                hashes: vec![Blake2bHash::default()],
                block_number: None,
            },
            MockPeerId(0),
            &blockchain,
        );

    assert!(matches!(
        response,
        Err(ResponseTransactionProofError::Other)
    ));
}

#[test]
fn transaction_receipts_request_without_index_returns_empty_list() {
    let blockchain = blockchain_without_history_index();

    let response = <RequestTransactionReceiptsByAddress as Handle<
        MockNetwork,
        Arc<RwLock<Blockchain>>,
    >>::handle(
        &RequestTransactionReceiptsByAddress {
            address: Address::burn_address(),
            max: None,
            start_at: None,
        },
        MockPeerId(0),
        &blockchain,
    );

    assert!(response.receipts.is_empty());
}

#[test]
fn trie_proof_request_rejects_duplicate_missing_keys() {
    let blockchain = blockchain_with_accounts();
    let absent_key: KeyNibbles = "deadbeefdeadbeefdeadbeef01".parse().unwrap();

    let response = <RequestTrieProof as Handle<MockNetwork, Arc<RwLock<Blockchain>>>>::handle(
        &RequestTrieProof {
            keys: vec![absent_key.clone(), absent_key],
        },
        MockPeerId(0),
        &blockchain,
    );

    assert!(matches!(
        response,
        Err(ResponseTrieProofError::DuplicateKeys)
    ));
}
