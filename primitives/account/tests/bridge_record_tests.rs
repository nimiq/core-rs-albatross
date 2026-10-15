//! What consensus accepts as a release record.
//!
//! A release carries a record the relayer committed to the record tree, for a forwarded record
//! `source ++ data`, and one `MerklePath` from the record's leaf through the tree and the oracle
//! slot. Anyone can forward a record of any content and length, so the leaf rule has to keep
//! records apart from inner nodes, and a record releases only if it comes from the bridge's
//! source contract and names the bridge executing the release.

use nimiq_account::{
    Account, AccountsError, BlockLogger, BlockState, BridgeContract, OperationReceipt,
    OracleContract, TransactionLog,
};
use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::accounts_revert::TestCommitRevert;
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32, HashType},
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, Endianness, OutgoingTransaction, ValidationOp,
        ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::merkle::MerklePath;

const SOURCE_CHAIN_ID: u32 = 1;
const AMOUNT: u64 = 500;
const BRIDGE_DEPOSIT: u64 = 1_000_000;
/// The contract on the source chain whose records the bridge releases.
const SOURCE_CONTRACT: [u8; 20] = [0x5Cu8; 20];

fn oracle_addr() -> Address {
    Address::from([0x0Eu8; 20])
}

fn bridge_addr() -> Address {
    Address::from([0x0Bu8; 20])
}

/// Another bridge with the same oracle and source contract.
fn second_bridge_addr() -> Address {
    Address::from([0x0Cu8; 20])
}

fn target_key() -> KeyPair {
    KeyPair::from(PrivateKey::from([0xAAu8; 32]))
}

fn attacker_key() -> KeyPair {
    KeyPair::from(PrivateKey::from([0xEEu8; 32]))
}

fn address(key: &KeyPair) -> Address {
    Address::from(&key.public)
}

fn blake2b(data: &[u8]) -> AnyHash {
    AnyHash::from(Blake2bHasher::default().digest(data))
}

/// The record-tree leaf of a record, `blake2b(blake2b(record))`.
fn record_leaf(record: &[u8]) -> AnyHash {
    blake2b(blake2b(record).as_bytes())
}

fn zero() -> AnyHash {
    AnyHash::Blake2b(AnyHash32::default())
}

/// A burn of `AMOUNT` luna to `target` with `nonce`, forwarded by the source contract and naming
/// the bridge at `bridge_addr()`, in the layout `chain_config` reads.
fn burn_record(target: &Address, nonce: u64) -> Vec<u8> {
    record_from(&SOURCE_CONTRACT, target, nonce, &bridge_addr())
}

/// A burn of `AMOUNT` luna to `target` with `nonce`, forwarded by `source` and naming `bridge`.
fn record_from(source: &[u8; 20], target: &Address, nonce: u64, bridge: &Address) -> Vec<u8> {
    let mut record = target.as_bytes().to_vec();
    record.extend_from_slice(&AMOUNT.to_le_bytes());
    record.extend_from_slice(&nonce.to_le_bytes());
    record.extend_from_slice(&42u32.to_le_bytes());
    record.extend_from_slice(&SOURCE_CHAIN_ID.to_le_bytes());
    record.extend_from_slice(source);
    record.extend_from_slice(bridge.as_bytes());
    record
}

/// Reads `[0..20]` target address, `[20..28]` amount, `[28..36]` nonce, `[36..40]` burn block
/// height and `[40..44]` target chain ID, all little-endian, then `[44..64]` source contract and
/// `[64..84]` bridge address.
fn chain_config() -> ChainConfig {
    let load = |offset, op, name: &str| {
        [
            ValidationOp::PushConst(offset),
            op,
            ValidationOp::Store(name.to_string()),
        ]
    };
    let le = Endianness::LittleEndian;
    let program = [
        load(0, ValidationOp::LoadAddress, "target_address"),
        load(20, ValidationOp::LoadU64(le), "amount"),
        load(28, ValidationOp::LoadU64(le), "target_nonce"),
        load(36, ValidationOp::LoadU32(le), "burn_block_height"),
        load(40, ValidationOp::LoadU32(le), "target_chain_id"),
        load(44, ValidationOp::LoadAddress, "source_contract"),
        load(64, ValidationOp::LoadAddress, "bridge_address"),
    ]
    .concat();

    ChainConfig {
        chain_id: SOURCE_CHAIN_ID,
        hash_function: zero(),
        address_format: AddressFormat::Nimiq,
        endianness: le,
        block_time: std::time::Duration::from_secs(60),
        validation_program: ValidationProgram::new(program),
        max_proof_depth: 64,
        source_contract: SOURCE_CONTRACT.to_vec(),
    }
}

fn bridge() -> BridgeContract {
    BridgeContract {
        owner: Address::from([0x01u8; 20]),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 0,
    }
}

