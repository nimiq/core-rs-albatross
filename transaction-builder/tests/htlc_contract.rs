use std::convert::TryInto;

use nimiq_hash::{Blake2bHash, Blake2bHasher, HashOutput, Hasher};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_transaction::{
    account::htlc_contract::{
        AnyHash, CreationTransactionData, HashAlgorithm, OutgoingHTLCTransactionProof,
    },
    SignatureProof, Transaction,
};
use nimiq_transaction_builder::{Recipient, TransactionBuilder};

#[test]
fn it_can_create_creation_transaction() {
    let sender = Address::from([0u8; 20]);
    let recipient = Address::from([0u8; 20]);

    let data = CreationTransactionData {
        sender: sender.clone(),
        recipient: recipient.clone(),
        hash_algorithm: HashAlgorithm::Blake2b,
        hash_root: AnyHash::from([0u8; 32]),
        hash_count: 2,
        timeout: 1000,
    };

    let transaction = Transaction::new_contract_creation(
        data.serialize_to_vec(),
        sender.clone(),
        AccountType::Basic,
        AccountType::HTLC,
        100.try_into().unwrap(),
        0.try_into().unwrap(),
        0,
        NetworkId::Dummy,
    );

    let mut htlc_builder = Recipient::new_htlc_builder();
    htlc_builder
        .with_sender(sender.clone())
        .with_recipient(recipient)
        .with_blake2b_hash(Blake2bHash::from([0u8; 32]), 2)
        .with_timeout(1000);

    let mut builder = TransactionBuilder::new();
    builder
        .with_sender(sender)
        .with_recipient(htlc_builder.generate().unwrap())
        .with_value(100.try_into().unwrap())
        .with_validity_start_height(0)
        .with_network_id(NetworkId::Dummy);
    let result = builder
        .generate()
        .expect("Builder should be able to create transaction");
    let result = result.unwrap_basic();

    assert_eq!(result.transaction, transaction);
}

fn prepare_outgoing_transaction() -> (
    Transaction,
    AnyHash,
    AnyHash,
    KeyPair,
    SignatureProof,
    KeyPair,
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
    let pre_image = AnyHash::from([1u8; 32]);
    let hash_root = AnyHash::from(<[u8; 32]>::from(
        Blake2bHasher::default().digest(
            Blake2bHasher::default()
                .digest(pre_image.as_bytes())
                .as_bytes(),
        ),
    ));

    let tx = Transaction::new_extended(
        Address::from([0u8; 20]),
        AccountType::HTLC,
        Address::from([1u8; 20]),
        AccountType::Basic,
        1000.try_into().unwrap(),
        0.try_into().unwrap(),
        vec![],
        1,
        NetworkId::Dummy,
    );

    let sender_signature = sender_key_pair.sign(&tx.serialize_content()[..]);
    let recipient_signature = recipient_key_pair.sign(&tx.serialize_content()[..]);
    let sender_signature_proof = SignatureProof::from(sender_key_pair.public, sender_signature);
    let recipient_signature_proof =
        SignatureProof::from(recipient_key_pair.public, recipient_signature);

    (
        tx,
        pre_image,
        hash_root,
        sender_key_pair,
        sender_signature_proof,
        recipient_key_pair,
        recipient_signature_proof,
    )
}

#[test]
fn it_can_create_regular_transfer() {
    let (mut tx, pre_image, hash_root, _, _, recipient_key_pair, recipient_signature_proof) =
        prepare_outgoing_transaction();

    // regular: valid Blake-2b
    let proof = OutgoingHTLCTransactionProof::RegularTransfer {
        hash_algorithm: HashAlgorithm::Blake2b,
        hash_depth: 1,
        hash_root: hash_root.clone(),
        pre_image: pre_image.clone(),
        signature_proof: recipient_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let mut builder = TransactionBuilder::new();
    builder
        .with_sender(Address::from([0u8; 20]))
        .with_sender_type(AccountType::HTLC)
        .with_recipient(Recipient::new_basic(Address::from([1u8; 20])))
        .with_value(1000.try_into().unwrap())
        .with_fee(0.try_into().unwrap())
        .with_validity_start_height(1)
        .with_network_id(NetworkId::Dummy);
    let proof_builder = builder
        .generate()
        .expect("Builder should be able to create transaction");
    let mut proof_builder = proof_builder.unwrap_htlc();
    let proof = proof_builder.signature_with_key_pair(&recipient_key_pair);
    proof_builder.regular_transfer(HashAlgorithm::Blake2b, pre_image, 1, hash_root, proof);
    let tx2 = proof_builder
        .generate()
        .expect("Builder should be able to create proof");

    assert_eq!(tx2, tx);
}

#[test]
fn it_can_create_early_resolve() {
    let (
        mut tx,
        _,
        _,
        sender_key_pair,
        sender_signature_proof,
        recipient_key_pair,
        recipient_signature_proof,
    ) = prepare_outgoing_transaction();

    // early resolve: valid
    let proof = OutgoingHTLCTransactionProof::EarlyResolve {
        signature_proof_recipient: recipient_signature_proof,
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let mut builder = TransactionBuilder::new();
    builder
        .with_sender(Address::from([0u8; 20]))
        .with_sender_type(AccountType::HTLC)
        .with_recipient(Recipient::new_basic(Address::from([1u8; 20])))
        .with_value(1000.try_into().unwrap())
        .with_fee(0.try_into().unwrap())
        .with_validity_start_height(1)
        .with_network_id(NetworkId::Dummy);
    let proof_builder = builder
        .generate()
        .expect("Builder should be able to create transaction");
    let mut proof_builder = proof_builder.unwrap_htlc();
    let sender_proof = proof_builder.signature_with_key_pair(&sender_key_pair);
    let recipient_proof = proof_builder.signature_with_key_pair(&recipient_key_pair);
    proof_builder.early_resolve(sender_proof, recipient_proof);
    let tx2 = proof_builder
        .generate()
        .expect("Builder should be able to create proof");

    assert_eq!(tx2, tx);
}

#[test]
fn it_can_create_timeout_resolve() {
    let (mut tx, _, _, sender_key_pair, sender_signature_proof, _, _) =
        prepare_outgoing_transaction();

    // timeout resolve: valid
    let proof = OutgoingHTLCTransactionProof::TimeoutResolve {
        signature_proof_sender: sender_signature_proof,
    };
    tx.proof = proof.serialize_to_vec();

    let mut builder = TransactionBuilder::new();
    builder
        .with_sender(Address::from([0u8; 20]))
        .with_sender_type(AccountType::HTLC)
        .with_recipient(Recipient::new_basic(Address::from([1u8; 20])))
        .with_value(1000.try_into().unwrap())
        .with_fee(0.try_into().unwrap())
        .with_validity_start_height(1)
        .with_network_id(NetworkId::Dummy);
    let proof_builder = builder
        .generate()
        .expect("Builder should be able to create transaction");
    let mut proof_builder = proof_builder.unwrap_htlc();
    let proof = proof_builder.signature_with_key_pair(&sender_key_pair);
    proof_builder.timeout_resolve(proof);
    let tx2 = proof_builder
        .generate()
        .expect("Builder should be able to create proof");

    assert_eq!(tx2, tx);
}
