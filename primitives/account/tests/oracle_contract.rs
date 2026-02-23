use std::convert::TryInto;

use nimiq_account::{
    Account, AccountPruningInteraction, AccountReceipt, AccountTransactionInteraction,
    BasicAccount, BlockState, Log, OracleContract, ReservedBalance, TransactionLog,
};
use nimiq_database::traits::Database;
use nimiq_hash::{sha512::Sha512Hasher, Hasher, Keccak256Hasher, Sha256Hasher};
use nimiq_keys::{Address, KeyPair};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
    transaction::TransactionError,
};
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::{accounts_revert::TestCommitRevert, test_rng};
use nimiq_transaction::{
    account::{
        htlc_contract::{AnyHash, AnyHash32},
        oracle_contract::{CreationTransactionData, IncomingOracleTransactionData},
    },
    SignatureProof, Transaction,
};
use nimiq_utils::key_rng::SecureGenerate;

fn init_tree() -> (TestCommitRevert, OracleContract, KeyPair, KeyPair) {
    let mut rng = test_rng(true);
    let key_1 = KeyPair::generate(&mut rng);
    let key_2 = KeyPair::generate(&mut rng);
    let oracle_contract = OracleContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_1.public),
        hash_count: 10,
        hashes: Vec::new(),
        latest_index: None,
    };

    let accounts = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_1.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(10000),
            }),
        ),
        (Address([1u8; 20]), Account::Oracle(oracle_contract.clone())),
    ]);

    (accounts, oracle_contract, key_1, key_2)
}

fn make_signed_transaction(key: &KeyPair, recipient: Address, value: u64) -> Transaction {
    let mut tx = Transaction::new_basic(
        Address([1u8; 20]), // Contract address
        recipient,
        Coin::from_u64_unchecked(value),
        Coin::ZERO,
        1,
        NetworkId::UnitAlbatross,
    );
    tx.sender_type = AccountType::Oracle;
    let signature = key.sign(&tx.serialize_content());
    let signature_proof = SignatureProof::from_ed25519(key.public.clone(), signature);
    tx.proof = signature_proof.serialize_to_vec();

    tx
}

fn make_hash(value: u8) -> AnyHash {
    AnyHash::Blake2b(AnyHash32::from([value; 32]))
}

fn make_hash_blake2b(value: u8) -> AnyHash {
    AnyHash::Blake2b(AnyHash32::from([value; 32]))
}

fn make_hash_sha256(value: u8) -> AnyHash {
    AnyHash::from(Sha256Hasher::default().digest(&[value; 32]))
}

fn make_hash_sha512(value: u8) -> AnyHash {
    AnyHash::from(Sha512Hasher::default().digest(&[value; 32]))
}

fn make_hash_keccak256(value: u8) -> AnyHash {
    AnyHash::from(Keccak256Hasher::default().digest(&[value; 32]))
}

// Helper to compute chained hashes as the contract does: data_i = H(data_{i-1} || state_i)
fn compute_chained_hashes(hashes: &[AnyHash]) -> Vec<AnyHash> {
    if hashes.is_empty() {
        return Vec::new();
    }
    let zero_hash = hashes[0].zero_of_same_type();
    compute_chained_hashes_from_previous(hashes, &zero_hash)
}

// Helper to compute chained hashes starting from a previous hash (uses AnyHash::digest like the contract)
fn compute_chained_hashes_from_previous(
    hashes: &[AnyHash],
    previous_hash: &AnyHash,
) -> Vec<AnyHash> {
    if hashes.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut current_hash = previous_hash.clone();
    for new_hash in hashes {
        current_hash = current_hash.digest(new_hash);
        result.push(current_hash.clone());
    }
    result
}

fn make_update_transaction(
    contract_address: Address,
    key: &KeyPair,
    hashes: Vec<AnyHash>,
) -> Transaction {
    // Create update data without signature first
    let update_data = IncomingOracleTransactionData::Update {
        hashes,
        proof: SignatureProof::default(),
    };

    let mut tx = Transaction::new_signaling(
        contract_address.clone(),
        AccountType::Oracle,
        contract_address,
        AccountType::Oracle,
        Coin::ZERO,
        update_data.serialize_to_vec(),
        0,
        NetworkId::UnitAlbatross,
    );

    // Create signature proof by signing the transaction with the data (but without signature)
    let signature = key.sign(&tx.serialize_content());
    let signature_proof = SignatureProof::from_ed25519(key.public, signature);

    // Set the signature in the recipient_data
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, signature_proof)
            .expect("Failed to set signature on data");

    tx
}

