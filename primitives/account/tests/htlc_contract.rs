use std::convert::TryInto;

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_account::{
    Account, AccountError, AccountTransactionInteraction, AccountsTrie, HashedTimeLockedContract,
};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHasher, HashOutput, Hasher, Sha256Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::account::htlc_contract::{AnyHash, HashAlgorithm, ProofType};
use nimiq_transaction::account::AccountTransactionVerification;
use nimiq_transaction::{SignatureProof, Transaction, TransactionError, TransactionFlags};
use nimiq_trie::key_nibbles::KeyNibbles;

const HTLC: &str = "00000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f0100000000000296350000000000000001";

// This function is used to create the HTLC constant above. If you need to generate a new one just
// uncomment out the test flag.
//#[test]
#[allow(dead_code)]
fn create_serialized_contract() {
    let contract = HashedTimeLockedContract {
        balance: Coin::ZERO,
        sender: "1b215589344cf570d36bec770825eae30b732139".parse().unwrap(),
        recipient: "24786862babbdb05e7c4430612135eb2a8368123".parse().unwrap(),
        hash_algorithm: HashAlgorithm::Sha256,
        hash_root: AnyHash::from(
            "daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f",
        ),
        hash_count: 1,
        timeout: 169525,
        total_amount: Coin::from_u64_unchecked(1),
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(contract.serialized_size());
    contract.serialize(&mut bytes).unwrap();
    println!("{}", hex::encode(bytes));
    panic!();
}

#[test]
fn it_can_deserialize_a_htlc() {
    let bytes: Vec<u8> = hex::decode(HTLC).unwrap();
    let htlc: HashedTimeLockedContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    assert_eq!(htlc.balance, Coin::ZERO);
    assert_eq!(htlc.hash_algorithm, HashAlgorithm::Sha256);
    assert_eq!(htlc.hash_count, 1);
    assert_eq!(
        htlc.hash_root,
        AnyHash::from("daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f")
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
    let htlc: HashedTimeLockedContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    let mut bytes2: Vec<u8> = Vec::with_capacity(htlc.serialized_size());
    let size = htlc.serialize(&mut bytes2).unwrap();
    assert_eq!(size, htlc.serialized_size());
    assert_eq!(hex::encode(bytes2), HTLC);
}

#[test]
#[allow(unused_must_use)]
fn it_can_verify_creation_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE * 2 + AnyHash::SIZE + 10);
    let sender = Address::from([0u8; 20]);
    let recipient = Address::from([0u8; 20]);
    sender.serialize(&mut data);
    recipient.serialize(&mut data);
    HashAlgorithm::Blake2b.serialize(&mut data);
    AnyHash::from([0u8; 32]).serialize(&mut data);
    Serialize::serialize(&2u8, &mut data);
    Serialize::serialize(&1000u64, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        vec![],
        sender,
        AccountType::Basic,
        AccountType::HTLC,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    // Invalid data
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidData)
    );
    transaction.data = data;

    // Invalid recipient
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
    transaction.recipient = transaction.contract_creation_address();

    // Valid
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Ok(())
    );

    // Invalid transaction flags
    transaction.flags = TransactionFlags::empty();
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidForRecipient)
    );
    transaction.flags = TransactionFlags::CONTRACT_CREATION;

    // Invalid hash algorithm
    transaction.data[40] = 200;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidSerialization(
            SerializingError::InvalidValue
        ))
    );
    transaction.data[40] = 1;

    // Invalid zero hash count
    transaction.data[73] = 0;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidData)
    );
}

#[test]
#[allow(unused_must_use)]
fn it_can_create_contract_from_transaction() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE * 2 + AnyHash::SIZE + 10);
    let sender = Address::from([0u8; 20]);
    let recipient = Address::from([0u8; 20]);
    sender.serialize(&mut data);
    recipient.serialize(&mut data);
    HashAlgorithm::Blake2b.serialize(&mut data);
    AnyHash::from([0u8; 32]).serialize(&mut data);
    Serialize::serialize(&2u8, &mut data);
    Serialize::serialize(&1000u64, &mut data);
    let transaction = Transaction::new_contract_creation(
        data,
        sender.clone(),
        AccountType::Basic,
        AccountType::HTLC,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    HashedTimeLockedContract::create(&accounts_tree, &mut db_txn, &transaction, 0, 0);

    match accounts_tree.get(
        &db_txn,
        &KeyNibbles::from(&transaction.contract_creation_address()),
    ) {
        Some(Account::HTLC(htlc)) => {
            assert_eq!(htlc.balance, 100.try_into().unwrap());
            assert_eq!(htlc.sender, sender);
            assert_eq!(htlc.recipient, recipient);
            assert_eq!(htlc.hash_root, AnyHash::from([0u8; 32]));
            assert_eq!(htlc.hash_count, 2);
            assert_eq!(htlc.timeout, 1000);
        }
        _ => panic!(),
    }
}

