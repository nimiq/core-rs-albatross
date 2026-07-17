use nimiq_account::{
    Account, BasicAccount, BlockLogger, BlockState, BridgeContract, OperationReceipt,
    OracleContract, Receipts, RevertInfo, TransactionLog,
};
use nimiq_database::traits::{Database, WriteTransaction};
use nimiq_hash::{Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::accounts_revert::TestCommitRevert;
use nimiq_transaction::{
    account::{
        bridge_contract::OutgoingBridgeTransactionData,
        htlc_contract::{AnyHash, AnyHash32},
        oracle_contract::{
            CreationTransactionData as OracleCreationData, IncomingOracleTransactionData,
        },
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, CreationTransactionData as BridgeCreationData,
        Endianness, OutgoingTransaction, ValidationOp, ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::{key_rng::SecureGenerate, merkle::MerklePath};

// =====================================================================
// Shared constants and helpers
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

fn chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: SOURCE_CHAIN_ID,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: standard_program(),
        max_proof_depth: 64,
    }
}

/// ValidationProgram that reads all required fields from burn_data:
///   [0..20]  target_address  [20..28] amount  [28..36] nonce
///   [36..40] burn_block_height  [40..44] target_chain_id
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

fn make_burn_data(target: [u8; 20], amount: u64, nonce: u64, chain_id: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity(44);
    d.extend_from_slice(&target);
    d.extend_from_slice(&amount.to_le_bytes());
    d.extend_from_slice(&nonce.to_le_bytes());
    d.extend_from_slice(&BURN_BLOCK_HEIGHT.to_le_bytes());
    d.extend_from_slice(&chain_id.to_le_bytes());
    d
}

fn blake2b(data: &[u8]) -> AnyHash {
    let h = Blake2bHasher::default().digest(data);
    AnyHash::Blake2b(AnyHash32::from(<[u8; 32]>::try_from(h.as_bytes()).unwrap()))
}

/// Build an oracle whose first state-hash is derived from `burn_data`.
/// oracle.hashes[0] = zero.digest(Blake2b(burn_data))
fn make_single_state_oracle(burn_data: &[u8]) -> OracleContract {
    let leaf = blake2b(burn_data);
    let zero = leaf.zero_of_same_type();
    let hash_0 = zero.digest(&leaf);
    let hash_count: u16 = 10;
    let mut hashes = vec![zero; hash_count as usize];
    hashes[0] = hash_0;
    OracleContract {
        owner: Address::from([0x01u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count,
        hashes,
        latest_index: Some(0),
    }
}

/// Build a signed outgoing bridge transaction.
fn make_outgoing_tx(
    bridge: &Address,
    target: &Address,
    amount: u64,
    burn_data: Vec<u8>,
    oracle_state_index: u64,
    owner: &KeyPair,
) -> Transaction {
    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index,
    };
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof: outgoing,
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        bridge.clone(),
        AccountType::Bridge,
        bridge_data.serialize_to_vec(),
        target.clone(),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(amount),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );
    let sig = owner.sign(&tx.serialize_content());
    bridge_data.set_signature(SignatureProof::from_ed25519(owner.public.clone(), sig));
    tx.sender_data = bridge_data.serialize_to_vec();
    tx
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

fn revert_block(test: &TestCommitRevert, txs: &[Transaction], r: Receipts, bs: &BlockState) {
    let env = test.env();
    let mut raw = env.write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut raw).into();
    test.revert(
        &mut txn,
        txs,
        &[],
        bs,
        RevertInfo::Receipts(r),
        &mut BlockLogger::empty_reverted(),
    )
    .unwrap();
    raw.commit();
}

// =====================================================================
// Bridge: create_new_contract
// =====================================================================

/// Success path: oracle present in trie with matching hash type.
#[test]
fn bridge_create_success_with_matching_oracle() {
    let owner = KeyPair::generate_default_csprng();
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data);

    let test = TestCommitRevert::with_initial_state(&[(oracle_addr(), Account::Oracle(oracle))]);

    let creation_data = BridgeCreationData {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
    };
    let tx = Transaction::new_contract_creation(
        Address::from(&owner.public),
        AccountType::Basic,
        vec![],
        AccountType::Bridge,
        creation_data.serialize_to_vec(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    let mut logger = TransactionLog::empty();
    let account = test
        .test_create_new_contract::<BridgeContract>(
            &tx,
            Coin::ZERO,
            &BlockState::new(1, 1, Policy::max_supported_version()),
            &mut logger,
            false,
        )
        .expect("creation should succeed");

    let bridge = match account {
        Account::Bridge(b) => b,
        _ => panic!("wrong account type"),
    };
    assert_eq!(bridge.owner, Address::from(&owner.public));
    assert_eq!(bridge.oracle_address, oracle_addr());
    assert_eq!(bridge.source_chain_id, SOURCE_CHAIN_ID);
    assert_eq!(bridge.balance, Coin::from_u64_unchecked(BRIDGE_DEPOSIT));
    assert_eq!(bridge.transaction_count, 0);
}

/// Oracle address doesn't exist in trie → creation rejected.
#[test]
fn bridge_create_rejects_when_oracle_missing() {
    let owner = KeyPair::generate_default_csprng();

    // No oracle in initial state
    let test = TestCommitRevert::with_initial_state(&[]);

    let creation_data = BridgeCreationData {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
    };
    let tx = Transaction::new_contract_creation(
        Address::from(&owner.public),
        AccountType::Basic,
        vec![],
        AccountType::Bridge,
        creation_data.serialize_to_vec(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    let result = test.test_create_new_contract::<BridgeContract>(
        &tx,
        Coin::ZERO,
        &BlockState::new(1, 1, Policy::max_supported_version()),
        &mut TransactionLog::empty(),
        false,
    );
    assert!(result.is_err(), "creation must fail when oracle is absent");
}

/// Oracle address points to a basic account, not an oracle contract → rejected.
#[test]
fn bridge_create_rejects_when_oracle_addr_is_not_oracle_contract() {
    let owner = KeyPair::generate_default_csprng();

    // Put a basic account at the oracle address
    let test = TestCommitRevert::with_initial_state(&[(
        oracle_addr(),
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1_000),
        }),
    )]);

    let creation_data = BridgeCreationData {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
    };
    let tx = Transaction::new_contract_creation(
        Address::from(&owner.public),
        AccountType::Basic,
        vec![],
        AccountType::Bridge,
        creation_data.serialize_to_vec(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    let result = test.test_create_new_contract::<BridgeContract>(
        &tx,
        Coin::ZERO,
        &BlockState::new(1, 1, Policy::max_supported_version()),
        &mut TransactionLog::empty(),
        false,
    );
    assert!(
        result.is_err(),
        "creation must fail when oracle addr is a basic account"
    );
}

/// Oracle exists but uses a different hash type than the bridge ChainConfig → rejected.
#[test]
fn bridge_create_rejects_hash_type_mismatch_with_oracle() {
    let owner = KeyPair::generate_default_csprng();

    // Oracle has a Keccak256 hash stored
    use nimiq_hash::Keccak256Hasher;
    let keccak_hash = {
        let h = Keccak256Hasher::default().digest(b"state");
        AnyHash::Keccak256(AnyHash32::from(<[u8; 32]>::try_from(h.as_bytes()).unwrap()))
    };
    let zero = keccak_hash.zero_of_same_type();
    let hash_count: u16 = 5;
    let mut hashes = vec![zero.clone(); hash_count as usize];
    hashes[0] = zero.digest(&keccak_hash);
    let oracle = OracleContract {
        owner: Address::from([0x01u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count,
        hashes,
        latest_index: Some(0),
    };

    let test = TestCommitRevert::with_initial_state(&[(oracle_addr(), Account::Oracle(oracle))]);

    // Bridge ChainConfig uses Blake2b — mismatch with oracle's Keccak256
    let creation_data = BridgeCreationData {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(), // Blake2b
    };
    let tx = Transaction::new_contract_creation(
        Address::from(&owner.public),
        AccountType::Basic,
        vec![],
        AccountType::Bridge,
        creation_data.serialize_to_vec(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    let result = test.test_create_new_contract::<BridgeContract>(
        &tx,
        Coin::ZERO,
        &BlockState::new(1, 1, Policy::max_supported_version()),
        &mut TransactionLog::empty(),
        false,
    );
    assert!(result.is_err(), "creation must fail when hash types differ");
}

/// Oracle exists but has no hashes yet → any hash type accepted.
#[test]
fn bridge_create_accepts_empty_oracle_any_hash_type() {
    let owner = KeyPair::generate_default_csprng();

    // Oracle with no hashes yet
    let oracle = OracleContract {
        owner: Address::from([0x01u8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count: 5,
        hashes: Vec::new(),
        latest_index: None,
    };

    let test = TestCommitRevert::with_initial_state(&[(oracle_addr(), Account::Oracle(oracle))]);

    let creation_data = BridgeCreationData {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
    };
    let tx = Transaction::new_contract_creation(
        Address::from(&owner.public),
        AccountType::Basic,
        vec![],
        AccountType::Bridge,
        creation_data.serialize_to_vec(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    let result = test.test_create_new_contract::<BridgeContract>(
        &tx,
        Coin::ZERO,
        &BlockState::new(1, 1, Policy::max_supported_version()),
        &mut TransactionLog::empty(),
        false,
    );
    assert!(
        result.is_ok(),
        "creation must succeed when oracle has no hashes yet"
    );
}

// =====================================================================
// Bridge: commit/revert incoming transaction
// =====================================================================

fn make_bridge_state(owner: &KeyPair) -> (TestCommitRevert, OracleContract) {
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data);
    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 0,
    };
    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle.clone())),
        (bridge_addr(), Account::Bridge(bridge)),
    ]);
    (test, oracle)
}

/// commit_incoming_transaction increments balance and transaction_count; revert restores both.
#[test]
fn bridge_incoming_commit_revert_roundtrip() {
    let owner = KeyPair::generate_default_csprng();
    let sender_addr = Address::from(&owner.public);
    let deposit = Coin::from_u64_unchecked(300);

    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data);
    let bridge = BridgeContract {
        owner: sender_addr.clone(),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 0,
    };
    // Sender must have enough balance to cover the deposit
    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
        (
            sender_addr.clone(),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(10_000),
            }),
        ),
    ]);

    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let tx = Transaction::new_extended(
        sender_addr,
        AccountType::Basic,
        vec![],
        bridge_addr(),
        AccountType::Bridge,
        vec![],
        deposit,
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    // commit_and_test commits, verifies revert is clean, and keeps the commit
    let receipts = test
        .commit_and_test(&[tx], &[], &bs, &mut BlockLogger::empty())
        .expect("incoming commit should succeed");

    assert!(
        matches!(receipts.transactions[0], OperationReceipt::Ok(_)),
        "incoming transaction must succeed"
    );
}

// =====================================================================
// Bridge: commit_outgoing_transaction rejection paths
// =====================================================================

fn setup_outgoing_env() -> (TestCommitRevert, KeyPair, Vec<u8>) {
    let owner = KeyPair::generate_default_csprng();
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data);
    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
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
    (test, owner, burn_data)
}