fn make_change_owner_transaction(
    contract_address: Address,
    key: &KeyPair,
    new_owner: Address,
) -> Transaction {
    // Create change owner data without signature first
    let change_owner_data = IncomingOracleTransactionData::ChangeOwner {
        new_owner,
        proof: SignatureProof::default(),
    };

    let mut tx = Transaction::new_signaling(
        contract_address.clone(),
        AccountType::Oracle,
        contract_address,
        AccountType::Oracle,
        Coin::ZERO,
        change_owner_data.serialize_to_vec(),
        0,
        NetworkId::UnitAlbatross,
    );

    // Create signature proof by signing the transaction with the data (but without signature)
    let signature = key.sign(&tx.serialize_content());
    let signature_proof = SignatureProof::from_ed25519(key.public, signature);

    // Set the signature in the recipient_data
    tx.recipient_data =
        IncomingOracleTransactionData::set_signature_on_data(&tx.recipient_data, signature_proof)
            .expect("Failed to set signature on data");

    tx
}

#[test]
fn it_can_create_contract_from_transaction() {
    let (accounts, _oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Create transaction data
    let creation_data = CreationTransactionData {
        owner: Address::from(&key_1.public),
        hash_count: 5,
    };

    let tx = Transaction::new_contract_creation(
        Address::from(&key_1.public),
        AccountType::Basic,
        vec![],
        AccountType::Oracle,
        creation_data.serialize_to_vec(),
        1000.try_into().unwrap(), // Deposit
        0.try_into().unwrap(),    // Fee
        0,
        NetworkId::UnitAlbatross,
    );

    let mut tx_logger = TransactionLog::empty();
    let contract = accounts
        .test_create_new_contract::<OracleContract>(
            &tx,
            Coin::ZERO,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to create contract");

    assert_eq!(
        tx_logger.logs,
        vec![Log::OracleCreate {
            contract_address: tx.contract_creation_address(),
            owner: Address::from(&key_1.public),
            hash_count: 5,
            deposit: 1000.try_into().unwrap(),
        }]
    );

    let contract = match contract {
        Account::Oracle(contract) => contract,
        _ => panic!("Wrong account type created"),
    };

    assert_eq!(contract.balance, 1000.try_into().unwrap());
    assert_eq!(contract.owner, Address::from(&key_1.public));
    assert_eq!(contract.hash_count, 5);
    assert_eq!(contract.hashes.len(), 0);
}

#[test]
fn it_rejects_contract_creation_with_zero_hash_count() {
    let (accounts, _oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Create transaction data with zero hash_count
    let creation_data = CreationTransactionData {
        owner: Address::from(&key_1.public),
        hash_count: 0, // Invalid
    };

    let tx = Transaction::new_contract_creation(
        Address::from(&key_1.public),
        AccountType::Basic,
        vec![],
        AccountType::Oracle,
        creation_data.serialize_to_vec(),
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::UnitAlbatross,
    );

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_create_new_contract::<OracleContract>(
        &tx,
        Coin::ZERO,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert!(result.is_err());
    assert!(matches!(
        result,
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        ))
    ));
}

#[test]
fn it_can_update_contract_with_hashes() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Create update transaction
    let hashes = vec![make_hash(1), make_hash(2), make_hash(3)];
    let tx = make_update_transaction(Address([1u8; 20]), &key_1, hashes.clone());

    let mut tx_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to update contract");

    assert_eq!(receipt, None);
    assert_eq!(
        oracle_contract.hashes.len(),
        oracle_contract.hash_count as usize
    );
    assert_eq!(oracle_contract.latest_index, Some(2));
    let expected_chained = compute_chained_hashes(&hashes);
    assert_eq!(oracle_contract.get_hashes_chronological(), expected_chained);

    assert_eq!(
        tx_logger.logs,
        vec![Log::OracleUpdate {
            contract_address: Address([1u8; 20]),
            hashes: hashes.clone(),
        }]
    );
}

#[test]
fn it_implements_ring_buffer() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // First, fill the contract to capacity (hash_count = 10)
    let initial_hashes: Vec<AnyHash> = (0..10).map(|i| make_hash(i)).collect();
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, initial_hashes.clone());
    let mut tx_logger = TransactionLog::empty();
    accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx1,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to add initial hashes");
    assert_eq!(oracle_contract.hashes.len(), 10);

    // Now add 3 more hashes - should remove the oldest 3 and add the new ones.
    // Note: This update starts at index 10 (== hash_count), so every write evicts; removed_hashes
    // align with (start_index + i) % hash_count. For the straddle case (some evictions, some not),
    // see it_reverts_update_straddling_hash_count_boundary.
    let new_hashes: Vec<AnyHash> = (10..13).map(|i| make_hash(i)).collect();
    let tx2 = make_update_transaction(Address([1u8; 20]), &key_1, new_hashes.clone());
    let mut tx_logger2 = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx2,
            &block_state,
            &mut tx_logger2,
            true,
        )
        .expect("Failed to add new hashes");

    // Should still have exactly hash_count hashes
    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(12));
    // With ring buffer, after writing indices 0-9, then 10-12:
    // - Positions 0-2 have indices 10-12 (newest, overwriting 0-2)
    // - Positions 3-9 have indices 3-9 (oldest remaining)
    // Check using chronological order helper
    let expected_all = compute_chained_hashes(&initial_hashes);
    // The new hashes are chained from the last hash in expected_all, not from zero
    let previous_hash = expected_all
        .last()
        .cloned()
        .unwrap_or_else(|| new_hashes[0].zero_of_same_type());
    let expected_new = compute_chained_hashes_from_previous(&new_hashes, &previous_hash);
    let expected_remaining: Vec<AnyHash> = expected_all[3..].to_vec();
    let expected_final: Vec<AnyHash> = expected_remaining
        .into_iter()
        .chain(expected_new.into_iter())
        .collect();

    // Get hashes in chronological order
    let chronological = oracle_contract.get_hashes_chronological();
    assert_eq!(chronological, expected_final);

    // Should have a receipt with the removed hashes
    assert!(receipt.is_some());

    // Test revert - should restore the removed hashes
    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let data_store = accounts.data_store(&Address([1u8; 20]));
    oracle_contract
        .revert_incoming_transaction(
            &tx2,
            &block_state,
            receipt,
            data_store.write(&mut txn),
            &mut tx_logger2,
        )
        .expect("Failed to revert");
    // Should be back to the original 10 chained hashes
    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(9));
    let chronological_after_revert = oracle_contract.get_hashes_chronological();
    assert_eq!(chronological_after_revert, expected_all);
}

