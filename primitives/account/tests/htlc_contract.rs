use std::convert::TryInto;

use nimiq_account::{
    Account, AccountPruningInteraction, AccountTransactionInteraction, Accounts, BasicAccount,
    BlockState, HashedTimeLockedContract, Log, ReservedBalance, TransactionLog,
};
use nimiq_database::traits::Database;
use nimiq_hash::{Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey, SecureGenerate};
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    networks::NetworkId,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::{
    accounts_revert::TestCommitRevert, test_rng::test_rng, transactions::TransactionsGenerator,
};
use nimiq_transaction::{
    account::htlc_contract::{
        AnyHash, AnyHash32, CreationTransactionData, OutgoingHTLCTransactionProof, PreImage,
    },
    EdDSASignatureProof, SignatureProof, Transaction,
};

const HTLC: &str = "00000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f0100000000000296350000000000000001";

fn prepare_outgoing_transaction() -> (
    HashedTimeLockedContract,
    Transaction,
    PreImage,
    SignatureProof,
    SignatureProof,
) {
    let sender_priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();
    let recipient_priv_key: PrivateKey = Deserialize::deserialize_from_vec(
        &hex::decode("bd1cfcd49a81048c8c8d22a25766bd01bfa0f6b2eb0030f65241189393af96a2").unwrap(),
    )
    .unwrap();

    let sender_key_pair = KeyPair::from(sender_priv_key);
    let recipient_key_pair = KeyPair::from(recipient_priv_key);
    let sender = Address::from(&sender_key_pair.public);
    let recipient = Address::from(&recipient_key_pair.public);
    let pre_image = PreImage::PreImage32(AnyHash32::from([1u8; 32]));
    let hash_root = AnyHash::from(
        Blake2bHasher::default().digest(
            Blake2bHasher::default()
                .digest(pre_image.as_bytes())
                .as_bytes(),
        ),
    );

    let htlc = HashedTimeLockedContract {
        balance: 1000.try_into().unwrap(),
        sender,
        recipient,
        hash_root,
        hash_count: 2,
        timeout: 100,
        total_amount: 1000.try_into().unwrap(),
    };

    let tx = Transaction::new_contract_creation(
        Address::from([0u8; 20]),
        AccountType::HTLC,
        vec![],
        AccountType::Basic,
        vec![],
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );

    let sender_signature = sender_key_pair.sign(&tx.serialize_content()[..]);
    let recipient_signature = recipient_key_pair.sign(&tx.serialize_content()[..]);
    let sender_signature_proof = SignatureProof::EdDSA(EdDSASignatureProof::from(
        sender_key_pair.public,
        sender_signature,
    ));
    let recipient_signature_proof = SignatureProof::EdDSA(EdDSASignatureProof::from(
        recipient_key_pair.public,
        recipient_signature,
    ));

    (
        htlc,
        tx,
        pre_image,
        sender_signature_proof,
        recipient_signature_proof,
    )
}

fn init_tree() -> (TestCommitRevert, KeyPair, KeyPair) {
    let accounts = TestCommitRevert::new();
    let generator = TransactionsGenerator::new(
        Accounts::new(accounts.env.clone()),
        NetworkId::UnitAlbatross,
        test_rng(true),
    );

    let mut rng = test_rng(true);

    let key_1 = KeyPair::generate(&mut rng);
    generator.put_account(
        &Address::from(&key_1),
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    );

    let key_2 = KeyPair::generate(&mut rng);
    generator.put_account(
        &Address::from(&key_2),
        Account::Basic(BasicAccount {
            balance: Coin::from_u64_unchecked(1000),
        }),
    );

    (accounts, key_1, key_2)
}

