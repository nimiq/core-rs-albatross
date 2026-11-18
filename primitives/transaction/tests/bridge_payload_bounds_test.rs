//! Burn-payload size bounds on the consensus release path.
//!
//! The release path has no payload cap of its own: it deserializes `sender_data` with
//! `OutgoingBridgeTransactionData::parse` and verifies the signature. What bounds the payload is
//! `Policy::MAX_TX_SENDER_DATA_SIZE`, enforced while deserializing an extended transaction, so it
//! applies to every transaction that arrives over the wire or is read out of a block. These tests
//! pin that ceiling, so that changing the policy limit or the envelope is a visible decision.

mod amoy;

use amoy::{amoy_chain_config, AMOY_SOURCE_CONTRACT};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::bridge_contract::OutgoingBridgeTransactionData,
    bridge_contract::{AnyMerkleProof, BridgeError, OutgoingTransaction},
    SignatureProof, Transaction,
};
use nimiq_utils::merkle::MerklePath;

fn key_pair() -> KeyPair {
    KeyPair::from(
        PrivateKey::deserialize_from_vec(
            &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0")
                .unwrap(),
        )
        .unwrap(),
    )
}

fn burn_proof(payload_len: usize, proof: AnyMerkleProof) -> OutgoingTransaction {
    OutgoingTransaction {
        burn_transaction_data: vec![0xABu8; payload_len],
        merkle_proof: proof,
        oracle_state_index: 0,
    }
}

/// The length of an Amoy burn record, the bytes the Amoy program reads.
const AMOY_RECORD_LEN: usize = 156;

/// A well-formed Amoy burn record of 500 luna, padded with filler to `total_len`. The program
/// reads only the first 156 bytes, so the filler is what an attacker would inflate.
fn padded_burn_record(total_len: usize) -> Vec<u8> {
    fn word(value: u128) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[16..].copy_from_slice(&value.to_be_bytes());
        word
    }
    let mut payload = Vec::with_capacity(total_len);
    payload.extend_from_slice(&AMOY_SOURCE_CONTRACT); // source_contract
    payload.extend_from_slice(&[0xAAu8; 20]); // target_address
    payload.extend_from_slice(&word(1)); // target_nonce
    payload.extend_from_slice(&word(500)); // amount, in luna
    payload.extend_from_slice(&word(80_002)); // target_chain_id
    payload.extend_from_slice(&[0x0Bu8; 20]); // bridge_address
    assert_eq!(payload.len(), AMOY_RECORD_LEN);
    assert!(total_len >= payload.len());
    payload.resize(total_len, 0xAB);
    payload
}

/// Serialized `sender_data` for a release carrying `payload_len` payload bytes and `proof`.
/// The signature proof is a real (not default) one so the envelope is realistically sized.
fn sender_data(payload_len: usize, proof: AnyMerkleProof) -> Vec<u8> {
    let key = key_pair();
    OutgoingBridgeTransactionData {
        burn_proof: burn_proof(payload_len, proof),
        proof: SignatureProof::from_ed25519(key.public, key.sign(b"content")),
    }
    .serialize_to_vec()
}

fn empty_path() -> AnyMerkleProof {
    AnyMerkleProof::Blake2bPath(MerklePath::empty())
}

/// A Merkle path with `nodes` sibling hashes.
fn path_with_nodes(nodes: usize) -> AnyMerkleProof {
    use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};
    let sibling = Blake2bHasher::default().digest(b"sibling");
    AnyMerkleProof::Blake2bPath(MerklePath::<Blake2bHash>::from_sibling_hashes(
        vec![sibling; nodes],
        vec![true; nodes],
    ))
}

fn release_tx(sender_data: Vec<u8>) -> Transaction {
    Transaction::new_extended(
        Address::from([0x0Bu8; 20]),
        AccountType::Bridge,
        sender_data,
        Address::from([0xAAu8; 20]),
        AccountType::Basic,
        vec![],
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    )
}

/// True if the transaction survives a serialize/deserialize round trip, i.e. if a node would
/// accept it off the wire or out of a block body.
fn survives_the_wire(tx: &Transaction) -> bool {
    Transaction::deserialize_from_vec(&tx.serialize_to_vec()).is_ok()
}

/// Bytes the `OutgoingBridgeTransactionData` envelope adds around the payload, measured rather
/// than hard-coded. Measured at a payload length in the same varint-length bucket
/// (128..=16_383 bytes) as the maximum payload, so the figure applies there too.
fn envelope_overhead(proof: AnyMerkleProof) -> usize {
    sender_data(1024, proof).len() - 1024
}

// ---------------------------------------------------------------------------------------------
// The bound that is actually enforced
// ---------------------------------------------------------------------------------------------

/// `sender_data` above `Policy::MAX_TX_SENDER_DATA_SIZE` cannot be deserialized, so an oversized
/// burn payload never reaches `commit_outgoing_transaction` in the first place. It is a transport
/// check rather than a bridge check.
#[test]
fn bridge_sender_data_over_the_policy_cap_is_rejected_off_the_wire() {
    let at_cap = release_tx(vec![0u8; Policy::MAX_TX_SENDER_DATA_SIZE]);
    assert!(
        survives_the_wire(&at_cap),
        "sender_data of exactly MAX_TX_SENDER_DATA_SIZE must be accepted"
    );

    let over_cap = release_tx(vec![0u8; Policy::MAX_TX_SENDER_DATA_SIZE + 1]);
    assert!(
        !survives_the_wire(&over_cap),
        "one byte over MAX_TX_SENDER_DATA_SIZE must be rejected"
    );
}