/// Revert when an update **straddles** the hash_count boundary: some writes don't evict (index < hash_count),
/// some do. The only revert test above uses start_index = hash_count (all evictions), so (start_index + i) % hash_count
/// accidentally matches the correct (hash_count + i) % hash_count. This test uses start_index = 9, add 3 hashes
/// (indices 9, 10, 11) so only indices 10 and 11 evict; revert must restore removed_hashes to positions 0 and 1.
#[test]
fn it_reverts_update_straddling_hash_count_boundary() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Fill to 9 hashes (indices 0..8), so latest_index = 8 and next start_index = 9
    let initial_hashes: Vec<AnyHash> = (0..9).map(|i| make_hash(i)).collect();
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, initial_hashes.clone());
    let mut tx_logger = TransactionLog::empty();
    accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx1,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to add initial hashes");
    // Ring buffer is resized to hash_count on first write, so len is 10
    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(8));

    // Add 3 hashes (indices 9, 10, 11): index 9 does not evict (9 < 10), indices 10 and 11 evict (positions 0, 1)
    let straddle_hashes: Vec<AnyHash> = (9..12).map(|i| make_hash(i)).collect();
    let tx2 = make_update_transaction(Address([1u8; 20]), &key_1, straddle_hashes.clone());
    let mut tx_logger2 = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx2,
            &block_state,
            &mut tx_logger2,
            true,
        )
        .expect("Failed to add straddle hashes");

    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(11));
    assert!(receipt.is_some());

    // After commit: ring has indices 2..11 (positions 2-8 = indices 2-8, pos 9 = index 9, pos 0-1 = indices 10-11)
    let expected_after_commit = {
        let expected_all = compute_chained_hashes(&initial_hashes);
        let previous = expected_all.last().cloned().unwrap();
        let expected_new = compute_chained_hashes_from_previous(&straddle_hashes, &previous);
        let remaining = expected_all[2..].to_vec();
        remaining
            .into_iter()
            .chain(expected_new.into_iter())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        oracle_contract.get_hashes_chronological(),
        expected_after_commit
    );

    // Revert: must restore removed_hashes[0] to pos 0 and removed_hashes[1] to pos 1 (not to pos 9 and 0)
    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let data_store = accounts.data_store(&Address([1u8; 20]));
    oracle_contract
        .revert_incoming_transaction(
            &tx2,
            &block_state,
            receipt,
            data_store.write(&mut txn),
            &mut tx_logger2,
        )
        .expect("Failed to revert");

    // Revert does not shrink the ring buffer; chronological content is restored
    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(8));
    let expected_all = compute_chained_hashes(&initial_hashes);
    assert_eq!(oracle_contract.get_hashes_chronological(), expected_all);
}