// This function is used to create the HTLC constant above.
#[test]
fn create_serialized_contract() {
    let contract = HashedTimeLockedContract {
        balance: Coin::ZERO,
        sender: "1b215589344cf570d36bec770825eae30b732139".parse().unwrap(),
        recipient: "24786862babbdb05e7c4430612135eb2a8368123".parse().unwrap(),
        hash_root: AnyHash::Sha256(AnyHash32::from(
            "daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f",
        )),
        hash_count: 1,
        timeout: 169525,
        total_amount: Coin::from_u64_unchecked(1),
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(contract.serialized_size());
    contract.serialize_to_writer(&mut bytes).unwrap();
    assert_eq!(HTLC, hex::encode(bytes));
}

#[test]
fn it_can_deserialize_a_htlc() {
    let bytes: Vec<u8> = hex::decode(HTLC).unwrap();
    let htlc: HashedTimeLockedContract = Deserialize::deserialize_from_vec(&bytes[..]).unwrap();
    assert_eq!(htlc.balance, Coin::ZERO);
    assert_eq!(htlc.hash_count, 1);
    assert_eq!(
        htlc.hash_root,
        AnyHash::Sha256(AnyHash32::from(
            "daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f"
        ))
    );
    assert_eq!(
        htlc.sender,
        "1b215589344cf570d36bec770825eae30b732139".parse().unwrap()
    );
    assert_eq!(
        htlc.recipient,
        "24786862babbdb05e7c4430612135eb2a8368123".parse().unwrap()
    );
    assert_eq!(htlc.timeout, 169525);
    assert_eq!(htlc.total_amount, 1.try_into().unwrap());
}

#[test]
fn it_can_serialize_a_htlc() {
    let bytes: Vec<u8> = hex::decode(HTLC).unwrap();
    let htlc: HashedTimeLockedContract = Deserialize::deserialize_from_vec(&bytes[..]).unwrap();
    let mut bytes2: Vec<u8> = Vec::with_capacity(htlc.serialized_size());
    let size = htlc.serialize_to_writer(&mut bytes2).unwrap();
    assert_eq!(size, htlc.serialized_size());
    assert_eq!(hex::encode(bytes2), HTLC);
}

#[test]
fn it_can_create_contract_from_transaction() {
    // Create contract creation transaction.
    let data = CreationTransactionData {
        sender: Address::from([0u8; 20]),
        recipient: Address::from([0u8; 20]),
        hash_root: AnyHash::Blake2b(AnyHash32::from([0u8; 32])),
        hash_count: 2,
        timeout: 1000,
    };

    let transaction = Transaction::new_contract_creation(
        data.sender.clone(),
        AccountType::Basic,
        vec![],
        AccountType::HTLC,
        data.serialize_to_vec(),
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::UnitAlbatross,
    );

    // Create contract from transaction.
    let (accounts, _key_1, _key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    let mut tx_logger = TransactionLog::empty();
    let htlc = accounts
        .test_create_new_contract::<HashedTimeLockedContract>(
            &transaction,
            Coin::ZERO,
            &block_state,
            &mut tx_logger,
            true,
        )
        .expect("Failed to create HTLC");

    let htlc = match htlc {
        Account::HTLC(htlc) => htlc,
        _ => panic!("Wrong account type created"),
    };

    assert_eq!(htlc.balance, 100.try_into().unwrap());
    assert_eq!(htlc.sender, data.sender);
    assert_eq!(htlc.recipient, data.recipient);
    assert_eq!(htlc.hash_root, AnyHash::Blake2b(AnyHash32::from([0u8; 32])));
    assert_eq!(htlc.hash_count, 2);
    assert_eq!(htlc.timeout, 1000);
    assert_eq!(
        tx_logger.logs,
        vec![Log::HTLCCreate {
            contract_address: transaction.contract_creation_address(),
            sender: htlc.sender,
            recipient: htlc.recipient,
            hash_root: htlc.hash_root,
            hash_count: htlc.hash_count,
            timeout: htlc.timeout,
            total_amount: htlc.total_amount
        }]
    );
}

#[test]
fn it_does_not_support_incoming_transactions() {
    let (accounts, _key_1, _key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    let (mut htlc, ..) = prepare_outgoing_transaction();

    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        1.try_into().unwrap(),
        1000.try_into().unwrap(),
        1,
        NetworkId::UnitAlbatross,
    );
    tx.recipient_type = AccountType::HTLC;

    let mut tx_logger = TransactionLog::empty();
    assert_eq!(
        accounts.test_commit_incoming_transaction(
            &mut htlc,
            &tx,
            &block_state,
            &mut tx_logger,
            true
        ),
        Err(AccountError::InvalidForRecipient)
    );
}

#[test]
fn it_can_apply_and_revert_regular_transfer() {
    let (accounts, _key_1, _key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    let (mut htlc, mut tx, pre_image, _sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // regular transfer
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 2,
        hash_root: htlc.hash_root.clone(),
        pre_image: pre_image.clone(),
        signature_proof: recipient_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let _receipt = accounts
        .test_commit_outgoing_transaction(&mut htlc, &tx, &block_state, &mut tx_logger, true)
        .expect("Failed to commit transaction");

    assert!(htlc.can_be_pruned());
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None
            },
            Log::HTLCRegularTransfer {
                contract_address: tx.sender,
                pre_image,
                hash_depth: 2
            }
        ]
    );
}

#[test]
fn it_can_apply_and_revert_early_resolve() {
    let (accounts, _key_1, _key_2) = init_tree();
    let block_state = BlockState::new(1, 1);

    let (mut htlc, mut tx, _pre_image, sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // early resolve
    let proof = OutgoingHTLCTransactionProof::EarlyResolve {
        signature_proof_recipient: recipient_signature_proof,
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let _receipt = accounts
        .test_commit_outgoing_transaction(&mut htlc, &tx, &block_state, &mut tx_logger, true)
        .expect("Failed to commit transaction");

    assert!(htlc.can_be_pruned());
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None
            },
            Log::HTLCEarlyResolve {
                contract_address: tx.sender.clone(),
            }
        ]
    );
}

