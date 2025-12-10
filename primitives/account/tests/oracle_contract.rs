use std::convert::TryInto;

use nimiq_account::{
    Account, AccountPruningInteraction, AccountTransactionInteraction, BasicAccount, BlockState,
    Log, OracleContract, ReservedBalance, TransactionLog,
};
use nimiq_database::traits::Database;
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
    assert_eq!(oracle_contract.hashes.len(), 3);
    assert_eq!(oracle_contract.hashes[0], make_hash(1));
    assert_eq!(oracle_contract.hashes[1], make_hash(2));
    assert_eq!(oracle_contract.hashes[2], make_hash(3));

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

    // Now add 3 more hashes - should remove the oldest 3 and add the new ones
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
    // The first 3 should be removed (hashes 0, 1, 2)
    // The last 3 should be the new ones (hashes 10, 11, 12)
    assert_eq!(oracle_contract.hashes[0], make_hash(3)); // Oldest remaining
    assert_eq!(oracle_contract.hashes[9], make_hash(12)); // Newest
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
    // Should be back to the original 10 hashes
    assert_eq!(oracle_contract.hashes.len(), 10);
    assert_eq!(oracle_contract.hashes, initial_hashes);
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

    assert_eq!(oracle_contract.hashes.len(), 2);
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