/// Revert after the ring buffer has already wrapped multiple times.
#[test]
fn it_reverts_update_after_multiple_wraps() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Fill the buffer twice plus one entry, so latest_index = 20 and the next write starts at pos 1.
    let initial_hashes: Vec<AnyHash> = (0..21).map(|i| make_hash(i)).collect();
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, initial_hashes.clone());
    let mut tx_logger = TransactionLog::empty();
    accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx1,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to pre-fill oracle");

    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(20));

    let wrap_hashes: Vec<AnyHash> = (21..24).map(|i| make_hash(i)).collect();
    let tx2 = make_update_transaction(Address([1u8; 20]), &key_1, wrap_hashes.clone());
    let mut tx_logger2 = TransactionLog::empty();
    let receipt = {
        let mut db_txn = accounts.env().write_transaction();
        let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
        let data_store = accounts.data_store(&Address([1u8; 20]));
        oracle_contract
            .commit_incoming_transaction(
                &tx2,
                &block_state,
                data_store.write(&mut txn),
                &mut tx_logger2,
            )
            .expect("Failed to add post-wrap hashes")
    };

    assert_eq!(oracle_contract.latest_index, Some(23));
    assert!(receipt.is_some());

    let expected_all = compute_chained_hashes(&initial_hashes);
    let expected_previous = expected_all.last().cloned().unwrap();
    let expected_new = compute_chained_hashes_from_previous(&wrap_hashes, &expected_previous);
    let expected_after_commit = expected_all[14..]
        .iter()
        .cloned()
        .chain(expected_new)
        .collect::<Vec<_>>();
    assert_eq!(
        oracle_contract.get_hashes_chronological(),
        expected_after_commit
    );

    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let data_store = accounts.data_store(&Address([1u8; 20]));
    oracle_contract
        .revert_incoming_transaction(
            &tx2,
            &block_state,
            receipt,
            data_store.write(&mut txn),
            &mut tx_logger2,
        )
        .expect("Failed to revert multi-wrap update");

    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.latest_index, Some(20));
    assert_eq!(
        oracle_contract.get_hashes_chronological(),
        expected_all[11..].to_vec()
    );
}

#[test]
fn it_can_change_owner() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();

    let block_state = BlockState::new(1, 1);
    let old_owner = oracle_contract.owner.clone();
    let new_owner = Address::from(&key_2.public);

    // Create owner change transaction
    let tx = make_change_owner_transaction(Address([1u8; 20]), &key_1, new_owner.clone());

    let mut tx_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to change owner");

    assert!(receipt.is_some()); // Should return a receipt for owner change
    assert_eq!(oracle_contract.owner, new_owner.clone());

    assert_eq!(
        tx_logger.logs,
        vec![Log::OracleChangeOwner {
            contract_address: Address([1u8; 20]),
            old_owner,
            new_owner: new_owner.clone(),
        }]
    );
}