#[test]
fn it_does_not_support_incoming_transactions() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    let mut tx = Transaction::new_basic(
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        1.try_into().unwrap(),
        1000.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );
    tx.recipient_type = AccountType::HTLC;

    assert_eq!(
        HashedTimeLockedContract::commit_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            2
        ),
        Err(AccountError::InvalidForRecipient)
    );
    assert_eq!(
        HashedTimeLockedContract::revert_incoming_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            2,
            2,
            None
        ),
        Err(AccountError::InvalidForRecipient)
    );
}

fn prepare_outgoing_transaction() -> (
    HashedTimeLockedContract,
    Transaction,
    AnyHash,
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
    let pre_image = AnyHash::from([1u8; 32]);
    let hash_root = AnyHash::from(<[u8; 32]>::from(
        Blake2bHasher::default().digest(
            Blake2bHasher::default()
                .digest(pre_image.as_bytes())
                .as_bytes(),
        ),
    ));

    let start_contract = HashedTimeLockedContract {
        balance: 1000.try_into().unwrap(),
        sender,
        recipient,
        hash_algorithm: HashAlgorithm::Blake2b,
        hash_root,
        hash_count: 2,
        timeout: 100,
        total_amount: 1000.try_into().unwrap(),
    };

    let tx = Transaction::new_contract_creation(
        vec![],
        Address::from([0u8; 20]),
        AccountType::HTLC,
        AccountType::Basic,
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    let sender_signature = sender_key_pair.sign(&tx.serialize_content()[..]);
    let recipient_signature = recipient_key_pair.sign(&tx.serialize_content()[..]);
    let sender_signature_proof = SignatureProof::from(sender_key_pair.public, sender_signature);
    let recipient_signature_proof =
        SignatureProof::from(recipient_key_pair.public, recipient_signature);

    (
        start_contract,
        tx,
        pre_image,
        sender_signature_proof,
        recipient_signature_proof,
    )
}

#[test]
#[allow(unused_must_use)]
fn it_can_verify_regular_transfer() {
    let (_, mut tx, _, _, recipient_signature_proof) = prepare_outgoing_transaction();

    // regular: valid Blake-2b
    let mut proof =
        Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(
        &AnyHash::from(<[u8; 32]>::from(
            Blake2bHasher::default().digest(&[0u8; 32]),
        )),
        &mut proof,
    );
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // regular: valid SHA-256
    proof = Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Sha256, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(
        &AnyHash::from(<[u8; 32]>::from(Sha256Hasher::default().digest(&[0u8; 32]))),
        &mut proof,
    );
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // regular: invalid hash
    let bak = tx.proof[35];
    tx.proof[35] = bak % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
    tx.proof[35] = bak;

    // regular: invalid algorithm
    tx.proof[1] = 99;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            SerializingError::InvalidValue
        ))
    );
    tx.proof[1] = HashAlgorithm::Sha256 as u8;

    // regular: invalid signature
    // Proof is not a valid point, so Deserialize will result in an error.
    tx.proof[72] = tx.proof[72] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            SerializingError::InvalidValue
        ))
    );

    // regular: invalid signature
    tx.proof[72] = tx.proof[72] % 250 + 2;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    // regular: invalid over-long
    proof = Vec::with_capacity(4 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(
        &AnyHash::from(<[u8; 32]>::from(
            Blake2bHasher::default().digest(&[0u8; 32]),
        )),
        &mut proof,
    );
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    Serialize::serialize(&0u8, &mut proof);
    tx.proof = proof;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
#[allow(unused_must_use)]
fn it_can_verify_early_resolve() {
    let (_, mut tx, _, sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // early resolve: valid
    let mut proof = Vec::with_capacity(
        1 + recipient_signature_proof.serialized_size() + sender_signature_proof.serialized_size(),
    );
    Serialize::serialize(&ProofType::EarlyResolve, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    tx.proof = proof;
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // early resolve: invalid signature 1
    // Proof is not a valid point, so Deserialize will result in an error.
    let bak = tx.proof[4];
    tx.proof[4] = tx.proof[4] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            SerializingError::InvalidValue
        ))
    );
    tx.proof[4] = bak;

    // early resolve: invalid signature 2
    let bak = tx.proof.len() - 2;
    tx.proof[bak] = tx.proof[bak] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    // early resolve: invalid over-long
    proof = Vec::with_capacity(
        2 + recipient_signature_proof.serialized_size() + sender_signature_proof.serialized_size(),
    );
    Serialize::serialize(&ProofType::EarlyResolve, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    Serialize::serialize(&0u8, &mut proof);
    tx.proof = proof;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
#[allow(unused_must_use)]
fn it_can_verify_timeout_resolve() {
    let (_, mut tx, _, sender_signature_proof, _) = prepare_outgoing_transaction();

    // timeout resolve: valid
    let mut proof = Vec::with_capacity(1 + sender_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    tx.proof = proof;
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // timeout resolve: invalid signature
    tx.proof[4] = tx.proof[4] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    // timeout resolve: invalid over-long
    proof = Vec::with_capacity(2 + sender_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    Serialize::serialize(&0u8, &mut proof);
    tx.proof = proof;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
}

#[test]
#[allow(unused_must_use)]
fn it_can_apply_and_revert_valid_transaction() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    let (start_contract, mut tx, pre_image, sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // regular transfer
    let mut proof =
        Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&2u8, &mut proof);
    Serialize::serialize(&start_contract.hash_root, &mut proof);
    Serialize::serialize(&pre_image, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[0u8; 20][..]),
        Account::HTLC(start_contract.clone()),
    );

    HashedTimeLockedContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 1)
        .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[0u8; 20][..]))
            .unwrap()
            .balance(),
        0.try_into().unwrap()
    );
    HashedTimeLockedContract::revert_outgoing_transaction(
        &accounts_tree,
        &mut db_txn,
        &tx,
        1,
        1,
        None,
    )
    .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[0u8; 20][..]))
            .unwrap(),
        Account::HTLC(start_contract.clone())
    );

    // early resolve
    let mut proof = Vec::with_capacity(
        1 + recipient_signature_proof.serialized_size() + sender_signature_proof.serialized_size(),
    );
    Serialize::serialize(&ProofType::EarlyResolve, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    tx.proof = proof;

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[0u8; 20][..]),
        Account::HTLC(start_contract.clone()),
    );

    HashedTimeLockedContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 1)
        .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[0u8; 20][..]))
            .unwrap()
            .balance(),
        0.try_into().unwrap()
    );
    HashedTimeLockedContract::revert_outgoing_transaction(
        &accounts_tree,
        &mut db_txn,
        &tx,
        1,
        1,
        None,
    )
    .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[0u8; 20][..]))
            .unwrap(),
        Account::HTLC(start_contract.clone())
    );

    // timeout resolve
    let mut proof = Vec::with_capacity(1 + sender_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    tx.proof = proof;

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[0u8; 20][..]),
        Account::HTLC(start_contract.clone()),
    );

    HashedTimeLockedContract::commit_outgoing_transaction(&accounts_tree, &mut db_txn, &tx, 1, 101)
        .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[0u8; 20][..]))
            .unwrap()
            .balance(),
        0.try_into().unwrap()
    );
    HashedTimeLockedContract::revert_outgoing_transaction(
        &accounts_tree,
        &mut db_txn,
        &tx,
        1,
        1,
        None,
    )
    .unwrap();
    assert_eq!(
        accounts_tree
            .get(&db_txn, &KeyNibbles::from(&[0u8; 20][..]))
            .unwrap(),
        Account::HTLC(start_contract)
    );
}