/// Happy path: valid outgoing tx is committed and balance decremented.
#[test]
fn bridge_outgoing_commit_success() {
    let (test, owner, burn_data) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data,
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(receipts.transactions[0], OperationReceipt::Ok(_)));
}

/// A *successful* outgoing bridge transaction with a non-zero fee must debit the
/// fee from the burn-proof signer, not mint it. The bridge releases only `value`
/// and the fee is still paid into the reward pot, so without a signer debit the
/// total supply would inflate by exactly the fee. `commit_and_test` also verifies
/// the commit/revert round-trip (state root + logs), covering the fee refund.
#[test]
fn bridge_outgoing_success_charges_fee_to_signer() {
    let owner = KeyPair::generate_default_csprng();
    let signer_address = Address::from(&owner.public);
    let initial_signer_balance = Coin::from_u64_unchecked(10_000);

    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data);
    let bridge = BridgeContract {
        owner: signer_address.clone(),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 0,
    };
    // The target is intentionally not seeded: a zero-balance account is never
    // stored in the trie (it is created by the release and pruned again on
    // revert), so seeding it would make the revert root-hash check spuriously
    // fail against the pruned post-revert trie.
    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
        (
            signer_address.clone(),
            Account::Basic(BasicAccount {
                balance: initial_signer_balance,
            }),
        ),
    ]);

    // Build a signed outgoing bridge transaction with a non-zero fee. The proof
    // in `sender_data` is signed by `owner`, so the fee payer is the owner.
    let fee = Coin::from_u64_unchecked(10);
    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index: 0,
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
        Coin::from_u64_unchecked(RELEASE_AMOUNT),
        fee,
        1,
        NetworkId::UnitAlbatross,
    );
    let sig = owner.sign(&tx.serialize_content());
    bridge_data.set_signature(SignatureProof::from_ed25519(owner.public.clone(), sig));
    tx.sender_data = bridge_data.serialize_to_vec();

    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let receipts = test
        .commit_and_test(&[tx], &[], &bs, &mut BlockLogger::empty())
        .expect("successful outgoing commit");

    // The fee payer is recorded as the burn-proof signer (so revert can refund it).
    match &receipts.transactions[0] {
        OperationReceipt::Ok(receipt) => assert_eq!(
            receipt.fee_payer,
            Some(signer_address.clone()),
            "fee payer must be the burn-proof signer",
        ),
        other => panic!("expected a successful receipt, got {other:?}"),
    }

    // Value leaves the bridge, the fee leaves the signer, the target gets the value.
    assert_eq!(
        test.get_complete(&bridge_addr(), None).balance(),
        Coin::from_u64_unchecked(BRIDGE_DEPOSIT - RELEASE_AMOUNT),
        "bridge debits exactly the released value",
    );
    assert_eq!(
        test.get_complete(&signer_address, None).balance(),
        initial_signer_balance - fee,
        "fee is debited from the signer (no supply inflation)",
    );
    assert_eq!(
        test.get_complete(&nimiq_target(), None).balance(),
        Coin::from_u64_unchecked(RELEASE_AMOUNT),
        "target receives the released value",
    );
}