#[test]
fn it_rejects_owner_change_with_invalid_signature() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();

    let block_state = BlockState::new(1, 1);
    let new_owner = Address::from(&key_2.public);

    // Create owner change transaction signed by wrong key
    let tx = make_change_owner_transaction(Address([1u8; 20]), &key_2, new_owner);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));
    assert_eq!(oracle_contract.owner, Address::from(&key_1.public)); // Owner should not change
}

#[test]
fn it_can_delete_contract_by_removing_balance() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Create transaction to withdraw full balance
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 1000);

    let mut tx_logger = TransactionLog::empty();
    let _ = accounts
        .test_commit_outgoing_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to commit transaction");

    assert_eq!(oracle_contract.balance, Coin::ZERO);

    // Contract should be prunable
    assert!(oracle_contract.can_be_pruned());
}

#[test]
fn it_rejects_partial_withdrawal() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Try to withdraw partial balance
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 500);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut oracle_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidForSender));
    assert_eq!(oracle_contract.balance, 1000.try_into().unwrap()); // Balance should not change
}

#[test]
fn it_rejects_withdrawal_with_invalid_signature() {
    let (accounts, mut oracle_contract, _key_1, key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // Create transaction signed by wrong key
    let tx = make_signed_transaction(&key_2, Address::from(&key_2.public), 1000);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut oracle_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));
    assert_eq!(oracle_contract.balance, 1000.try_into().unwrap()); // Balance should not change
}

#[test]
fn it_does_not_support_regular_incoming_transactions() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    let mut tx = Transaction::new_basic(
        Address::from(&key_1.public),
        Address([1u8; 20]),
        1.try_into().unwrap(),
        1000.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    tx.recipient_type = AccountType::Oracle;
    // Not a signaling transaction

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidForRecipient));
    assert_eq!(tx_logger.logs.len(), 0);
}

#[test]
fn it_can_apply_and_revert_transaction() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // First, add some hashes
    let hashes = vec![make_hash(1), make_hash(2)];
    let tx = make_update_transaction(Address([1u8; 20]), &key_1, hashes.clone());

    let mut tx_logger = TransactionLog::empty();
    let _receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to update contract");

    assert_eq!(
        oracle_contract.hashes.len(),
        oracle_contract.hash_count as usize
    );
    let expected_hashes = compute_chained_hashes(&hashes);
    assert_eq!(oracle_contract.get_hashes_chronological(), expected_hashes);
}

#[test]
fn reserve_release_balance_works() {
    let (accounts, oracle_contract, key_1, key_2) = init_tree();
    let mut db_txn = accounts.env().write_transaction();
    let sender_address = Address([1u8; 20]);
    let data_store = accounts.data_store(&sender_address);

    let block_state = BlockState::new(1, 1);

    let mut reserved_balance = ReservedBalance::new(sender_address.clone());

    // Reserve balance - need to account for fee too
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 500);
    let result = oracle_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    // total_value = value + fee = 500 + 0 = 500
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(500));
    assert!(result.is_ok());

    // Reserve more
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 500);
    let result = oracle_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_ok());

    // Can't reserve more than balance
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 1);
    let result = oracle_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_err());

    // Release and reserve again
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 500);
    let result =
        oracle_contract.release_balance(&tx, &mut reserved_balance, data_store.read(&mut db_txn));
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(500));
    assert!(result.is_ok());

    let result = oracle_contract.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_ok());
}

#[test]
fn it_can_be_pruned_when_balance_is_zero() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();

    // Initially, contract should not be prunable
    assert!(!oracle_contract.can_be_pruned());

    // Withdraw full balance
    let block_state = BlockState::new(1, 1);
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 1000);

    let mut tx_logger = TransactionLog::empty();
    let _ = accounts
        .test_commit_outgoing_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to commit transaction");

    assert_eq!(oracle_contract.balance, Coin::ZERO);
    assert!(oracle_contract.can_be_pruned());

    // Test pruning
    let pruned_receipt = oracle_contract.prune(
        accounts
            .data_store(&Address([1u8; 20]))
            .read(&mut accounts.env().write_transaction()),
    );
    assert!(pruned_receipt.is_some());
}