#[test]
fn it_can_apply_and_revert_timeout_resolve() {
    let (accounts, _key_1, _key_2) = init_tree();

    let (mut htlc, mut tx, _pre_image, sender_signature_proof, _recipient_signature_proof) =
        prepare_outgoing_transaction();

    // timeout resolve
    let proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let block_state = BlockState::new(1, 101);

    let mut tx_logger = TransactionLog::empty();
    let _ = accounts
        .test_commit_outgoing_transaction(&mut htlc, &tx, &block_state, &mut tx_logger, true)
        .expect("Failed to commit transaction");

    assert!(htlc.can_be_pruned());
    assert_eq!(
        tx_logger.logs,
        vec![
            Log::PayFee {
                from: tx.sender.clone(),
                fee: tx.fee
            },
            Log::Transfer {
                from: tx.sender.clone(),
                to: tx.recipient.clone(),
                amount: tx.value,
                data: None
            },
            Log::HTLCTimeoutResolve {
                contract_address: tx.sender,
            }
        ]
    );
}

#[test]
fn it_refuses_invalid_transactions() {
    let (accounts, _key_1, _key_2) = init_tree();

    let (htlc, mut tx, pre_image, sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // regular transfer: timeout passed
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 2,
        hash_root: htlc.hash_root.clone(),
        pre_image: pre_image.clone(),
        signature_proof: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    let mut htlc = htlc.clone();

    let block_state = BlockState::new(1, 101);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidForSender));

    // regular transfer: hash mismatch
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 2,
        hash_root: AnyHash::Blake2b(AnyHash32([1u8; 32])),
        pre_image: pre_image.clone(),
        signature_proof: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    let block_state = BlockState::new(1, 1);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidForSender));

    // regular transfer: invalid signature
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 2,
        hash_root: htlc.hash_root.clone(),
        pre_image: pre_image.clone(),
        signature_proof: sender_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));

    // regular transfer: underflow
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 1,
        hash_root: htlc.hash_root.clone(),
        pre_image: PreImage::from(Blake2bHasher::default().digest(pre_image.as_bytes())),
        signature_proof: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: 1000.try_into().unwrap(),
            balance: 500.try_into().unwrap()
        })
    );

    // early resolve: invalid signature
    let proof = OutgoingHTLCTransactionProof::EarlyResolve {
        signature_proof_recipient: sender_signature_proof.clone(),
        signature_proof_sender: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));

    // timeout resolve: timeout not expired
    let proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidForSender));

    // timeout resolve: invalid signature
    let proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: recipient_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let block_state = BlockState::new(1, 101);

    let mut tx_logger = TransactionLog::empty();
    let result = accounts.test_commit_outgoing_transaction(
        &mut htlc,
        &tx,
        &block_state,
        &mut tx_logger,
        true,
    );

    assert_eq!(result, Err(AccountError::InvalidSignature));
}

#[test]
fn reserve_release_balance_works() {
    // -----------------------------------
    // Test setup:
    // -----------------------------------
    let (accounts, key_1, _key_2) = init_tree();
    let (htlc, mut tx, pre_image, _sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    let mut db_txn = accounts.env().write_transaction();
    let sender_address = Address::from(&key_1);
    let data_store = accounts.data_store(&sender_address);

    // regular transfer
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 2,
        hash_root: htlc.hash_root.clone(),
        pre_image: pre_image.clone(),
        signature_proof: recipient_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();
    let block_state = BlockState::new(2, 100);

    let mut reserved_balance = ReservedBalance::new(sender_address.clone());
    // -----------------------------------
    // Test execution:
    // -----------------------------------
    // Works in the normal case.
    let result = htlc.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_ok());

    // Doesn't work when there is not enough avl reserve.
    let result = htlc.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert_eq!(
        result,
        Err(AccountError::InsufficientFunds {
            needed: Coin::from_u64_unchecked(2000),
            balance: Coin::from_u64_unchecked(1000)
        })
    );

    // Can release and reserve again.
    let result = htlc.release_balance(&tx, &mut reserved_balance, data_store.read(&mut db_txn));
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(0));
    assert!(result.is_ok());

    let result = htlc.reserve_balance(
        &tx,
        &mut reserved_balance,
        &block_state,
        data_store.read(&mut db_txn),
    );
    assert_eq!(reserved_balance.balance(), Coin::from_u64_unchecked(1000));
    assert!(result.is_ok());
}
