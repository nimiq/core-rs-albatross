use nimiq_account::{
    Account, AccountTransactionInteraction, BasicAccount, BlockLogger, BlockState, BridgeContract,
    OperationReceipt, Receipts, ReservedBalance, TransactionLog,
};
use nimiq_database::traits::{Database, WriteTransaction};
use nimiq_hash::{Blake2bHasher, Hasher, Keccak256Hasher, Sha256Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    policy::Policy,
};
use nimiq_serde::{Deserialize, Serialize};
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

/// The key of `nimiq_target()`. Only the target can sign its releases.
fn target_key() -> KeyPair {
    KeyPair::from(PrivateKey::from([0xAAu8; 32]))
}

fn nimiq_target() -> Address {
    Address::from(&target_key().public)
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

fn make_burn_data(target: &Address, amount: u64, nonce: u64, chain_id: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity(44);
    d.extend_from_slice(target.as_bytes());
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
    make_outgoing_tx_with_fee(
        amount,
        0,
        burn_data,
        merkle_proof,
        oracle_state_index,
        signer,
    )
}

/// Like `make_outgoing_tx_full`, with a fee. The fee comes out of the burned amount, so the
/// release is only valid if `value + fee` is the amount in `burn_data`.
fn make_outgoing_tx_with_fee(
    value: u64,
    fee: u64,
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
        Coin::from_u64_unchecked(value),
        Coin::from_u64_unchecked(fee),
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

/// A target-signed outgoing tx is accepted by `reserve_balance`, and
/// `release_balance` returns the reserved amount.
#[test]
fn bridge_reserve_and_release_balance() {
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

    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &target_key(),
    );

    let mut reserved = ReservedBalance::new(bridge_addr());

    // Reserve succeeds with the target's signature.
    let res = bridge.reserve_balance(&tx, &mut reserved, &bs, data_store.read(&mut db_txn));
    assert!(res.is_ok(), "target-signed reserve must succeed: {res:?}");
    assert_eq!(reserved.balance(), Coin::from_u64_unchecked(RELEASE_AMOUNT));

    // Release returns the reserved amount.
    let rel = bridge.release_balance(&tx, &mut reserved, data_store.read(&mut db_txn));
    assert!(rel.is_ok());
    assert_eq!(reserved.balance(), Coin::ZERO);
}

/// `reserve_balance` accepts an outgoing tx whose burn-proof signature is NOT
/// the bridge owner. A release is authorized by its Merkle burn-proof and the
/// target's signature, both verified at commit, so mempool admission must
/// accept the same transactions block execution accepts. (An owner-signature
/// check here used to diverge the mempool from consensus.)
#[test]
fn bridge_reserve_balance_accepts_non_owner() {
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

    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &target_key(),
    );

    let mut reserved = ReservedBalance::new(bridge_addr());
    let res = bridge.reserve_balance(&tx, &mut reserved, &bs, data_store.read(&mut db_txn));
    assert!(
        res.is_ok(),
        "reserve_balance must accept a non-owner burn-proof signature: {res:?}"
    );
    assert_eq!(reserved.balance(), Coin::from_u64_unchecked(RELEASE_AMOUNT));
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

    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &target_key(),
    );

    let mut reserved = ReservedBalance::new(bridge_addr());
    let res = bridge.reserve_balance(&tx, &mut reserved, &bs, data_store.read(&mut db_txn));
    assert!(
        matches!(res, Err(AccountError::InsufficientFunds { .. })),
        "reserve must fail with InsufficientFunds, got {res:?}"
    );
}

// =====================================================================
// Release authorization: signed by the target, submitted by anyone
// =====================================================================

fn blake2b_bridge(owner: &KeyPair) -> BridgeContract {
    BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config_with_hash(AnyHash::Blake2b(AnyHash32::default())),
        transaction_count: 0,
    }
}

/// `bridge`, an oracle attesting `burn_data` at index 0 so that an empty proof verifies, and the
/// given extra accounts.
fn env_with_attested_burn(
    burn_data: &[u8],
    bridge: BridgeContract,
    accounts: &[(Address, Account)],
) -> TestCommitRevert {
    let leaf = AnyHash::from(Blake2bHasher::default().digest(burn_data));
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

    let mut state = vec![
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
    ];
    state.extend_from_slice(accounts);
    TestCommitRevert::with_initial_state(&state)
}

