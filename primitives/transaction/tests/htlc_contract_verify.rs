use nimiq_hash::{sha512::Sha512Hasher, Blake2bHasher, Hasher, Sha256Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, networks::NetworkId, transaction::TransactionError};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use nimiq_transaction::{
    account::{
        htlc_contract::{
            AnyHash, AnyHash32, AnyHash64, CreationTransactionData, OutgoingHTLCTransactionProof,
            PreImage,
        },
        AccountTransactionVerification,
    },
    SignatureProof, Transaction, TransactionFlags,
};

fn prepare_outgoing_transaction() -> (Transaction, AnyHash, SignatureProof, SignatureProof) {
    let sender_priv_key = PrivateKey::deserialize_from_vec(
        &hex::decode("9d5bd02379e7e45cf515c788048f5cf3c454ffabd3e83bd1d7667716c325c3c0").unwrap(),
    )
    .unwrap();
    let recipient_priv_key = PrivateKey::deserialize_from_vec(
        &hex::decode("bd1cfcd49a81048c8c8d22a25766bd01bfa0f6b2eb0030f65241189393af96a2").unwrap(),
    )
    .unwrap();

    let sender_key_pair = KeyPair::from(sender_priv_key);
    let recipient_key_pair = KeyPair::from(recipient_priv_key);
    let pre_image = AnyHash::Blake2b(AnyHash32::from([1u8; 32]));

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
    let sender_signature_proof =
        SignatureProof::from_ed25519(sender_key_pair.public, sender_signature);
    let recipient_signature_proof =
        SignatureProof::from_ed25519(recipient_key_pair.public, recipient_signature);

    (
        tx,
        pre_image,
        sender_signature_proof,
        recipient_signature_proof,
    )
}

#[test]
fn it_can_verify_creation_transaction() {
    let data = CreationTransactionData {
        sender: Address::from([0u8; 20]),
        recipient: Address::from([0u8; 20]),
        hash_root: AnyHash::Blake2b(AnyHash32::from([0u8; 32])),
        hash_count: 2,
        timeout: 1000,
    };

    let mut transaction = Transaction::new_contract_creation(
        data.sender.clone(),
        AccountType::Basic,
        vec![],
        AccountType::HTLC,
        vec![],
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::UnitAlbatross,
    );

    // Invalid data
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidData)
    );
    transaction.recipient_data = data.serialize_to_vec();

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
    transaction.recipient_data[40] = 200;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidSerialization(
            DeserializeError::serde_custom()
        ))
    );
    transaction.recipient_data[40] = 1;

    // Invalid zero hash count
    transaction.recipient_data[73] = 0;
    transaction.recipient = transaction.contract_creation_address();
    assert_eq!(
        AccountType::verify_incoming_transaction(&transaction),
        Err(TransactionError::InvalidData)
    );
}

#[test]
fn it_can_verify_regular_transfer() {
    let (mut tx, _, _, recipient_signature_proof) = prepare_outgoing_transaction();

    // regular: valid Blake-2b
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 1,
        hash_root: AnyHash::from(Blake2bHasher::default().digest(&[0u8; 32])),
        pre_image: PreImage::PreImage32(AnyHash32::from([0u8; 32])),
        signature_proof: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // regular: valid SHA-512
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 1,
        hash_root: AnyHash::from(Sha512Hasher::default().digest(&[0u8; 64])),
        pre_image: PreImage::PreImage64(AnyHash64::from([0u8; 64])),
        signature_proof: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // regular: valid SHA-256
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 1,
        hash_root: AnyHash::from(Sha256Hasher::default().digest(&[0u8; 32])),
        pre_image: PreImage::PreImage32(AnyHash32::from([0u8; 32])),
        signature_proof: recipient_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();
    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // regular: invalid hash
    let bak = tx.proof[36];
    tx.proof[36] = bak % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );
    tx.proof[36] = bak;

    // regular: invalid algorithm
    tx.proof[2] = 99;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            DeserializeError::serde_custom()
        ))
    );
    tx.proof[2] = 3_u8; // 3 -> Sha256

    // regular: invalid signature
    // Proof is not a valid point, so Deserialize will result in an error.
    tx.proof[73] = tx.proof[73] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    // regular: invalid signature
    tx.proof[73] = tx.proof[73] % 250 + 2;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    // regular: invalid over-long
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_depth: 1,
        hash_root: AnyHash::from(Blake2bHasher::default().digest(&[0u8; 32])),
        pre_image: PreImage::PreImage32(AnyHash32::from([0u8; 32])),
        signature_proof: recipient_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();
    tx.proof.push(0);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            DeserializeError::extra_data()
        )),
    );
}

#[test]
fn it_can_verify_early_resolve() {
    let (mut tx, _, sender_signature_proof, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // early resolve: valid
    let proof = OutgoingHTLCTransactionProof::EarlyResolve {
        signature_proof_recipient: recipient_signature_proof.clone(),
        signature_proof_sender: sender_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // early resolve: invalid signature 1
    // Proof is not a valid point, so Deserialize will result in an error.
    let bak = tx.proof[4];
    tx.proof[4] = tx.proof[4] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
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
    let proof = OutgoingHTLCTransactionProof::EarlyResolve {
        signature_proof_recipient: recipient_signature_proof,
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();
    tx.proof.push(0);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            DeserializeError::extra_data()
        )),
    );
}

#[test]
fn it_can_verify_timeout_resolve() {
    let (mut tx, _, sender_signature_proof, _) = prepare_outgoing_transaction();

    // timeout resolve: valid
    let proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: sender_signature_proof.clone(),
    };
    tx.proof = proof.serialize_to_vec();

    assert_eq!(AccountType::verify_outgoing_transaction(&tx), Ok(()));

    // timeout resolve: invalid signature
    tx.proof[4] = tx.proof[4] % 250 + 1;
    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidProof)
    );

    // timeout resolve: invalid over-long
    let proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();
    tx.proof.push(0);

    assert_eq!(
        AccountType::verify_outgoing_transaction(&tx),
        Err(TransactionError::InvalidSerialization(
            DeserializeError::extra_data()
        )),
    );
}
