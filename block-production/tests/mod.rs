use std::sync::Arc;

use beserial::Serialize;
use nimiq_account::AccountType;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, PushResult};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_mempool::{Mempool, MempoolConfig, ReturnCode};
use nimiq_network_primitives::{networks::NetworkId, time::NetworkTime};
use nimiq_primitives::coin::Coin;
use nimiq_transaction::{SignatureProof, Transaction};

#[test]
fn it_can_produce_empty_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());

    let keypair: KeyPair = PrivateKey::from([1u8; PrivateKey::SIZE]).into();
    let miner = Address::from(&keypair.public);

    let producer = BlockProducer::new(blockchain.clone(), mempool);
    let mut block = producer.next_block(1523727060, miner.clone(), Vec::new());
    block.header.nonce = 34932;
    assert_eq!(blockchain.push(block), PushResult::Extended);

    let mut block = producer.next_block(1523727120, miner, Vec::new());
    block.header.nonce = 5648;
    assert_eq!(blockchain.push(block), PushResult::Extended);
}

#[test]
fn it_can_produce_nonempty_blocks() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap());
    let mempool = Mempool::new(blockchain.clone(), MempoolConfig::default());

    let keypair: KeyPair = PrivateKey::from([1u8; PrivateKey::SIZE]).into();
    let miner = Address::from(&keypair.public);

    let producer = BlockProducer::new(Arc::clone(&blockchain), Arc::clone(&mempool));
    let mut block = producer.next_block(1523727060, miner.clone(), Vec::new());
    block.header.nonce = 34932;
    assert_eq!(blockchain.push(block), PushResult::Extended);

    // Create vesting contract
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE + 4);
    Serialize::serialize(&miner, &mut data).unwrap();
    Serialize::serialize(&3u32, &mut data).unwrap();
    let mut tx = Transaction::new_contract_creation(
        data,
        miner.clone(),
        AccountType::Basic,
        AccountType::Vesting,
        Coin::from_u64(100).unwrap(),
        Coin::from_u64(0).unwrap(),
        0,
        NetworkId::Main,
    );
    let contract_address = tx.recipient.clone();
    let signature = keypair.sign(tx.serialize_content().as_slice());
    tx.proof = SignatureProof::from(keypair.public, signature).serialize_to_vec();
    assert_eq!(mempool.push_transaction(tx), ReturnCode::Accepted);

    let mut block = producer.next_block(1523727120, miner.clone(), Vec::new());
    block.header.nonce = 30367;
    assert_eq!(blockchain.push(block), PushResult::Extended);

    let contract = blockchain.state().accounts().get(&contract_address, None);
    assert_eq!(contract.account_type(), AccountType::Vesting);
    assert_eq!(contract.balance(), Coin::from_u64(100).unwrap());

    // Prune vesting contract with 2 transactions
    let mut tx = Transaction::new_basic(
        contract_address.clone(),
        miner.clone(),
        Coin::from_u64(49).unwrap(),
        Coin::ZERO,
        0,
        NetworkId::Main,
    );
    tx.sender_type = AccountType::Vesting;
    let signature = keypair.sign(tx.serialize_content().as_slice());
    tx.proof = SignatureProof::from(keypair.public, signature).serialize_to_vec();
    assert_eq!(mempool.push_transaction(tx), ReturnCode::Accepted);

    let mut tx = Transaction::new_basic(
        contract_address.clone(),
        miner.clone(),
        Coin::from_u64(51).unwrap(),
        Coin::ZERO,
        0,
        NetworkId::Main,
    );
    tx.sender_type = AccountType::Vesting;
    let signature = keypair.sign(tx.serialize_content().as_slice());
    tx.proof = SignatureProof::from(keypair.public, signature).serialize_to_vec();
    assert_eq!(mempool.push_transaction(tx), ReturnCode::Accepted);

    let mut block = producer.next_block(1523727180, miner.clone(), Vec::new());
    block.header.nonce = 11673;
    assert_eq!(blockchain.push(block), PushResult::Extended);

    let contract = blockchain.state().accounts().get(&contract_address, None);
    assert_eq!(contract.account_type(), AccountType::Basic);
    assert_eq!(contract.balance(), Coin::ZERO);
}
