use nimiq_account::{
    Account, BlockLogger, BlockState, BridgeContract, OperationReceipt, OracleContract, Receipts,
    RevertInfo,
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
    },
    bridge_contract::{
        AddressFormat, AnyMerkleProof, ChainConfig, CreationTransactionData, Endianness,
        OutgoingTransaction, ValidationOp, ValidationProgram,
    },
    SignatureProof, Transaction,
};
use nimiq_utils::{key_rng::SecureGenerate, merkle::MerklePath};

// =====================================================================
// Helpers
// =====================================================================

const SOURCE_CHAIN_ID: u32 = 1;
const RELEASE_AMOUNT: u64 = 100;
const BURN_BLOCK_HEIGHT: u32 = 42;

fn target_address() -> Address {
    Address::from([5u8; 20])
}

fn target_address_bytes() -> [u8; 20] {
    [5u8; 20]
}

/// Returns a ValidationProgram that reads all required fields from burn_data:
///   [0..20]  target_address
///   [20..28] amount (u64 LE)
///   [28..36] target_nonce (u64 LE)
///   [36..40] burn_block_height (u32 LE)
///   [40..44] target_chain_id (u32 LE)
fn standard_validation_program() -> ValidationProgram {
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

fn make_chain_config(program: ValidationProgram) -> ChainConfig {
    ChainConfig {
        chain_id: SOURCE_CHAIN_ID,
        hash_function: AnyHash::Blake2b(AnyHash32::default()),
        address_format: AddressFormat::Nimiq,
        endianness: Endianness::LittleEndian,
        block_time: std::time::Duration::from_secs(60),
        validation_program: program,
        max_proof_depth: 64,
    }
}

/// Encode burn_data for a given nonce.
fn make_burn_data(nonce: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(44);
    data.extend_from_slice(&target_address_bytes());
    data.extend_from_slice(&RELEASE_AMOUNT.to_le_bytes());
    data.extend_from_slice(&nonce.to_le_bytes());
    data.extend_from_slice(&BURN_BLOCK_HEIGHT.to_le_bytes());
    data.extend_from_slice(&SOURCE_CHAIN_ID.to_le_bytes());
    data
}

/// Compute AnyHash::Blake2b over raw bytes.
fn blake2b_hash(data: &[u8]) -> AnyHash {
    let h = Blake2bHasher::default().digest(data);
    AnyHash::Blake2b(AnyHash32::from(<[u8; 32]>::try_from(h.as_bytes()).unwrap()))
}

/// Build the oracle contract with exactly two chained state-hashes so that both
/// nonce=1 and nonce=2 outgoing transactions can be verified.
///
/// oracle.hashes[0] = zero.digest(leaf_hash_1)
/// oracle.hashes[1] = oracle.hashes[0].digest(leaf_hash_2)
fn make_oracle_for_two_txs() -> (OracleContract, AnyHash, AnyHash) {
    let burn_data_1 = make_burn_data(1);
    let burn_data_2 = make_burn_data(2);

    let leaf_hash_1 = blake2b_hash(&burn_data_1);
    let leaf_hash_2 = blake2b_hash(&burn_data_2);

    let zero_hash = leaf_hash_1.zero_of_same_type();
    let oracle_hash_0 = zero_hash.digest(&leaf_hash_1);
    let oracle_hash_1 = oracle_hash_0.digest(&leaf_hash_2);

    let hash_count: u16 = 10;
    let mut hashes = vec![zero_hash; hash_count as usize];
    hashes[0] = oracle_hash_0;
    hashes[1] = oracle_hash_1;

    let oracle = OracleContract {
        owner: Address::from([0x0Eu8; 20]),
        balance: Coin::from_u64_unchecked(1_000),
        hash_count,
        hashes,
        latest_index: Some(1),
    };

    (oracle, leaf_hash_1, leaf_hash_2)
}

/// Construct a signed outgoing bridge transaction.
///
/// `oracle_state_index=0` for nonce=1, `oracle_state_index=1` for nonce=2.
fn make_outgoing_tx(
    bridge_address: &Address,
    nonce: u64,
    oracle_state_index: u64,
    owner_keypair: &KeyPair,
) -> Transaction {
    let burn_data = make_burn_data(nonce);

    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        // Empty MerklePath → compute_root_from_hash returns leaf unchanged, so
        // merkle_root == leaf_hash == Blake2b(burn_data). This is what the oracle stores.
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index,
    };

    // Build bridge_data with default (zero) proof first — this is what gets signed.
    let mut bridge_data = OutgoingBridgeTransactionData {
        burn_proof: outgoing,
        proof: SignatureProof::default(),
    };

    let amount = Coin::from_u64_unchecked(RELEASE_AMOUNT);
    let mut tx = Transaction::new_extended(
        bridge_address.clone(),
        AccountType::Bridge,
        bridge_data.serialize_to_vec(), // sender_data with zeroed proof
        target_address(),
        AccountType::Basic,
        vec![],
        amount,
        Coin::ZERO, // zero fee avoids signer-deduction path
        1,
        NetworkId::UnitAlbatross,
    );

    // Sign over tx content (which already has zeroed proof in sender_data).
    // This matches what verify_transaction_signature does before checking the sig.
    let sig = owner_keypair.sign(&tx.serialize_content());
    let sig_proof = SignatureProof::from_ed25519(owner_keypair.public.clone(), sig);

    // Set the real signature and update sender_data.
    bridge_data.set_signature(sig_proof);
    tx.sender_data = bridge_data.serialize_to_vec();

    tx
}