#[test]
fn it_can_restore_from_pruned_receipt() {
    let (accounts, oracle_contract, _key_1, _key_2) = init_tree();

    // Save contract state before pruning
    let original_owner = oracle_contract.owner.clone();
    let original_hash_count = oracle_contract.hash_count;
    let original_hashes = oracle_contract.hashes.clone();

    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let pruned_receipt =
        oracle_contract.prune(accounts.data_store(&Address([1u8; 20])).read(&mut txn));
    assert!(pruned_receipt.is_some());

    // Restore contract
    let restored_account = Account::restore(
        AccountType::Oracle,
        pruned_receipt.as_ref(),
        accounts.data_store(&Address([1u8; 20])).write(&mut txn),
    )
    .expect("Failed to restore contract");

    let restored = match restored_account {
        Account::Oracle(contract) => contract,
        _ => panic!("Wrong account type restored"),
    };

    assert_eq!(restored.balance, Coin::ZERO);
    assert_eq!(restored.owner, original_owner);
    assert_eq!(restored.hash_count, original_hash_count);
    assert_eq!(restored.hashes, original_hashes);
}

#[test]
fn it_rejects_mixing_hash_types() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // First, add Blake2b hashes
    let blake2b_hashes = vec![make_hash_blake2b(1), make_hash_blake2b(2)];
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, blake2b_hashes.clone());
    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx1,
        &block_state,
        &mut tx_logger,
        true,
    );
    assert!(result.is_ok(), "Should accept Blake2b hashes");
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 2);
    assert_eq!(oracle_contract.latest_index, Some(1));

    // Try to add Sha256 hashes - should fail
    let sha256_hashes = vec![make_hash_sha256(3)];
    let tx2 = make_update_transaction(Address([1u8; 20]), &key_1, sha256_hashes);
    let mut tx_logger2 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx2,
        &block_state,
        &mut tx_logger2,
        true,
    );
    assert_eq!(
        result,
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        )),
        "Should reject Sha256 hashes when contract uses Blake2b"
    );
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 2);

    // Try to add Sha512 hashes - should fail
    let sha512_hashes = vec![make_hash_sha512(4)];
    let tx3 = make_update_transaction(Address([1u8; 20]), &key_1, sha512_hashes);
    let mut tx_logger3 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx3,
        &block_state,
        &mut tx_logger3,
        true,
    );
    assert_eq!(
        result,
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        )),
        "Should reject Sha512 hashes when contract uses Blake2b"
    );
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 2);

    // Try to add Keccak256 hashes - should fail
    let keccak256_hashes = vec![make_hash_keccak256(5)];
    let tx4 = make_update_transaction(Address([1u8; 20]), &key_1, keccak256_hashes);
    let mut tx_logger4 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx4,
        &block_state,
        &mut tx_logger4,
        true,
    );
    assert_eq!(
        result,
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        )),
        "Should reject Keccak256 hashes when contract uses Blake2b"
    );
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 2);

    // Adding more Blake2b hashes should still work
    let more_blake2b_hashes = vec![make_hash_blake2b(6), make_hash_blake2b(7)];
    let tx5 = make_update_transaction(Address([1u8; 20]), &key_1, more_blake2b_hashes.clone());
    let mut tx_logger5 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx5,
        &block_state,
        &mut tx_logger5,
        true,
    );
    assert!(result.is_ok(), "Should accept more Blake2b hashes");
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 4);
}

