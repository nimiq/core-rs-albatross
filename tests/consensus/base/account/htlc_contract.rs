use beserial::{Serialize, Deserialize};
use nimiq::consensus::base::account::{AccountType, HashedTimeLockedContract};
use nimiq::consensus::base::account::htlc_contract::{AnyHash, ProofType, HashAlgorithm};
use nimiq::consensus::base::primitive::{Address, Coin, crypto::KeyPair, hash::{Blake2bHasher, Hasher}};
use nimiq::consensus::base::transaction::{SignatureProof, Transaction, TransactionFlags};
use nimiq::consensus::networks::NetworkId;

const HTLC: &str = "00000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f01000296350000000000000001";

#[test]
fn it_can_deserialize_a_htlc() {
    let bytes: Vec<u8> = hex::decode(HTLC).unwrap();
    let htlc: HashedTimeLockedContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    assert_eq!(htlc.balance, Coin::ZERO);
    assert_eq!(htlc.hash_algorithm, HashAlgorithm::Sha256);
    assert_eq!(htlc.hash_count, 1);
    assert_eq!(htlc.hash_root, AnyHash::from("daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f"));
    assert_eq!(htlc.sender, Address::from("1b215589344cf570d36bec770825eae30b732139"));
    assert_eq!(htlc.recipient, Address::from("24786862babbdb05e7c4430612135eb2a8368123"));
    assert_eq!(htlc.timeout, 169525);
    assert_eq!(htlc.total_amount, Coin::from(1));
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
        Err(_) => assert!(false)
    }
}

#[test]
fn it_can_verify_valid_outgoing_transaction() {
    let key_pair = KeyPair::generate();
    let addr = Address::from(&key_pair.public);
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
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
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
