//! Burn-payload size bounds on the consensus release path.
//!
//! `OutgoingTransaction::validate()` documents a 10,000-byte cap on the burn payload, but the
//! consensus release path never calls it: it deserializes `sender_data` with
//! `OutgoingBridgeTransactionData::parse` and then calls `.verify()`, which checks only the
//! signature. `validate_outgoing_transaction_comprehensive`, which does re-check the cap, has no
//! callers at all.
//!
//! What actually bounds the payload is `Policy::MAX_TX_SENDER_DATA_SIZE`, enforced while
//! deserializing an extended transaction — so it applies to every transaction that arrives over
//! the wire or is read out of a block. That bound is *tighter* than the documented cap, which is
//! why the unenforced cap is not currently reachable. These tests pin the real ceiling, pin the
//! discrepancy, and fail loudly if `MAX_TX_SENDER_DATA_SIZE` is ever raised past 10,000 — at which
//! point the unenforced cap would stop being harmless.

use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32},
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, BridgeError, ChainConfig, Endianness, OutgoingTransaction,
        ValidationOp, ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::merkle::MerklePath;

/// The documented (but unenforced) cap in `OutgoingTransaction::validate()`.
const DOCUMENTED_PAYLOAD_CAP: usize = 10_000;

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

/// A well-formed burn record for `chain_config`'s program, padded with filler to `total_len`.
/// The program reads only the first 44 bytes, so the filler is what an attacker would inflate.
fn padded_burn_record(total_len: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(total_len);
    payload.extend_from_slice(&[0xAAu8; 20]); // target_address
    payload.extend_from_slice(&500u64.to_le_bytes()); // amount
    payload.extend_from_slice(&1u64.to_le_bytes()); // target_nonce
    payload.extend_from_slice(&42u32.to_le_bytes()); // burn_block_height
    payload.extend_from_slice(&1u32.to_le_bytes()); // target_chain_id
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

fn chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 1,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        // Reads target_address [0..20], amount [20..28], nonce [28..36],
        // burn_block_height [36..40], target_chain_id [40..44].
        validation_program: ValidationProgram::new(vec![
            ValidationOp::PushConst(0),
            ValidationOp::LoadAddress,
            ValidationOp::Store("target_address".to_string()),
            ValidationOp::PushConst(20),
            ValidationOp::LoadU64(Endianness::LittleEndian),
            ValidationOp::Store("amount".to_string()),
            ValidationOp::PushConst(28),
            ValidationOp::LoadU64(Endianness::LittleEndian),
            ValidationOp::Store("target_nonce".to_string()),
            ValidationOp::PushConst(36),
            ValidationOp::LoadU32(Endianness::LittleEndian),
            ValidationOp::Store("burn_block_height".to_string()),
            ValidationOp::PushConst(40),
            ValidationOp::LoadU32(Endianness::LittleEndian),
            ValidationOp::Store("target_chain_id".to_string()),
        ]),
        max_proof_depth: 64,
    }
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
// The documented cap, and why it is currently unreachable
// ---------------------------------------------------------------------------------------------

