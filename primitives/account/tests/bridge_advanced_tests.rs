use nimiq_account::{
    Account, AccountTransactionInteraction, BasicAccount, BlockLogger, BlockState, BridgeContract,
    OperationReceipt, Receipts, ReservedBalance,
};
use nimiq_database::traits::{Database, WriteTransaction};
use nimiq_hash::{Blake2bHasher, Hasher, Keccak256Hasher, Sha256Hasher};
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::accounts_revert::TestCommitRevert;
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32},
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, Endianness, OutgoingTransaction, ValidationOp,
        ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::{key_rng::SecureGenerate, merkle::MerklePath};

// =====================================================================
// Shared helpers
// =====================================================================

const SOURCE_CHAIN_ID: u32 = 1;
const RELEASE_AMOUNT: u64 = 500;
const BURN_BLOCK_HEIGHT: u32 = 42;
const BRIDGE_DEPOSIT: u64 = 10_000;

fn nimiq_target() -> Address {
    Address::from([0xAAu8; 20])
}
fn oracle_addr() -> Address {
    Address::from([0x0Eu8; 20])
}
fn bridge_addr() -> Address {
    Address::from([0x0Bu8; 20])
}

fn standard_program() -> ValidationProgram {
    ValidationProgram::new(vec![
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
    ])
}

fn chain_config_with_hash(hash_function: AnyHash) -> ChainConfig {
    ChainConfig {
        chain_id: SOURCE_CHAIN_ID,
        hash_function,
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: standard_program(),
        max_proof_depth: 64,
    }
}

fn make_burn_data(target: [u8; 20], amount: u64, nonce: u64, chain_id: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity(44);
    d.extend_from_slice(&target);
    d.extend_from_slice(&amount.to_le_bytes());
    d.extend_from_slice(&nonce.to_le_bytes());
    d.extend_from_slice(&BURN_BLOCK_HEIGHT.to_le_bytes());
    d.extend_from_slice(&chain_id.to_le_bytes());
    d
}

fn commit_block(test: &TestCommitRevert, txs: &[Transaction], bs: &BlockState) -> Receipts {
    let env = test.env();
    let mut raw = env.write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut raw).into();
    let r = test
        .commit(&mut txn, txs, &[], bs, &mut BlockLogger::empty())
        .unwrap();
    raw.commit();
    r
}

/// Build a signed outgoing bridge tx using an explicit Merkle-proof variant and signer.
fn make_outgoing_tx_full(
    amount: u64,
    burn_data: Vec<u8>,
    merkle_proof: AnyMerkleProof,
    oracle_state_index: u64,
    signer: &KeyPair,
) -> Transaction {
    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        merkle_proof,
        oracle_state_index,
    };
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof: outgoing,
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        bridge_addr(),
        AccountType::Bridge,
        bridge_data.serialize_to_vec(),
        nimiq_target(),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(amount),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );
    let sig = signer.sign(&tx.serialize_content());
    bridge_data.set_signature(SignatureProof::from_ed25519(signer.public.clone(), sig));
    tx.sender_data = bridge_data.serialize_to_vec();
    tx
}

// =====================================================================
// reserve_balance / release_balance
// =====================================================================

/// A correctly owner-signed outgoing tx is accepted by `reserve_balance`, and
/// `release_balance` returns the reserved amount.
#[test]
fn bridge_reserve_and_release_balance_owner() {
    let owner = KeyPair::generate_default_csprng();
    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(AnyHash::Blake2b(AnyHash32::default())),
        transaction_count: 0,
    };

    let test =
        TestCommitRevert::with_initial_state(&[(bridge_addr(), Account::Bridge(bridge.clone()))]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let mut db_txn = test.env().write_transaction();
    let data_store = test.data_store(&bridge_addr());

    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &owner,
    );

    let mut reserved = ReservedBalance::new(bridge_addr());

    // Reserve succeeds with the owner's signature.
    let res = bridge.reserve_balance(&tx, &mut reserved, &bs, data_store.read(&mut db_txn));
    assert!(res.is_ok(), "owner-signed reserve must succeed: {res:?}");
    assert_eq!(reserved.balance(), Coin::from_u64_unchecked(RELEASE_AMOUNT));

    // Release returns the reserved amount.
    let rel = bridge.release_balance(&tx, &mut reserved, data_store.read(&mut db_txn));
    assert!(rel.is_ok());
    assert_eq!(reserved.balance(), Coin::ZERO);
}

