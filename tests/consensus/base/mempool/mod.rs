use beserial::{Serialize, Deserialize};
use hex;
use parking_lot::RwLock;
use std::sync::Arc;

use nimiq::consensus::base::primitive::{Address, Coin};
use nimiq::consensus::base::primitive::hash::Hash;
use nimiq::consensus::base::primitive::crypto::KeyPair;
use nimiq::consensus::base::block::BlockBody;
use nimiq::consensus::base::mempool::{Mempool, ReturnCode};
use nimiq::consensus::base::transaction::{Transaction, SignatureProof};
use nimiq::consensus::networks::NetworkId;
use nimiq::utils::db::WriteTransaction;
use nimiq::utils::db::volatile::VolatileEnvironment;
use nimiq::consensus::base::blockchain::Blockchain;
use nimiq::network::NetworkTime;

const BASIC_TRANSACTION: &str = "000222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0754d1260f15bea0e8fb07ab18f45301483599e34000000000000c350000000000000008a00019640023fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30a";

#[test]
fn push_same_tx_twice() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(NetworkTime::new(0)), NetworkId::Main));
    let mempool = Mempool::new(blockchain.clone());

    let keypair_a = KeyPair::generate();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    // Give address_a balance
    let body = BlockBody { miner: address_a.clone(), extra_data: Vec::new(), transactions: Vec::new(), pruned_accounts: Vec::new() };
    let mut txn = WriteTransaction::new(&env);
    blockchain.accounts().commit_block_body(&mut txn, &body, 1).unwrap();
    txn.commit();

    // Generate and sign transaction from address_a
    let mut tx = Transaction::new_basic( address_a.clone(), address_b.clone(), Coin::from(10), Coin::from(0), 1, NetworkId::Main );
    let signature_proof = SignatureProof::from(keypair_a.public.clone(), keypair_a.sign(&tx.serialize_content()));
    tx.proof = signature_proof.serialize_to_vec();

    assert_eq!(mempool.push_transaction(tx), ReturnCode::Accepted);
}

#[test]
fn push_tx_with_wrong_signature() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(NetworkTime::new(0)), NetworkId::Main));
    let mempool = Mempool::new(blockchain);

    let v: Vec<u8> = hex::decode(BASIC_TRANSACTION).unwrap();
    let mut t: Transaction = Deserialize::deserialize(&mut &v[..]).unwrap();
    t.proof = hex::decode("0222666efadc937148a6d61589ce6d4aeecca97fda4c32348d294eab582f14a0003fecb82d3aef4be76853d5c5b263754b7d495d9838f6ae5df60cf3addd3512a82988db0056059c7a52ae15285983ef0db8229ae446c004559147686d28f0a30b").unwrap(); // last char a (valid) -> b
    assert_eq!(mempool.push_transaction(t), ReturnCode::Invalid);
}

#[test]
fn push_tx_with_insufficient_balance() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(NetworkTime::new(0)), NetworkId::Main));
    let mempool = Mempool::new(blockchain);

    let v: Vec<u8> = hex::decode(BASIC_TRANSACTION).unwrap();
    let t: Transaction = Deserialize::deserialize(&mut &v[..]).unwrap();

    assert_eq!(mempool.push_transaction(t), ReturnCode::Invalid);
}