#[test]
#[allow(unused_must_use)]
fn it_refuses_invalid_transaction() {
    let env = VolatileEnvironment::new(10).unwrap();
    let accounts_tree = AccountsTrie::new(env.clone(), "AccountsTree");
    let mut db_txn = WriteTransaction::new(&env);

    let (start_contract, mut tx, pre_image, sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    accounts_tree.put(
        &mut db_txn,
        &KeyNibbles::from(&[0u8; 20][..]),
        Account::HTLC(start_contract.clone()),
    );

    // regular transfer: timeout passed
    let mut proof =
        Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&2u8, &mut proof);
    Serialize::serialize(&start_contract.hash_root, &mut proof);
    Serialize::serialize(&pre_image, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            101
        ),
        Err(AccountError::InvalidForSender)
    );

    // regular transfer: hash mismatch
    let mut proof =
        Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&2u8, &mut proof);
    Serialize::serialize(&AnyHash::from([1u8; 32]), &mut proof);
    Serialize::serialize(&pre_image, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            1
        ),
        Err(AccountError::InvalidForSender)
    );

    // regular transfer: invalid signature
    let mut proof =
        Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&2u8, &mut proof);
    Serialize::serialize(&start_contract.hash_root, &mut proof);
    Serialize::serialize(&pre_image, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            1
        ),
        Err(AccountError::InvalidSignature)
    );

    // regular transfer: underflow
    let mut proof =
        Vec::with_capacity(3 + 2 * AnyHash::SIZE + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(&start_contract.hash_root, &mut proof);
    Serialize::serialize(
        &AnyHash::from(<[u8; 32]>::from(
            Blake2bHasher::default().digest(&(<[u8; 32]>::from(pre_image))),
        )),
        &mut proof,
    );
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            1
        ),
        Err(AccountError::InsufficientFunds {
            needed: 500.try_into().unwrap(),
            balance: 0.try_into().unwrap()
        })
    );

    // early resolve: invalid signature
    let mut proof = Vec::with_capacity(
        1 + recipient_signature_proof.serialized_size() + sender_signature_proof.serialized_size(),
    );
    Serialize::serialize(&ProofType::EarlyResolve, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            1
        ),
        Err(AccountError::InvalidSignature)
    );

    // timeout resolve: timeout not expired
    let mut proof = Vec::with_capacity(1 + sender_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&sender_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            1
        ),
        Err(AccountError::InvalidForSender)
    );

    // timeout resolve: invalid signature
    let mut proof = Vec::with_capacity(1 + recipient_signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&recipient_signature_proof, &mut proof);
    tx.proof = proof;

    assert_eq!(
        HashedTimeLockedContract::commit_outgoing_transaction(
            &accounts_tree,
            &mut db_txn,
            &tx,
            1,
            101
        ),
        Err(AccountError::InvalidSignature)
    );
}
