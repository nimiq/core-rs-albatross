use beserial::Serialize;
use nimiq::consensus::base::account::{Account, AccountType, HashedTimeLockedContract};
use nimiq::consensus::base::account::htlc_contract::{AnyHash, ProofType};
use nimiq::consensus::base::primitive::{Address, coin::Coin, crypto::{KeyPair, Signature}, hash::{Blake2bHash, Blake2bHasher, HashAlgorithm, Hasher, HashOutput}};
use nimiq::consensus::base::transaction::{SignatureProof, Transaction, TransactionFlags};
use nimiq::consensus::networks::NetworkId;

#[test]
fn it_can_create_contract_from_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE * 2 + AnyHash::SIZE + 6);
    let sender = Address::from([0u8; 20]);
    let recipient = Address::from([0u8; 20]);
    sender.serialize(&mut data);
    recipient.serialize(&mut data);
    HashAlgorithm::Blake2b.serialize(&mut data);
    AnyHash::from([0u8; 32]).serialize(&mut data);
    Serialize::serialize(&2u8, &mut data);
    Serialize::serialize(&1000u32, &mut data);
    println!("{}", hex::encode(&data));
    let transaction = Transaction::new_contract_creation(
        data,
        sender.clone(),
        AccountType::Basic,
        AccountType::HTLC,
        Coin::from(100),
        Coin::from(0),
        0,
        NetworkId::Dummy,
    );
    assert!(HashedTimeLockedContract::verify_incoming_transaction(&transaction));
    match HashedTimeLockedContract::create(Coin::from(0), &transaction, 0) {
        Ok(htlc) => {
            assert_eq!(htlc.balance, Coin::from(100));
            assert_eq!(htlc.sender, sender);
            assert_eq!(htlc.recipient, recipient);
            assert_eq!(htlc.hash_root, AnyHash::from([0u8; 32]));
            assert_eq!(htlc.hash_count, 2);
            assert_eq!(htlc.timeout, 1000);
        }
        Err(e) => assert!(false)
    }
}

#[test]
fn it_can_verify_valid_outgoing_transaction() {
    let keyPair = KeyPair::generate();
    let addr = Address::from(&keyPair.public);
    let mut tx = Transaction {
        data: vec![],
        sender: Address::from([0u8; 20]),
        sender_type: AccountType::HTLC,
        recipient: Address::from([0u8; 20]),
        recipient_type: AccountType::Basic,
        value: Coin::from(100),
        fee: Coin::from(0),
        validity_start_height: 1,
        network_id: NetworkId::Dummy,
        flags: TransactionFlags::empty(),
        proof: vec![],
    };

    // regular
    let signature = keyPair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(keyPair.public, signature);
    let mut proof = Vec::with_capacity(3 + 2 * AnyHash::SIZE + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(&AnyHash::from(<[u8; 32]>::from(Blake2bHasher::default().digest(&[0u8; 32]))), &mut proof);
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // early resolve
    proof = Vec::with_capacity(1 + 2 * signature_proof.serialized_size());
    Serialize::serialize(&ProofType::EarlyResolve, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // timeout resolve
    proof = Vec::with_capacity(1 + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));
}