#[test]
fn it_allows_different_hash_types_in_different_contracts() {
    let (accounts, mut oracle_contract1, key_1, _key_2) = init_tree();
    let mut rng = test_rng(true);
    let key_3 = KeyPair::generate(&mut rng);
    let oracle_contract2 = OracleContract {
        balance: 1000.try_into().unwrap(),
        owner: Address::from(&key_3.public),
        hash_count: 10,
        hashes: Vec::new(),
        latest_index: None,
    };

    let accounts2 = TestCommitRevert::with_initial_state(&[
        (
            Address::from(&key_3.public),
            Account::Basic(BasicAccount {
                balance: Coin::from_u64_unchecked(10000),
            }),
        ),
        (
            Address([2u8; 20]),
            Account::Oracle(oracle_contract2.clone()),
        ),
    ]);

    let block_state = BlockState::new(1, 1);

    // Contract 1 uses Blake2b
    let blake2b_hashes = vec![make_hash_blake2b(1)];
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, blake2b_hashes);
    let mut tx_logger1 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract1,
        &tx1,
        &block_state,
        &mut tx_logger1,
        true,
    );
    assert!(result.is_ok(), "Contract 1 should accept Blake2b hashes");

    // Contract 2 uses Sha256
    let mut oracle_contract2_mut = oracle_contract2;
    let sha256_hashes = vec![make_hash_sha256(1)];
    let tx2 = make_update_transaction(Address([2u8; 20]), &key_3, sha256_hashes);
    let mut tx_logger2 = TransactionLog::empty();
    let result = accounts2.test_commit_incoming_transaction(
        &mut oracle_contract2_mut,
        &tx2,
        &block_state,
        &mut tx_logger2,
        true,
    );
    assert!(result.is_ok(), "Contract 2 should accept Sha256 hashes");

    assert_eq!(oracle_contract1.get_hashes_chronological().len(), 1);
    assert_eq!(oracle_contract1.latest_index, Some(0));
    assert_eq!(oracle_contract2_mut.get_hashes_chronological().len(), 1);
    assert_eq!(oracle_contract2_mut.latest_index, Some(0));
    match (
        &oracle_contract1.get_hashes_chronological()[0],
        &oracle_contract2_mut.get_hashes_chronological()[0],
    ) {
        (AnyHash::Blake2b(_), AnyHash::Sha256(_)) => {
            // Correct - different hash types in different contracts
        }
        _ => panic!("Contracts should have different hash types"),
    }
}

#[test]
fn it_handles_empty_hash_update_with_existing_hashes() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();

    let block_state = BlockState::new(1, 1);

    // First, add some Sha256 hashes to establish the contract's hash type
    let sha256_hashes = vec![make_hash_sha256(1), make_hash_sha256(2)];
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, sha256_hashes.clone());
    let mut tx_logger1 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx1,
        &block_state,
        &mut tx_logger1,
        true,
    );
    assert!(result.is_ok(), "Should accept Sha256 hashes");
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 2);
    assert_eq!(oracle_contract.latest_index, Some(1));

    // Save the state before empty update
    let hashes_before = oracle_contract.hashes.clone();
    let latest_index_before = oracle_contract.latest_index;

    // Now try to update with an empty hash array
    // This should be allowed and should preserve the contract's hash type
    let empty_hashes: Vec<AnyHash> = vec![];
    let tx2 = make_update_transaction(Address([1u8; 20]), &key_1, empty_hashes);
    let mut tx_logger2 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx2,
        &block_state,
        &mut tx_logger2,
        true,
    );
    assert!(result.is_ok(), "Should accept empty hash update");

    // Contract state should remain unchanged
    assert_eq!(oracle_contract.hashes, hashes_before);
    assert_eq!(oracle_contract.latest_index, latest_index_before);

    // Verify the contract still uses Sha256 type by checking the hash type
    // The zero hash type should be determined from existing hashes (Sha256)
    // This is tested implicitly - if we add more hashes, they should match Sha256
    let new_sha256_hashes = vec![make_hash_sha256(3)];
    let tx3 = make_update_transaction(Address([1u8; 20]), &key_1, new_sha256_hashes.clone());
    let mut tx_logger3 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx3,
        &block_state,
        &mut tx_logger3,
        true,
    );
    assert!(
        result.is_ok(),
        "Should still accept Sha256 hashes after empty update"
    );
    assert_eq!(oracle_contract.get_hashes_chronological().len(), 3);
    assert_eq!(oracle_contract.latest_index, Some(2));

    // Try to add Blake2b - should fail (type mismatch)
    let blake2b_hashes = vec![make_hash_blake2b(4)];
    let tx4 = make_update_transaction(Address([1u8; 20]), &key_1, blake2b_hashes);
    let mut tx_logger4 = TransactionLog::empty();
    let result = accounts.test_commit_incoming_transaction(
        &mut oracle_contract,
        &tx4,
        &block_state,
        &mut tx_logger4,
        true,
    );
    assert_eq!(
        result,
        Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData
        )),
        "Should reject Blake2b hashes when contract uses Sha256"
    );
}

