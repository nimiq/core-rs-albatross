use std::collections::HashSet;
use std::convert::TryFrom;
use std::sync::Arc;

use beserial::Serialize;
use nimiq_blockchain::{Blockchain, PushResult};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::Hash;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_network_primitives::time::NetworkTime;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::{SignatureProof, Transaction};

#[test]
fn it_can_compute_trivial_transactions_proof() {
    let keypair: KeyPair = PrivateKey::from([1u8; PrivateKey::SIZE]).into();

    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(&env, NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let miner = Address::from(&keypair.public);
    let block2 = crate::next_block(&blockchain)
        .with_miner(miner.clone())
        .with_nonce(34932)
        .build();

    let mut status = blockchain.push(block2);
    assert_eq!(status, Ok(PushResult::Extended));

    // Push block 3 containing a tx.
    let mut tx = Transaction::new_basic(
        miner.clone(),
        [2u8; Address::SIZE].into(),
        Coin::try_from(10).unwrap(),
        Coin::try_from(0).unwrap(),
        1,
        NetworkId::Main
    );
    tx.proof = SignatureProof::from(keypair.public.clone(), keypair.sign(&tx.serialize_content())).serialize_to_vec();

    let block3 = crate::next_block(&blockchain)
        .with_miner(miner.clone())
        .with_transactions(vec![tx.clone()])
        .with_nonce(23026)
        .build();
    let body_hash = block3.header.body_hash.clone();
    let block_hash = block3.header.hash();
    status = blockchain.push(block3);
    assert_eq!(status, Ok(PushResult::Extended));

    // Generate transactions proof.
    let mut addresses = HashSet::new();
    addresses.insert(miner.clone());
    let transactions_proof = blockchain.get_transactions_proof(
        &block_hash,
        &addresses,
    ).unwrap();

    let root_hash = transactions_proof.proof
        .compute_root_from_values(&transactions_proof.transactions[..]).unwrap();
    assert_eq!(root_hash, body_hash);
}