/// `reserve_balance` rejects an outgoing tx whose burn-proof signature is NOT
/// the bridge owner. This is the mempool-admission half of the intentional
/// asymmetry: the same tx is accepted at block execution (see the
/// permissionless test below).
#[test]
fn bridge_reserve_balance_rejects_non_owner() {
    let owner = KeyPair::generate_default_csprng();
    let relayer = KeyPair::generate_default_csprng(); // not the owner

    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(AnyHash::Blake2b(AnyHash32::default())),
        transaction_count: 0,
    };

    let test =
        TestCommitRevert::with_initial_state(&[(bridge_addr(), Account::Bridge(bridge.clone()))]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let mut db_txn = test.env().write_transaction();
    let data_store = test.data_store(&bridge_addr());

    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &relayer,
    );

    let mut reserved = ReservedBalance::new(bridge_addr());
    let res = bridge.reserve_balance(&tx, &mut reserved, &bs, data_store.read(&mut db_txn));
    assert_eq!(
        res,
        Err(AccountError::InvalidSignature),
        "reserve_balance must reject a non-owner burn-proof signature"
    );
}

/// `reserve_balance` rejects when the requested total exceeds the bridge balance.
#[test]
fn bridge_reserve_balance_rejects_insufficient_funds() {
    let owner = KeyPair::generate_default_csprng();
    let small_balance = 100u64; // less than RELEASE_AMOUNT (500)
    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(small_balance),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(AnyHash::Blake2b(AnyHash32::default())),
        transaction_count: 0,
    };

    let test =
        TestCommitRevert::with_initial_state(&[(bridge_addr(), Account::Bridge(bridge.clone()))]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let mut db_txn = test.env().write_transaction();
    let data_store = test.data_store(&bridge_addr());

    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &owner,
    );

    let mut reserved = ReservedBalance::new(bridge_addr());
    let res = bridge.reserve_balance(&tx, &mut reserved, &bs, data_store.read(&mut db_txn));
    assert!(
        matches!(res, Err(AccountError::InsufficientFunds { .. })),
        "reserve must fail with InsufficientFunds, got {res:?}"
    );
}

// =====================================================================
// Permissionless burn-proof submission (block execution)
// =====================================================================

/// A valid burn proof signed by a NON-owner relayer is accepted by
/// `commit_outgoing_transaction`. The owner-signature check is intentionally
/// skipped at block execution to allow permissionless relaying, even though
/// `reserve_balance` (mempool) rejects the same tx.
#[test]
fn bridge_permissionless_non_owner_submission_succeeds_at_commit() {
    let owner = KeyPair::generate_default_csprng();
    let relayer = KeyPair::generate_default_csprng(); // not the owner

    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    // Oracle state[0] = zero.digest(Blake2b(burn_data)); empty path => root == leaf.
    let leaf = AnyHash::from(Blake2bHasher::default().digest(&burn_data));
    let zero = leaf.zero_of_same_type();
    let mut hashes = vec![zero.clone(); 10];
    hashes[0] = zero.digest(&leaf);
    let oracle = nimiq_account::OracleContract {
        owner: Address::from([0x01u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count: 10,
        hashes,
        latest_index: Some(0),
    };

    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(AnyHash::Blake2b(AnyHash32::default())),
        transaction_count: 0,
    };

    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
        (
            nimiq_target(),
            Account::Basic(BasicAccount {
                balance: Coin::ZERO,
            }),
        ),
    ]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());

    // Signed by the relayer, NOT the owner. Zero fee avoids the signer-fee path.
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &relayer,
    );

    let receipts = commit_block(&test, &[tx], &bs);
    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Ok(_)),
        "permissionless non-owner burn submission must succeed at block execution"
    );
}