/// An oracle that has written `root` at index 0, into slot 0's zero hash.
fn oracle_with_root(root: &AnyHash) -> OracleContract {
    let hash_count = 4;
    let mut hashes = vec![zero(); hash_count];
    hashes[0] = zero().digest(root);
    OracleContract {
        owner: Address::from([0x02u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count: hash_count as u16,
        hash_algorithm: HashType::Blake2b,
        hashes,
        latest_index: Some(0),
    }
}

/// A Blake2b Merkle path from `(sibling, sibling_is_left)` levels, leaf first.
fn path(levels: &[(AnyHash, bool)]) -> MerklePath<Blake2bHash> {
    let (siblings, left) = levels
        .iter()
        .map(|(hash, left)| {
            let bytes = <[u8; 32]>::try_from(hash.as_bytes()).unwrap();
            (Blake2bHash::from(bytes), *left)
        })
        .unzip();
    MerklePath::from_sibling_hashes(siblings, left)
}

/// A release of `record` from the bridge at `bridge_addr()` to the key's address, proven by
/// `levels` against oracle index 0 and signed by `key`.
fn release(record: &[u8], levels: &[(AnyHash, bool)], key: &KeyPair) -> Transaction {
    release_from(&bridge_addr(), record, levels, key)
}

/// Like `release`, from the bridge at `bridge`.
fn release_from(
    bridge: &Address,
    record: &[u8],
    levels: &[(AnyHash, bool)],
    key: &KeyPair,
) -> Transaction {
    let mut data = OutgoingBridgeTransactionData {
        burn_proof: OutgoingTransaction {
            burn_transaction_data: record.to_vec(),
            merkle_proof: AnyMerkleProof::Blake2bPath(path(levels)),
            oracle_state_index: 0,
        },
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        bridge.clone(),
        AccountType::Bridge,
        data.serialize_to_vec(),
        address(key),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(AMOUNT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );
    data.set_signature(SignatureProof::from_ed25519(
        key.public,
        key.sign(&tx.serialize_content()),
    ));
    tx.sender_data = data.serialize_to_vec();
    tx
}

/// Two fresh bridges with the same oracle, `oracle`, and the same source contract.
fn accounts(oracle: &OracleContract) -> TestCommitRevert {
    TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle.clone())),
        (bridge_addr(), Account::Bridge(bridge())),
        (second_bridge_addr(), Account::Bridge(bridge())),
    ])
}

fn block_state() -> BlockState {
    BlockState::new(1, 1, Policy::max_supported_version())
}

/// Commits `tx` in a block against the fresh bridges of `accounts(oracle)`, checks that it
/// succeeded, and that reverting the block restores the accounts.
fn assert_released(oracle: &OracleContract, tx: &Transaction) {
    let receipts = accounts(oracle)
        .commit_and_test(
            std::slice::from_ref(tx),
            &[],
            &block_state(),
            &mut BlockLogger::empty(),
        )
        .expect("the block must commit");
    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Ok(_)),
        "the release must succeed: {receipts:?}"
    );
}

/// The error the bridge sending `tx` rejects it with, against the fresh bridges of
/// `accounts(oracle)`. A failing release makes its block invalid, for the same reason.
fn rejection(oracle: &OracleContract, tx: &Transaction) -> AccountError {
    // In a block first, so that a release that is paid out fails here rather than in the
    // harness's revert check below, which cannot revert a successful release.
    let block_error = match accounts(oracle).commit_and_test(
        std::slice::from_ref(tx),
        &[],
        &block_state(),
        &mut BlockLogger::empty(),
    ) {
        Err(AccountsError::InvalidTransaction(error, _)) => error,
        other => panic!("the release must make its block invalid, got {other:?}"),
    };
    let error = accounts(oracle)
        .test_commit_outgoing_transaction(
            &mut bridge(),
            tx,
            &block_state(),
            &mut TransactionLog::empty(),
            false,
        )
        .expect_err("the release must be rejected");
    assert_eq!(block_error, error);
    error
}