/// Wrong chain_id in burn data → rejected.
#[test]
fn bridge_outgoing_rejects_wrong_chain_id() {
    let (test, owner, _) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let wrong_chain_id = SOURCE_CHAIN_ID + 1;
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, wrong_chain_id);
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data,
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

/// Transaction recipient doesn't match burn data target address → rejected.
#[test]
fn bridge_outgoing_rejects_wrong_recipient() {
    let (test, owner, burn_data) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let wrong_recipient = Address::from([0xBBu8; 20]);
    // burn_data has [0xAA; 20] as target, but tx recipient is [0xBB; 20]
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &wrong_recipient,
        RELEASE_AMOUNT,
        burn_data,
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

/// Transaction value doesn't match burn data amount → rejected.
#[test]
fn bridge_outgoing_rejects_wrong_value() {
    let (test, owner, burn_data) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let wrong_amount = RELEASE_AMOUNT + 1;
    // burn_data says RELEASE_AMOUNT, but tx sends wrong_amount
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        wrong_amount,
        burn_data,
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

/// Nonce gap: nonce=2 submitted when no prior nonce exists (expected nonce=1) → rejected.
#[test]
fn bridge_outgoing_rejects_nonce_gap() {
    let owner = KeyPair::generate_default_csprng();
    // Build oracle for nonce=2 burn data
    let burn_data_2 = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 2, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data_2);
    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 0,
    };
    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
    ]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    // No nonce=1 has been processed; submitting nonce=2 directly is a gap
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data_2,
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