// =====================================================================
// Non-Blake2b hash types end-to-end
// =====================================================================

#[derive(Clone, Copy)]
enum HashKind {
    Sha256,
    Keccak256,
}

fn leaf_for(kind: HashKind, data: &[u8]) -> AnyHash {
    match kind {
        HashKind::Sha256 => AnyHash::from(Sha256Hasher::default().digest(data)),
        HashKind::Keccak256 => AnyHash::from(Keccak256Hasher::default().digest(data)),
    }
}

fn config_hash_for(kind: HashKind) -> AnyHash {
    match kind {
        HashKind::Sha256 => AnyHash::Sha256(AnyHash32::default()),
        HashKind::Keccak256 => AnyHash::Keccak256(AnyHash32::default()),
    }
}

fn empty_path_for(kind: HashKind) -> AnyMerkleProof {
    match kind {
        HashKind::Sha256 => AnyMerkleProof::Sha256Path(MerklePath::empty()),
        HashKind::Keccak256 => AnyMerkleProof::Keccak256Path(MerklePath::empty()),
    }
}

/// Runs a full successful outgoing-transaction commit for a given hash type,
/// proving the leaf-hash / oracle-state / proof machinery is hash-agnostic.
fn run_outgoing_success_for(kind: HashKind) {
    let owner = KeyPair::generate_default_csprng();
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);

    // Oracle state[0] = zero.digest(H(burn_data)) for the chosen hash type.
    let leaf = leaf_for(kind, &burn_data);
    let zero = leaf.zero_of_same_type();
    let mut hashes = vec![zero.clone(); 10];
    hashes[0] = zero.digest(&leaf);
    let oracle = nimiq_account::OracleContract {
        owner: Address::from([0x01u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count: 10,
        hashes,
        latest_index: Some(0),
    };

    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(config_hash_for(kind)),
        transaction_count: 0,
    };

    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
        (
            nimiq_target(),
            Account::Basic(BasicAccount {
                balance: Coin::ZERO,
            }),
        ),
    ]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());

    let tx = make_outgoing_tx_full(RELEASE_AMOUNT, burn_data, empty_path_for(kind), 0, &owner);
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Ok(_)),
        "outgoing commit must succeed for the selected hash type"
    );
}

#[test]
fn bridge_outgoing_sha256_end_to_end() {
    run_outgoing_success_for(HashKind::Sha256);
}

#[test]
fn bridge_outgoing_keccak256_end_to_end() {
    run_outgoing_success_for(HashKind::Keccak256);
}

/// Sanity check that a mismatched proof variant (Blake2b path against a
/// Keccak-configured bridge) is rejected rather than silently mis-verified.
#[test]
fn bridge_outgoing_rejects_mismatched_proof_variant() {
    let owner = KeyPair::generate_default_csprng();
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);

    let leaf = leaf_for(HashKind::Keccak256, &burn_data);
    let zero = leaf.zero_of_same_type();
    let mut hashes = vec![zero.clone(); 10];
    hashes[0] = zero.digest(&leaf);
    let oracle = nimiq_account::OracleContract {
        owner: Address::from([0x01u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count: 10,
        hashes,
        latest_index: Some(0),
    };

    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(config_hash_for(HashKind::Keccak256)),
        transaction_count: 0,
    };

    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
        (
            nimiq_target(),
            Account::Basic(BasicAccount {
                balance: Coin::ZERO,
            }),
        ),
    ]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());

    // Bridge expects Keccak256, but the proof is a Blake2b path → compute_root
    // returns an error (variant mismatch).
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Err(_, _)),
        "mismatched proof variant must be rejected"
    );
}