/// Review finding K1. A forwarded record of exactly 64 bytes hashes like an inner node. With
/// single-hashed leaves, an attacker forwards one whose second half is the leaf of a forged burn
/// record, and the forged record proves through it with one extra path node: the bridge pays out
/// a burn that never happened. Leaves hash the record twice, so a leaf's outer hash takes 32 bytes
/// and never equals a node, whichever second half the attacker picks.
#[test]
fn a_64_byte_record_cannot_serve_as_an_inner_node_for_a_forged_record() {
    let attacker = attacker_key();
    // The forged record names the source contract and this bridge, so it passes every record
    // check; only the leaf rule stands in its way.
    let forged = burn_record(&address(&attacker), 1);
    let honest = burn_record(&address(&target_key()), 1);

    // The first 32 bytes of the planted record are the forwarding contract and the start of its
    // data, whatever they are; the forged path takes them as its first sibling.
    let prefix = [0x5Au8; 32];
    let plant = |second_half: &AnyHash| [&prefix[..], second_half.as_bytes()].concat();
    let prefix_node = (AnyHash::Blake2b(AnyHash32::from(prefix)), true);

    // What K1 was: under single-hashed leaves the forged record proves into the tree holding the
    // planted record, so a release would have paid it.
    let old_leaf = blake2b;
    let planted_for_old_rule = plant(&old_leaf(&forged));
    assert_eq!(planted_for_old_rule.len(), 64);
    let old_root = old_leaf(&planted_for_old_rule).digest(&old_leaf(&honest));
    let old_forged_levels = [prefix_node.clone(), (old_leaf(&honest), false)];
    let folded =
        path(&old_forged_levels).compute_root_from_hash(Blake2bHasher::default().digest(&forged));
    assert_eq!(AnyHash::from(folded), old_root);

    // A relayer still on the old rule, writing that root, does not get the forged record paid.
    let forged_levels = [&old_forged_levels[..], &[(zero(), true)]].concat();
    assert_eq!(
        rejection(
            &oracle_with_root(&old_root),
            &release(&forged, &forged_levels, &attacker)
        ),
        AccountError::InvalidTransaction(TransactionError::InvalidProof),
        "the forged record must not prove into a single-hashed tree",
    );

    // Under the current rule, the attacker can plant either the old leaf of the forged record or
    // its current one; neither turns the planted record into a node.
    for planted in [planted_for_old_rule, plant(&record_leaf(&forged))] {
        let root = record_leaf(&planted).digest(&record_leaf(&honest));
        let oracle = oracle_with_root(&root);

        // The honest record next to it releases, so the tree and the slot are as described.
        let honest_levels = [(record_leaf(&planted), true), (zero(), true)];
        assert_released(&oracle, &release(&honest, &honest_levels, &target_key()));

        // The forged record, proven through the planted record's position, does not.
        let forged_levels = [
            prefix_node.clone(),
            (record_leaf(&honest), false),
            (zero(), true),
        ];
        assert_eq!(
            rejection(&oracle, &release(&forged, &forged_levels, &attacker)),
            AccountError::InvalidTransaction(TransactionError::InvalidProof),
            "the forged record must not prove through a 64-byte record",
        );
    }
}

/// A tree of a single record is legal: its root is the record's leaf, so the path holds only the
/// slot's buffer levels and the first burn through a fresh bridge can be released.
#[test]
fn a_single_record_tree_has_the_double_hashed_leaf_as_its_root() {
    let record = burn_record(&address(&target_key()), 1);
    let tx = release(&record, &[(zero(), true)], &target_key());
    assert_released(&oracle_with_root(&record_leaf(&record)), &tx);

    // A root written as the record's single hash is not its leaf.
    assert_eq!(
        rejection(&oracle_with_root(&blake2b(&record)), &tx),
        AccountError::InvalidTransaction(TransactionError::InvalidProof)
    );
}

/// The relayer commits records from every contract that forwards data, so the tree also holds
/// burn-shaped records anyone can write. Only those from the bridge's source contract release:
/// another contract's record is refused although the oracle commits to it, and the same burn
/// forwarded by the source contract is paid.
#[test]
fn a_record_from_a_contract_other_than_the_source_is_rejected() {
    let target = address(&target_key());
    for source in [[0x5Du8; 20], [0x00u8; 20], [0xFFu8; 20]] {
        let record = record_from(&source, &target, 1, &bridge_addr());
        let oracle = oracle_with_root(&record_leaf(&record));
        assert_eq!(
            rejection(&oracle, &release(&record, &[(zero(), true)], &target_key())),
            AccountError::InvalidTransaction(TransactionError::InvalidData),
            "a record forwarded by {} must not release",
            hex::encode(source),
        );
    }

    let record = record_from(&SOURCE_CONTRACT, &target, 1, &bridge_addr());
    assert_released(
        &oracle_with_root(&record_leaf(&record)),
        &release(&record, &[(zero(), true)], &target_key()),
    );
}

/// Review finding K3. A burn names the Nimiq bridge that may release it. A second bridge with the
/// same oracle and source contract starts with empty nonce counters, so without the name it could
/// release every burn the first bridge already paid, since proofs never expire.
#[test]
fn a_burn_naming_another_nimiq_bridge_is_rejected() {
    let record = burn_record(&address(&target_key()), 1);
    let oracle = oracle_with_root(&record_leaf(&record));
    let levels = [(zero(), true)];

    // The bridge the burn names releases it...
    assert_released(&oracle, &release(&record, &levels, &target_key()));

    // ...and the second bridge does not, although the record, its proof and the target's nonce
    // there are all valid.
    assert_eq!(
        rejection(
            &oracle,
            &release_from(&second_bridge_addr(), &record, &levels, &target_key())
        ),
        AccountError::InvalidTransaction(TransactionError::InvalidData),
    );

    // A burn naming the second bridge releases only there.
    let record = record_from(
        &SOURCE_CONTRACT,
        &address(&target_key()),
        1,
        &second_bridge_addr(),
    );
    let oracle = oracle_with_root(&record_leaf(&record));
    assert_eq!(
        rejection(&oracle, &release(&record, &levels, &target_key())),
        AccountError::InvalidTransaction(TransactionError::InvalidData),
    );
    assert_released(
        &oracle,
        &release_from(&second_bridge_addr(), &record, &levels, &target_key()),
    );
}