/// Same nonce submitted twice → second submission rejected.
#[test]
fn bridge_outgoing_rejects_duplicate_nonce() {
    let (test, owner, burn_data) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data,
        0,
        &owner,
    );
    // First submission succeeds
    let r1 = commit_block(&test, &[tx.clone()], &bs);
    assert!(matches!(r1.transactions[0], OperationReceipt::Ok(_)));
    // Second identical submission must fail (nonce=1 already used)
    let r2 = commit_block(&test, &[tx], &bs);
    assert!(matches!(r2.transactions[0], OperationReceipt::Err(_, _)));
}

/// Oracle state index out of bounds → rejected.
#[test]
fn bridge_outgoing_rejects_oracle_index_out_of_bounds() {
    let (test, owner, burn_data) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    // Oracle has only index 0; submitting index 5 is out of bounds
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data,
        5,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

/// Merkle proof doesn't match oracle state hash → rejected.
#[test]
fn bridge_outgoing_rejects_wrong_merkle_proof() {
    let (test, owner, _) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    // Use burn_data for a completely different transaction — leaf hash won't match oracle
    let wrong_burn_data = make_burn_data([0xFFu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &Address::from([0xFFu8; 20]),
        RELEASE_AMOUNT,
        wrong_burn_data,
        0,
        &owner,
    );
    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

/// Proof depth exceeds max_proof_depth → rejected.
#[test]
fn bridge_outgoing_rejects_proof_depth_exceeded() {
    let owner = KeyPair::generate_default_csprng();
    let burn_data = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data);

    // Bridge with a very tight max_proof_depth of 1
    let mut cfg = chain_config();
    cfg.max_proof_depth = 1;
    let bridge = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::from_u64_unchecked(BRIDGE_DEPOSIT),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: cfg,
        transaction_count: 0,
    };
    let test = TestCommitRevert::with_initial_state(&[
        (oracle_addr(), Account::Oracle(oracle)),
        (bridge_addr(), Account::Bridge(bridge)),
    ]);
    let bs = BlockState::new(1, 1, Policy::max_supported_version());

    // Build a proof with 2 sibling nodes — exceeds max_proof_depth=1
    use nimiq_hash::Blake2bHash;
    use nimiq_utils::merkle::MerkleProof;
    let dummy = Blake2bHasher::default().digest(b"a");
    let proof =
        MerkleProof::<Blake2bHash>::new(&[dummy.clone(), dummy.clone(), dummy.clone()], &[dummy]);
    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        merkle_proof: AnyMerkleProof::Blake2b(proof),
        oracle_state_index: 0,
    };
    let mut bd = OutgoingBridgeTransactionData {
        burn_proof: outgoing,
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_extended(
        bridge_addr(),
        AccountType::Bridge,
        bd.serialize_to_vec(),
        nimiq_target(),
        AccountType::Basic,
        vec![],
        Coin::from_u64_unchecked(RELEASE_AMOUNT),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );
    let sig = owner.sign(&tx.serialize_content());
    bd.set_signature(SignatureProof::from_ed25519(owner.public.clone(), sig));
    tx.sender_data = bd.serialize_to_vec();

    let receipts = commit_block(&test, &[tx], &bs);
    assert!(matches!(
        receipts.transactions[0],
        OperationReceipt::Err(_, _)
    ));
}

// =====================================================================
// Bridge: revert_outgoing_transaction — nonce=1 direct round-trip
// =====================================================================

/// commit(nonce=1) then revert(nonce=1): nonce entry must be fully removed.
/// Verifies the `receipt.nonce == 1 → remove_nonce` branch of the fix.
#[test]
fn bridge_outgoing_revert_nonce1_removes_entry() {
    let (test, owner, burn_data) = setup_outgoing_env();
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let tx = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data,
        0,
        &owner,
    );

    // Commit nonce=1 then revert it
    let receipts = commit_block(&test, &[tx.clone()], &bs);
    assert!(matches!(receipts.transactions[0], OperationReceipt::Ok(_)));
    revert_block(&test, &[tx.clone()], receipts, &bs);

    // After revert, nonce=1 must be accepted again (entry was cleanly removed)
    let burn_data_again = make_burn_data([0xAAu8; 20], RELEASE_AMOUNT, 1, SOURCE_CHAIN_ID);
    let oracle = make_single_state_oracle(&burn_data_again);
    // Re-build oracle state (same as initial since we're re-using same burn data)
    // Re-submit nonce=1 — should succeed because the revert cleaned up correctly
    let tx2 = make_outgoing_tx(
        &bridge_addr(),
        &nimiq_target(),
        RELEASE_AMOUNT,
        burn_data_again,
        0,
        &owner,
    );
    let r2 = commit_block(&test, &[tx2], &bs);
    assert!(
        matches!(r2.transactions[0], OperationReceipt::Ok(_)),
        "nonce=1 should be accepted again after clean revert"
    );
    let _ = oracle; // suppress unused warning
}