/// Commit `transactions` to the trie (persisting the result) and return the receipts.
fn commit_block(
    test: &TestCommitRevert,
    transactions: &[Transaction],
    block_state: &BlockState,
) -> Receipts {
    let env = test.env();
    let mut raw_txn = env.write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut raw_txn).into();
    let receipts = test
        .commit(
            &mut txn,
            transactions,
            &[],
            block_state,
            &mut BlockLogger::empty(),
        )
        .expect("commit_block failed");
    raw_txn.commit();
    receipts
}

/// Revert `transactions` from the trie (persisting the revert) using the supplied receipts.
fn revert_block(
    test: &TestCommitRevert,
    transactions: &[Transaction],
    receipts: Receipts,
    block_state: &BlockState,
) {
    let env = test.env();
    let mut raw_txn = env.write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut raw_txn).into();
    test.revert(
        &mut txn,
        transactions,
        &[],
        block_state,
        RevertInfo::Receipts(receipts),
        &mut BlockLogger::empty_reverted(),
    )
    .expect("revert_block failed");
    raw_txn.commit();
}

#[test]
fn test_nonce_revert_opens_replay_attack() {
    let owner_keypair = KeyPair::generate_default_csprng();
    let owner_address = Address::from(&owner_keypair.public);
    let oracle_address = Address::from([0x0Eu8; 20]);
    let bridge_address = Address::from([0x0Bu8; 20]);

    let (oracle, _, _) = make_oracle_for_two_txs();
    let chain_config = make_chain_config(standard_validation_program());

    let bridge = BridgeContract {
        owner: owner_address.clone(),
        oracle_address: oracle_address.clone(),
        balance: Coin::from_u64_unchecked(10_000),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config,
        transaction_count: 0,
    };

    let test = TestCommitRevert::with_initial_state(&[
        (oracle_address.clone(), Account::Oracle(oracle)),
        (bridge_address.clone(), Account::Bridge(bridge)),
    ]);

    let block_state = BlockState::new(1, 1, Policy::max_supported_version());

    let tx1 = make_outgoing_tx(&bridge_address, 1, 0, &owner_keypair);
    let tx2 = make_outgoing_tx(&bridge_address, 2, 1, &owner_keypair);

    // Step 1: commit nonce=1 — must succeed.
    let receipts1 = commit_block(&test, &[tx1.clone()], &block_state);
    assert!(
        matches!(receipts1.transactions[0], OperationReceipt::Ok(_)),
        "tx nonce=1 should be committed successfully"
    );

    // Step 2: commit nonce=2 — must succeed.
    let receipts2 = commit_block(&test, &[tx2.clone()], &block_state);
    assert!(
        matches!(receipts2.transactions[0], OperationReceipt::Ok(_)),
        "tx nonce=2 should be committed successfully"
    );

    // Step 3: revert nonce=2.
    // should restore nonce to 1
    revert_block(&test, &[tx2.clone()], receipts2, &block_state);

    // Step 4: attempt to replay nonce=1.
    // Expected (correct): OperationReceipt::Err — nonce=1 was already processed.
    // Actual (buggy):     OperationReceipt::Ok  — nonce was wiped, replay succeeds.
    let receipts_replay = commit_block(&test, &[tx1.clone()], &block_state);

    assert!(
        matches!(receipts_replay.transactions[0], OperationReceipt::Err(_, _)),
        "REPLAY ATTACK CONFIRMED: nonce=1 was accepted again after reverting nonce=2. \
         The nonce revert logic removes the entry instead of restoring it to the previous value."
    );
}

#[test]
fn test_large_amount_in_burn_data_returns_error() {
    // Amount one unit above Coin::MAX_SAFE_VALUE.
    let oversize_amount = Coin::MAX_SAFE_VALUE + 1; // 9_007_199_254_740_992u64

    let program = standard_validation_program();
    let chain_config = make_chain_config(program);

    let mut burn_data = make_burn_data(1);
    // Overwrite the amount field (bytes 20..28) with the oversize value.
    burn_data[20..28].copy_from_slice(&oversize_amount.to_le_bytes());

    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index: 0,
    };

    // Must return an error and not panic.
    assert!(
        outgoing.parse_burn_data(&chain_config).is_err(),
        "oversized amount must be rejected with an error, not a panic"
    );
}

#[test]
fn test_assert_in_validation_program_permanently_locks_funds() {
    // ValidationProgram with all required stores PLUS a final Assert.
    // A bridge operator might add this for "extra on-chain validation".
    let locked_program = ValidationProgram::new(vec![
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
        // "Sanity check" assertion that looks harmless but breaks extraction-only mode.
        ValidationOp::PushConst(1), // push true
        ValidationOp::Assert,       // assert top of stack is non-zero
    ]);

    let chain_config = make_chain_config(locked_program.clone());

    //  Creation must be rejected when the program is incompatible.
    let creation_data = CreationTransactionData {
        owner: Address::from([1u8; 20]),
        oracle_address: Address::from([2u8; 20]),
        source_chain_id: SOURCE_CHAIN_ID,
        chain_config: chain_config.clone(),
    };
    assert!(
        creation_data.verify().is_err(),
        "Bridge creation must be rejected when the ValidationProgram contains Assert"
    );

    // parse_burn_data fails for ALL burn proofs — funds are permanently locked.
    let burn_data = make_burn_data(1);
    let outgoing = OutgoingTransaction {
        burn_transaction_data: burn_data,
        merkle_proof: AnyMerkleProof::Blake2bPath(MerklePath::empty()),
        oracle_state_index: 0,
    };

    let result = outgoing.parse_burn_data(&chain_config);
    assert!(
        result.is_err(),
        "parse_burn_data must fail when the ValidationProgram contains Assert"
    );
}