/// Only the burn's target can sign its release. The release consumes the target's nonce, so a
/// release signed by anyone else could spend that nonce on a transfer the target did not choose.
/// Neither the bridge owner nor a relayer can release on the target's behalf, and a rejected
/// release leaves the bridge and the target's nonce untouched.
#[test]
fn a_release_signed_by_anyone_but_the_target_is_rejected() {
    let owner = KeyPair::generate_default_csprng();
    let relayer = KeyPair::generate_default_csprng();
    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let bridge = blake2b_bridge(&owner);
    let test = env_with_attested_burn(&burn_data, bridge.clone(), &[]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());

    for (who, signer) in [("the bridge owner", &owner), ("a relayer", &relayer)] {
        let tx = make_outgoing_tx_full(
            RELEASE_AMOUNT,
            burn_data.clone(),
            AnyMerkleProof::Blake2bPath(MerklePath::empty()),
            0,
            signer,
        );
        // The signature itself is valid, and so is everything else about the release.
        assert_eq!(
            tx.verify(NetworkId::UnitAlbatross, Policy::max_supported_version()),
            Ok(()),
        );

        let mut bridge = bridge.clone();
        assert_eq!(
            test.test_commit_outgoing_transaction(
                &mut bridge,
                &tx,
                &bs,
                &mut TransactionLog::empty(),
                false,
            ),
            Err(AccountError::InvalidSignature),
            "a release signed by {who} must be rejected",
        );
        let receipts = commit_block(&test, &[tx], &bs);
        assert!(
            matches!(receipts.transactions[0], OperationReceipt::Err(_, _)),
            "a release signed by {who} must not apply: {receipts:?}",
        );
    }
    assert_eq!(
        test.get_complete(&bridge_addr(), None).balance(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
    );
    assert_eq!(
        test.get_complete(&nimiq_target(), None).balance(),
        Coin::ZERO
    );

    // The target's nonce was not consumed: the target's own release still takes nonce 1.
    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &target_key(),
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(receipts.transactions[0], OperationReceipt::Ok(_)));
    assert_eq!(
        test.get_complete(&nimiq_target(), None).balance(),
        Coin::from_u64_unchecked(RELEASE_AMOUNT),
    );
}

/// Consent belongs to the target, submission does not. The target signs a release, fee
/// included, and hands the bytes to a relayer, which submits them unchanged. The relayer pays
/// nothing, and the target needs no NIM beforehand, since the fee comes out of the burned amount.
#[test]
fn a_target_signed_release_submitted_by_a_third_party_is_accepted() {
    const FEE: u64 = 10;
    let owner = KeyPair::generate_default_csprng();
    let relayer = Address::from(&KeyPair::generate_default_csprng().public);
    let relayer_funds = Coin::from_u64_unchecked(1_000);
    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let test = env_with_attested_burn(
        &burn_data,
        blake2b_bridge(&owner),
        &[(
            relayer.clone(),
            Account::Basic(BasicAccount {
                balance: relayer_funds,
            }),
        )],
    );
    let bs = BlockState::new(1, 1, Policy::max_supported_version());

    // The target signs the release and hands over its bytes.
    let signed = make_outgoing_tx_with_fee(
        RELEASE_AMOUNT - FEE,
        FEE,
        burn_data,
        AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        0,
        &target_key(),
    )
    .serialize_to_vec();

    // The relayer submits them as they are.
    let tx = Transaction::deserialize_from_vec(&signed).expect("the release must decode");
    assert_eq!(
        tx.verify(NetworkId::UnitAlbatross, Policy::max_supported_version()),
        Ok(()),
    );
    let receipts = test
        .commit_and_test(&[tx], &[], &bs, &mut BlockLogger::empty())
        .expect("a target-signed release submitted by a relayer commits");
    assert!(matches!(receipts.transactions[0], OperationReceipt::Ok(_)));

    assert_eq!(
        test.get_complete(&bridge_addr(), None).balance(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT - RELEASE_AMOUNT),
    );
    assert_eq!(
        test.get_complete(&nimiq_target(), None).balance(),
        Coin::from_u64_unchecked(RELEASE_AMOUNT - FEE),
    );
    assert_eq!(test.get_complete(&relayer, None).balance(), relayer_funds);
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
    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);

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

    let tx = make_outgoing_tx_full(
        RELEASE_AMOUNT,
        burn_data,
        empty_path_for(kind),
        0,
        &target_key(),
    );
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
    let burn_data = make_burn_data(&nimiq_target(), RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);

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
        &target_key(),
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Err(_, _)),
        "mismatched proof variant must be rejected"
    );
}