// =====================================================================
// Bridge: prune / restore round-trip
// =====================================================================

/// Prune/restore invariant: restored bridge has same fields as original (balance=0).
#[test]
fn bridge_prune_restore_roundtrip_fields() {
    use nimiq_account::{AccountPruningInteraction, AccountReceipt};

    let owner = KeyPair::generate_default_csprng();
    let original = BridgeContract {
        owner: Address::from(&owner.public),
        oracle_address: oracle_addr(),
        balance: Coin::ZERO,
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config(),
        transaction_count: 7,
    };

    assert!(original.can_be_pruned());

    let test = TestCommitRevert::with_initial_state(&[]);
    let env = test.env();

    // Prune
    let read_txn = env.read_transaction();
    let ds = test.data_store(&bridge_addr());
    let receipt: Option<AccountReceipt> = original.clone().prune(ds.read(&read_txn));
    assert!(receipt.is_some());

    // Restore in a write txn
    let mut raw = env.write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut raw).into();
    let restored =
        BridgeContract::restore(AccountType::Bridge, receipt.as_ref(), ds.write(&mut txn))
            .expect("restore must succeed");
    drop(txn);
    raw.commit();

    let bridge = match restored {
        Account::Bridge(b) => b,
        _ => panic!("wrong account type after restore"),
    };
    assert_eq!(bridge.owner, original.owner);
    assert_eq!(bridge.oracle_address, original.oracle_address);
    assert_eq!(bridge.source_chain_id, original.source_chain_id);
    assert_eq!(bridge.transaction_count, original.transaction_count);
    assert_eq!(bridge.balance, Coin::ZERO);

    // Non-zero balance bridge must not be prunable
    let rich = BridgeContract {
        balance: Coin::from_u64_unchecked(1),
        ..original.clone()
    };
    assert!(!rich.can_be_pruned());
}