/// The 10,000-byte cap is real code that works when called, is *not* applied on the path consensus
/// uses, and cannot currently be reached because the transport cap is tighter.
///
/// The last assertion is the one that matters over time: `MAX_TX_SENDER_DATA_SIZE` has already
/// been raised once for the bridge's EVM-compatible Merkle proofs. If it is raised past 10,000,
/// the unenforced cap stops being harmless and this test fails, forcing a choice: enforce the cap
/// on the deserialized path, or delete the dead validator and document the real bound.
#[test]
fn documented_10kb_burn_payload_cap_is_unreachable_from_the_wire() {
    // The cap works when it is called...
    assert!(matches!(
        burn_proof(DOCUMENTED_PAYLOAD_CAP + 1, empty_path()).validate(),
        Err(BridgeError::InvalidDataLength)
    ));
    assert!(burn_proof(DOCUMENTED_PAYLOAD_CAP, empty_path())
        .validate()
        .is_ok());

    // ...but it is not applied on the path consensus takes: `parse` deserializes an oversized
    // payload happily, and `verify` only checks the signature.
    let oversized = release_tx(sender_data(DOCUMENTED_PAYLOAD_CAP + 1_000, empty_path()));
    let parsed = OutgoingBridgeTransactionData::parse(&oversized)
        .expect("parse does not apply the payload cap");
    assert_eq!(
        parsed.burn_proof.burn_transaction_data.len(),
        DOCUMENTED_PAYLOAD_CAP + 1_000,
    );

    // It stays harmless only because such a transaction cannot be deserialized at all.
    assert!(
        !survives_the_wire(&oversized),
        "the transport cap is what keeps the unenforced payload cap harmless"
    );
    // The guard that has to hold over time, stated behaviourally so it is actually compiled: a
    // payload *at* the documented cap must not be able to arrive at all. The day
    // `MAX_TX_SENDER_DATA_SIZE` grows enough for one to arrive, this fails and forces the choice
    // between enforcing the cap and removing it.
    assert!(
        !survives_the_wire(&release_tx(sender_data(
            DOCUMENTED_PAYLOAD_CAP,
            empty_path()
        ))),
        "a burn payload at the unenforced {DOCUMENTED_PAYLOAD_CAP}-byte cap can now reach \
         consensus (MAX_TX_SENDER_DATA_SIZE is {}): the cap must be enforced on the deserialized \
         path, or deleted and the real bound documented",
        Policy::MAX_TX_SENDER_DATA_SIZE,
    );
}

// ---------------------------------------------------------------------------------------------
// Bounded work at the ceiling, and the empty payload
// ---------------------------------------------------------------------------------------------

/// A payload at the ceiling is parsed with bounded work and typed errors — never a panic. The
/// program reads fixed offsets, so payload size buys an attacker nothing beyond one hash of at
/// most `MAX_TX_SENDER_DATA_SIZE` bytes.
#[test]
fn max_size_burn_payload_parses_without_panicking() {
    let config = chain_config();
    let max_payload = Policy::MAX_TX_SENDER_DATA_SIZE - envelope_overhead(empty_path());

    // A valid 44-byte record padded out to the ceiling with filler.
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
    assert_eq!(parsed.target_chain_id, 1);

    // The trailing filler is never read: the program indexes fixed offsets, so the extracted
    // values are identical to those from a minimal 44-byte payload. Payload size buys an attacker
    // nothing beyond one hash of at most `MAX_TX_SENDER_DATA_SIZE` bytes.
    let minimal = OutgoingTransaction {
        burn_transaction_data: padded_burn_record(44),
        merkle_proof: empty_path(),
        oracle_state_index: 0,
    };
    let minimal_parsed = minimal.parse_burn_data(&config).unwrap();
    assert_eq!(parsed.amount, minimal_parsed.amount);
    assert_eq!(parsed.target_address, minimal_parsed.target_address);
    assert_eq!(parsed.target_nonce, minimal_parsed.target_nonce);
    assert_eq!(parsed.target_chain_id, minimal_parsed.target_chain_id);
}

/// The other half of the unreachable `validate()`: empty payloads. The risk rationale rests on
/// these being caught downstream, so pin that they are — on the path consensus actually takes.
#[test]
fn empty_burn_payload_is_rejected_on_the_release_path() {
    let config = chain_config();
    let empty = burn_proof(0, empty_path());

    assert!(matches!(
        empty.transaction_id(&config.hash_function),
        Err(BridgeError::InvalidDataLength)
    ));
    assert!(matches!(
        empty.parse_burn_data(&config),
        Err(BridgeError::InvalidDataLength)
    ));
    // And the unreachable validator would have rejected it too.
    assert!(matches!(
        empty.validate(),
        Err(BridgeError::InvalidDataLength)
    ));
}