#[test]
fn it_commits_and_reverts_failed_transaction() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    // Failed transactions on Oracle contracts are fee-only. With zero fee this is a no-op
    // on balance, but still exercises commit_failed/revert_failed and log symmetry.
    let tx = make_signed_transaction(&key_1, Address::from(&key_2.public), 1);
    let mut tx_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_failed_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to commit/revert failed transaction");

    assert_eq!(receipt, None);
    assert_eq!(oracle_contract.balance, 1000.try_into().unwrap());
}

#[test]
fn it_reverts_update_when_num_hashes_exceeds_current_latest() {
    let (accounts, mut oracle_contract, key_1, _key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    // Put exactly one hash, so latest_index = 0.
    let tx1 = make_update_transaction(Address([1u8; 20]), &key_1, vec![make_hash(1)]);
    let mut tx_logger1 = TransactionLog::empty();
    accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx1,
            &block_state,
            &mut tx_logger1,
            true,
        )
        .expect("Failed to add initial hash");
    assert_eq!(oracle_contract.latest_index, Some(0));

    // Revert a larger update (2 hashes) directly. This exercises the defensive branch
    // num_hashes > current_latest, which clears state.
    let revert_tx = make_update_transaction(
        Address([1u8; 20]),
        &key_1,
        vec![make_hash(10), make_hash(11)],
    );
    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let data_store = accounts.data_store(&Address([1u8; 20]));
    let mut tx_logger2 = TransactionLog::empty();
    oracle_contract
        .revert_incoming_transaction(
            &revert_tx,
            &block_state,
            None,
            data_store.write(&mut txn),
            &mut tx_logger2,
        )
        .expect("Failed to revert oversized update");

    assert_eq!(oracle_contract.latest_index, None);
    assert!(oracle_contract.hashes.is_empty());
}

#[test]
fn it_rejects_revert_owner_change_without_receipt() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    // Commit owner change first.
    let tx =
        make_change_owner_transaction(Address([1u8; 20]), &key_1, Address::from(&key_2.public));
    let mut tx_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &tx,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to change owner");
    assert!(receipt.is_some());

    // Now try to revert without receipt: should be rejected.
    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let data_store = accounts.data_store(&Address([1u8; 20]));
    let mut revert_logger = TransactionLog::empty();
    let result = oracle_contract.revert_incoming_transaction(
        &tx,
        &block_state,
        None,
        data_store.write(&mut txn),
        &mut revert_logger,
    );

    assert_eq!(result, Err(AccountError::InvalidReceipt));
    assert_eq!(oracle_contract.owner, Address::from(&key_2.public));
}

#[test]
fn it_rejects_revert_owner_change_with_invalid_receipt_serialization() {
    let (accounts, mut oracle_contract, key_1, key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    // Commit owner change.
    let owner_change_tx =
        make_change_owner_transaction(Address([1u8; 20]), &key_1, Address::from(&key_2.public));
    let mut owner_change_logger = TransactionLog::empty();
    let receipt = accounts
        .test_commit_incoming_transaction(
            &mut oracle_contract,
            &owner_change_tx,
            &block_state,
            &mut owner_change_logger,
            true,
        )
        .expect("Failed to change owner");
    assert!(receipt.is_some());

    // Revert owner change with an invalidly encoded receipt.
    let mut db_txn = accounts.env().write_transaction();
    let mut txn: nimiq_trie::WriteTransactionProxy = (&mut db_txn).into();
    let data_store = accounts.data_store(&Address([1u8; 20]));
    let mut revert_logger = TransactionLog::empty();
    let result = oracle_contract.revert_incoming_transaction(
        &owner_change_tx,
        &block_state,
        Some(AccountReceipt(vec![0xff])),
        data_store.write(&mut txn),
        &mut revert_logger,
    );

    assert!(
        matches!(result, Err(AccountError::InvalidSerialization(_))),
        "Expected invalid serialization error, got: {result:?}"
    );
    assert_eq!(oracle_contract.owner, Address::from(&key_2.public));
}