// =====================================================================
// Oracle: revert_new_contract
// =====================================================================

/// revert_new_contract must undo the deposit.
#[test]
fn oracle_revert_new_contract_removes_deposit() {
    let owner = KeyPair::generate_default_csprng();

    let test = TestCommitRevert::with_initial_state(&[(
        Address::from(&owner.public),
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(10_000),
        }),
    )]);

    let creation_data = OracleCreationData {
        owner: Address::from(&owner.public),
        hash_count: 5,
    };
    let tx = Transaction::new_contract_creation(
        Address::from(&owner.public),
        AccountType::Basic,
        vec![],
        AccountType::Oracle,
        creation_data.serialize_to_vec(),
        Coin::from_u64_unchecked(1_000),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );

    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let mut logger = TransactionLog::empty();

    // keep_commit=false → commit then immediately revert and assert state is restored
    let result =
        test.test_create_new_contract::<OracleContract>(&tx, Coin::ZERO, &bs, &mut logger, false);
    // If revert didn't work, the trie root check inside test_create_new_contract would panic.
    assert!(result.is_ok(), "create+revert must both succeed");
}

// =====================================================================
// Oracle: successful ChangeOwner revert restores old owner
// =====================================================================

fn oracle_addr_for_test() -> Address {
    Address::from([0x01u8; 20])
}

fn make_oracle_with_owner(owner: &KeyPair) -> OracleContract {
    OracleContract {
        owner: Address::from(&owner.public),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count: 10,
        hashes: Vec::new(),
        latest_index: None,
    }
}

fn make_change_owner_tx(
    contract_address: Address,
    key: &KeyPair,
    new_owner: Address,
) -> Transaction {
    let data = IncomingOracleTransactionData::ChangeOwner {
        new_owner,
        proof: SignatureProof::default(),
    };
    let mut tx = Transaction::new_signaling(
        contract_address.clone(),
        AccountType::Oracle,
        contract_address,
        AccountType::Oracle,
        Coin::ZERO,
        data.serialize_to_vec(),
        0,
        NetworkId::UnitAlbatross,
    );
    let sig = key.sign(&tx.serialize_content());
    let proof = SignatureProof::from_ed25519(key.public, sig);
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, proof).unwrap();
    tx
}