#[test]
fn push_and_get_valid_tx() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(NetworkTime::new(0)), NetworkId::Main));
    let mempool = Mempool::new(blockchain.clone());

    let keypair_a = KeyPair::generate();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    // Give address_a balance
    let body = BlockBody { miner: address_a.clone(), extra_data: Vec::new(), transactions: Vec::new(), pruned_accounts: Vec::new() };
    let mut txn = WriteTransaction::new(&env);
    blockchain.accounts().commit_block_body(&mut txn, &body, 1).unwrap();
    txn.commit();

    // Generate and sign transaction from address_a
    let mut tx = Transaction::new_basic( address_a.clone(), address_b.clone(), Coin::from(10), Coin::from(0), 1, NetworkId::Main );
    let signature_proof = SignatureProof::from(keypair_a.public.clone(), keypair_a.sign(&tx.serialize_content()));
    tx.proof = signature_proof.serialize_to_vec();
    let tx_copy = tx.clone();
    let hash = tx.hash();

    assert_eq!(mempool.push_transaction(tx), ReturnCode::Accepted);

    let t2 = mempool.get_transaction(&hash);
    assert!(t2.is_some());
    assert_eq!(Arc::new(tx_copy), t2.unwrap());
}

#[test]
fn push_and_get_two_tx_same_user() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(NetworkTime::new(0)), NetworkId::Main));
    let mempool = Mempool::new(blockchain.clone());

    let keypair_a = KeyPair::generate();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    // Give address_a balance
    let body = BlockBody { miner: address_a.clone(), extra_data: Vec::new(), transactions: Vec::new(), pruned_accounts: Vec::new() };
    let mut txn = WriteTransaction::new(&env);
    blockchain.accounts().commit_block_body(&mut txn, &body, 1).unwrap();
    txn.commit();

    // Generate, sign and push 1st transaction from address_a
    let mut tx1 = Transaction::new_basic( address_a.clone(), address_b.clone(), Coin::from(10), Coin::from(0), 1, NetworkId::Main );
    let signature_proof1 = SignatureProof::from(keypair_a.public.clone(), keypair_a.sign(&tx1.serialize_content()));
    tx1.proof = signature_proof1.serialize_to_vec();
    let tx1_copy = tx1.clone();
    let hash1 = tx1.hash();
    assert_eq!(mempool.push_transaction(tx1), ReturnCode::Accepted);

    // Generate, sign and push 2nd transaction from address_a
    let mut tx2 = Transaction::new_basic( address_a.clone(), address_b.clone(), Coin::from(9), Coin::from(0), 1, NetworkId::Main );
    let signature_proof2 = SignatureProof::from(keypair_a.public.clone(), keypair_a.sign(&tx2.serialize_content()));
    tx2.proof = signature_proof2.serialize_to_vec();
    let tx2_copy = tx2.clone();
    let hash2 = tx2.hash();
    assert_eq!(mempool.push_transaction(tx2), ReturnCode::Accepted);

    assert_eq!(Arc::new(tx1_copy), mempool.get_transaction(&hash1).unwrap());
    assert_eq!(Arc::new(tx2_copy), mempool.get_transaction(&hash2).unwrap());
}

#[test]
fn reject_free_tx_beyond_limit() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, Arc::new(NetworkTime::new(0)), NetworkId::Main));
    let mempool = Mempool::new(blockchain.clone());

    let keypair_a = KeyPair::generate();
    let address_a = Address::from(&keypair_a.public);
    let address_b = Address::from([2u8; Address::SIZE]);

    // Give address_a balance
    let body = BlockBody { miner: address_a.clone(), extra_data: Vec::new(), transactions: Vec::new(), pruned_accounts: Vec::new() };
    let mut txn = WriteTransaction::new(&env);
    blockchain.accounts().commit_block_body(&mut txn, &body, 1).unwrap();
    txn.commit();

    for i in 0..10 + 1 {
        let mut tx1 = Transaction::new_basic( address_a.clone(), address_b.clone(), Coin::from(1 + i), Coin::from(0), 1, NetworkId::Main );
        let signature_proof1 = SignatureProof::from(keypair_a.public.clone(), keypair_a.sign(&tx1.serialize_content()));
        tx1.proof = signature_proof1.serialize_to_vec();
        if i < 10 {
            assert_eq!(mempool.push_transaction(tx1), ReturnCode::Accepted);
        } else {
            assert_eq!(mempool.push_transaction(tx1), ReturnCode::FeeTooLow);
        }
    }
}