/// The payload ceiling a release can actually carry: the policy cap minus the envelope around it.
/// Pinned so that a change to either the cap or the `OutgoingBridgeTransactionData` layout is a
/// deliberate, visible decision.
#[test]
fn largest_burn_payload_that_can_reach_consensus_is_pinned() {
    let overhead = envelope_overhead(empty_path());
    assert_eq!(
        overhead, 103,
        "envelope size changed; the payload ceiling moved with it"
    );

    let max_payload = Policy::MAX_TX_SENDER_DATA_SIZE - overhead;
    assert_eq!(max_payload, 4_897, "payload ceiling drifted");

    let largest = release_tx(sender_data(max_payload, empty_path()));
    assert_eq!(largest.sender_data.len(), Policy::MAX_TX_SENDER_DATA_SIZE);
    assert!(
        survives_the_wire(&largest),
        "the largest payload must still be accepted"
    );

    let too_large = release_tx(sender_data(max_payload + 1, empty_path()));
    assert!(
        !survives_the_wire(&too_large),
        "one payload byte more must be rejected"
    );
}

/// The payload and the Merkle proof share one `sender_data` budget: a payload that fits with an
/// empty proof stops fitting once the proof carries sibling hashes. Worth pinning because it means
/// the ceiling above is a best case, not a guarantee.
#[test]
fn merkle_proof_nodes_shrink_the_burn_payload_budget() {
    let max_payload = Policy::MAX_TX_SENDER_DATA_SIZE - envelope_overhead(empty_path());

    assert!(survives_the_wire(&release_tx(sender_data(
        max_payload,
        empty_path()
    ))));
    assert!(
        !survives_the_wire(&release_tx(sender_data(max_payload, path_with_nodes(10)))),
        "a 10-node proof must eat into the payload budget"
    );

    let with_proof_overhead = envelope_overhead(path_with_nodes(10));
    assert!(
        with_proof_overhead > envelope_overhead(empty_path()),
        "proof nodes must cost sender_data bytes"
    );
    assert!(survives_the_wire(&release_tx(sender_data(
        Policy::MAX_TX_SENDER_DATA_SIZE - with_proof_overhead,
        path_with_nodes(10)
    ))));
}

// ---------------------------------------------------------------------------------------------
// Bounded work at the ceiling, and the empty payload
// ---------------------------------------------------------------------------------------------

/// A payload at the ceiling is parsed with bounded work and typed errors — never a panic. The
/// program reads fixed offsets, so payload size buys an attacker nothing beyond one hash of at
/// most `MAX_TX_SENDER_DATA_SIZE` bytes.
#[test]
fn max_size_burn_payload_parses_without_panicking() {
    let config = amoy_chain_config();
    let max_payload = Policy::MAX_TX_SENDER_DATA_SIZE - envelope_overhead(empty_path());

    // A valid 156-byte record padded out to the ceiling with filler.
    let outgoing = OutgoingTransaction {
        burn_transaction_data: padded_burn_record(max_payload),
        merkle_proof: empty_path(),
        oracle_state_index: 0,
    };
    assert!(outgoing.transaction_id(&config.hash_function).is_ok());

    let parsed = outgoing
        .parse_burn_data(&config)
        .expect("a ceiling-sized payload must parse");
    assert_eq!(parsed.amount, Coin::from_u64_unchecked(500));
    assert_eq!(parsed.target_nonce, 1);
    assert_eq!(parsed.target_chain_id, 80_002);

    // The trailing filler is never read: the program indexes fixed offsets, so the extracted
    // values are identical to those from a minimal 156-byte payload. Payload size buys an attacker
    // nothing beyond one hash of at most `MAX_TX_SENDER_DATA_SIZE` bytes.
    let minimal = OutgoingTransaction {
        burn_transaction_data: padded_burn_record(AMOY_RECORD_LEN),
        merkle_proof: empty_path(),
        oracle_state_index: 0,
    };
    let minimal_parsed = minimal.parse_burn_data(&config).unwrap();
    assert_eq!(parsed.amount, minimal_parsed.amount);
    assert_eq!(parsed.target_address, minimal_parsed.target_address);
    assert_eq!(parsed.target_nonce, minimal_parsed.target_nonce);
    assert_eq!(parsed.target_chain_id, minimal_parsed.target_chain_id);
    assert_eq!(parsed.source_contract, minimal_parsed.source_contract);
    assert_eq!(parsed.bridge_address, minimal_parsed.bridge_address);
}

/// An empty payload is refused on the path consensus takes, and by the constructor the RPC and the
/// web client build releases with.
#[test]
fn empty_burn_payload_is_rejected_on_the_release_path() {
    let config = amoy_chain_config();
    let empty = burn_proof(0, empty_path());

    assert!(matches!(
        empty.transaction_id(&config.hash_function),
        Err(BridgeError::InvalidDataLength)
    ));
    assert!(matches!(
        empty.parse_burn_data(&config),
        Err(BridgeError::InvalidDataLength)
    ));
    assert!(matches!(
        OutgoingTransaction::new(Vec::new(), empty_path(), 0),
        Err(BridgeError::InvalidDataLength)
    ));
}