/// ChangeOwner commit sets new owner; revert restores old owner.
///
/// `test_commit_incoming_transaction(keep_commit=false)` commits, internally
/// asserts the revert brings the account back to `initial_account`, then
/// returns leaving `oracle_mut` in the initial (pre-commit) state.
/// If no panic occurs the revert is proven correct.
#[test]
fn oracle_change_owner_revert_restores_old_owner() {
    let old_owner = KeyPair::generate_default_csprng();
    let new_owner_kp = KeyPair::generate_default_csprng();
    let new_owner_addr = Address::from(&new_owner_kp.public);
    let old_owner_addr = Address::from(&old_owner.public);

    let oracle = make_oracle_with_owner(&old_owner);
    let contract_addr = oracle_addr_for_test();

    let test = TestCommitRevert::with_initial_state(&[(
        contract_addr.clone(),
        Account::Oracle(oracle.clone()),
    )]);

    let tx = make_change_owner_tx(contract_addr.clone(), &old_owner, new_owner_addr.clone());
    let bs = BlockState::new(1, 1, Policy::max_supported_version());
    let mut logger = TransactionLog::empty();
    let mut oracle_mut = oracle.clone();

    // keep_commit=false: the framework internally checks that after the revert
    // oracle_mut == initial state (old owner). A panic here means revert is broken.
    test.test_commit_incoming_transaction(&mut oracle_mut, &tx, &bs, &mut logger, false)
        .expect("ChangeOwner commit+revert must succeed without panic");

    // After keep_commit=false the framework leaves oracle_mut in the initial state.
    assert_eq!(
        oracle_mut.owner, old_owner_addr,
        "after revert, owner must be the original owner"
    );
}

// =====================================================================
// ValidationProgram: missing opcode coverage
// =====================================================================

fn run_program(
    ops: Vec<ValidationOp>,
    burn_data: &[u8],
) -> Vec<(String, nimiq_transaction::bridge_contract::StackValue)> {
    use nimiq_transaction::bridge_contract::ValidationProgram;
    let prog = ValidationProgram::new(ops);
    let result = prog
        .extract_only(burn_data)
        .expect("program should succeed");
    result.extracted_values.into_iter().collect()
}

/// LoadBytes: load a slice from burn_data.
#[test]
fn validation_op_load_bytes() {
    let data: Vec<u8> = (0u8..32).collect();
    // stack before LoadBytes: [offset=2, length=4] (offset pushed first, length pushed last → length is on top)
    let ops = vec![
        ValidationOp::PushConst(2), // offset
        ValidationOp::PushConst(4), // length (top)
        ValidationOp::LoadBytes,
        ValidationOp::Store("slice".to_string()),
    ];
    let result = run_program(ops, &data);
    let val = result
        .iter()
        .find(|(k, _)| k == "slice")
        .map(|(_, v)| v)
        .unwrap();
    assert_eq!(val.as_bytes().unwrap(), vec![2u8, 3, 4, 5]);
}

/// LoadU32: reads a 4-byte little-endian u32.
#[test]
fn validation_op_load_u32() {
    let value: u32 = 0x01020304;
    let mut data = vec![0u8; 8];
    data[2..6].copy_from_slice(&value.to_le_bytes());
    let ops = vec![
        ValidationOp::PushConst(2),
        ValidationOp::LoadU32(Endianness::LittleEndian),
        ValidationOp::Store("val".to_string()),
    ];
    let result = run_program(ops, &data);
    let v = result
        .iter()
        .find(|(k, _)| k == "val")
        .unwrap()
        .1
        .as_u64()
        .unwrap();
    assert_eq!(v, value as u64);
}

/// Mod: remainder operation.
#[test]
fn validation_op_mod() {
    let data = vec![0u8; 4];
    let ops = vec![
        ValidationOp::PushConst(10),
        ValidationOp::PushConst(3),
        ValidationOp::Mod,
        ValidationOp::Store("r".to_string()),
    ];
    let result = run_program(ops, &data);
    assert_eq!(
        result
            .iter()
            .find(|(k, _)| k == "r")
            .unwrap()
            .1
            .as_u64()
            .unwrap(),
        1
    );
}

/// IsZero: 0 → 1, non-zero → 0.
#[test]
fn validation_op_is_zero() {
    let data = vec![0u8; 4];
    let ops_zero = vec![
        ValidationOp::PushConst(0),
        ValidationOp::IsZero,
        ValidationOp::Store("z".to_string()),
    ];
    let ops_nonzero = vec![
        ValidationOp::PushConst(5),
        ValidationOp::IsZero,
        ValidationOp::Store("z".to_string()),
    ];
    let r1 = run_program(ops_zero, &data);
    let r2 = run_program(ops_nonzero, &data);
    assert_eq!(r1[0].1.as_u64().unwrap(), 1);
    assert_eq!(r2[0].1.as_u64().unwrap(), 0);
}

/// And / Or / Not logical operations.
#[test]
fn validation_op_logical() {
    let data = vec![0u8; 4];
    // AND: 1 && 0 → 0
    let and_ops = vec![
        ValidationOp::PushConst(1),
        ValidationOp::PushConst(0),
        ValidationOp::And,
        ValidationOp::Store("r".to_string()),
    ];
    // OR: 0 || 1 → 1
    let or_ops = vec![
        ValidationOp::PushConst(0),
        ValidationOp::PushConst(1),
        ValidationOp::Or,
        ValidationOp::Store("r".to_string()),
    ];
    // NOT: !0 → 1
    let not_ops = vec![
        ValidationOp::PushConst(0),
        ValidationOp::Not,
        ValidationOp::Store("r".to_string()),
    ];
    assert_eq!(run_program(and_ops, &data)[0].1.as_u64().unwrap(), 0);
    assert_eq!(run_program(or_ops, &data)[0].1.as_u64().unwrap(), 1);
    assert_eq!(run_program(not_ops, &data)[0].1.as_u64().unwrap(), 1);
}

/// Pop discards the top stack value.
#[test]
fn validation_op_pop() {
    let data = vec![0u8; 4];
    let ops = vec![
        ValidationOp::PushConst(99),
        ValidationOp::PushConst(42),
        ValidationOp::Pop,                      // remove 42
        ValidationOp::Store("top".to_string()), // should store 99
    ];
    let result = run_program(ops, &data);
    assert_eq!(result[0].1.as_u64().unwrap(), 99);
}

/// JumpIfZero: skips N operations when condition is zero.
#[test]
fn validation_op_jump_if_zero_taken() {
    let data = vec![0u8; 4];
    // Condition=0 → skip next 2 ops → Store(99) is never reached → Store(1) is
    let ops = vec![
        ValidationOp::PushConst(0),           // condition = 0
        ValidationOp::JumpIfZero(2),          // skip 2 ops if zero
        ValidationOp::PushConst(99),          // skipped
        ValidationOp::Store("v".to_string()), // skipped
        ValidationOp::PushConst(1),
        ValidationOp::Store("v".to_string()),
    ];
    let result = run_program(ops, &data);
    assert_eq!(
        result[0].1.as_u64().unwrap(),
        1,
        "jump should have been taken"
    );
}

#[test]
fn validation_op_jump_if_zero_not_taken() {
    let data = vec![0u8; 4];
    // Condition=1 → do NOT skip → Store(99) runs
    let ops = vec![
        ValidationOp::PushConst(1),  // condition = 1 (non-zero)
        ValidationOp::JumpIfZero(2), // NOT taken
        ValidationOp::PushConst(99),
        ValidationOp::Store("v".to_string()),
        ValidationOp::PushConst(0), // never stored (would overwrite)
        ValidationOp::Pop,
    ];
    let result = run_program(ops, &data);
    assert_eq!(
        result[0].1.as_u64().unwrap(),
        99,
        "jump should not have been taken"
    );
}

/// Store / Load named variable round-trip.
#[test]
fn validation_op_store_load_roundtrip() {
    let data = vec![0u8; 4];
    let ops = vec![
        ValidationOp::PushConst(77),
        ValidationOp::Store("x".to_string()),
        ValidationOp::Load("x".to_string()),
        ValidationOp::Store("y".to_string()),
    ];
    let result = run_program(ops, &data);
    // x should still be 77 and y should also be 77
    let x = result
        .iter()
        .find(|(k, _)| k == "x")
        .unwrap()
        .1
        .as_u64()
        .unwrap();
    let y = result
        .iter()
        .find(|(k, _)| k == "y")
        .unwrap()
        .1
        .as_u64()
        .unwrap();
    assert_eq!(x, 77);
    assert_eq!(y, 77);
}

/// extract_only errors on all four PushExpected* variants.
#[test]
fn extract_only_rejects_push_expected_variants() {
    use nimiq_transaction::bridge_contract::ValidationProgram;
    let data = vec![0u8; 4];
    for op in [
        ValidationOp::PushExpectedAmount,
        ValidationOp::PushExpectedAddress,
        ValidationOp::PushExpectedNonce,
        ValidationOp::PushExpectedValidityHeight,
    ] {
        let prog = ValidationProgram::new(vec![op.clone()]);
        assert!(
            prog.extract_only(&data).is_err(),
            "extract_only must reject {:?}",
            op
        );
    }
}
