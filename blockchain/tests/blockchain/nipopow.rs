use std::sync::Arc;

use nimiq_blockchain::{Blockchain, PushResult};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_primitives::time::NetworkTime;
use nimiq_primitives::networks::NetworkId;

#[test]
fn it_can_compute_trivial_chain_proofs() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain =
        Blockchain::new(env.clone(), NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let proof = blockchain.get_chain_proof();
    assert_eq!(proof.prefix.len(), 1);
    assert_eq!(
        proof.prefix[0].header.hash::<Blake2bHash>(),
        blockchain.head_hash()
    );
    assert!(proof.suffix.is_empty());

    let block = crate::next_block(&blockchain).with_nonce(83054).build();
    assert_eq!(blockchain.push(block.clone()), Ok(PushResult::Extended));

    let proof = blockchain.get_chain_proof();
    assert_eq!(proof.prefix.len(), 1);
    assert_eq!(proof.prefix[0].header.height, 1);
    assert_eq!(proof.suffix.len(), 1);
    assert_eq!(
        proof.suffix[0].hash::<Blake2bHash>(),
        blockchain.head_hash()
    );

    let block = crate::next_block(&blockchain).with_nonce(23192).build();
    assert_eq!(blockchain.push(block.clone()), Ok(PushResult::Extended));

    let proof = blockchain.get_chain_proof();
    assert_eq!(proof.prefix.len(), 1);
    assert_eq!(proof.prefix[0].header.height, 1);
    assert_eq!(proof.suffix.len(), 2);
    assert_eq!(proof.suffix[0].height, 2);
    assert_eq!(
        proof.suffix[1].hash::<Blake2bHash>(),
        blockchain.head_hash()
    );
}

#[test]
fn it_can_compute_trivial_block_proofs() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain =
        Blockchain::new(env.clone(), NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let block = crate::next_block(&blockchain).with_nonce(83054).build();
    let hash_to_prove1 = block.header.hash::<Blake2bHash>();
    assert_eq!(blockchain.push(block), Ok(PushResult::Extended));

    let block = crate::next_block(&blockchain).with_nonce(23192).build();
    let hash_to_prove2 = block.header.hash::<Blake2bHash>();
    assert_eq!(blockchain.push(block), Ok(PushResult::Extended));

    let block = crate::next_block(&blockchain).with_nonce(39719).build();
    let known_hash1 = block.header.hash::<Blake2bHash>();
    assert_eq!(blockchain.push(block), Ok(PushResult::Extended));

    let proof = blockchain
        .get_block_proof(&hash_to_prove1, &known_hash1)
        .unwrap();
    assert_eq!(proof.len(), 1);
    assert_eq!(proof[0].header.hash::<Blake2bHash>(), hash_to_prove1);
    assert_eq!(proof[0].header.height, 2);

    let block = crate::next_block(&blockchain).with_nonce(1644).build();
    let known_hash2 = block.header.hash::<Blake2bHash>();
    assert_eq!(blockchain.push(block), Ok(PushResult::Extended));

    let proof = blockchain
        .get_block_proof(&hash_to_prove1, &known_hash2)
        .unwrap();
    assert_eq!(proof.len(), 1);
    assert_eq!(proof[0].header.hash::<Blake2bHash>(), hash_to_prove1);
    assert_eq!(proof[0].header.height, 2);

    let proof = blockchain
        .get_block_proof(&hash_to_prove2, &known_hash2)
        .unwrap();
    assert_eq!(proof.len(), 2);
    assert_eq!(proof[0].header.hash::<Blake2bHash>(), hash_to_prove2);
    assert_eq!(proof[0].header.height, 3);
    assert_eq!(proof[1].header.hash::<Blake2bHash>(), known_hash1);
    assert_eq!(proof[1].header.height, 4);
}

#[test]
fn it_can_compute_empty_block_proofs() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain =
        Blockchain::new(env.clone(), NetworkId::Main, Arc::new(NetworkTime::new())).unwrap();

    let head_hash1 = blockchain.head_hash();
    let proof = blockchain.get_block_proof(&head_hash1, &head_hash1);
    assert_eq!(proof, Some(vec![]));

    let block = crate::next_block(&blockchain).with_nonce(83054).build();
    assert_eq!(blockchain.push(block), Ok(PushResult::Extended));

    let head_hash2 = blockchain.head_hash();
    let proof = blockchain.get_block_proof(&head_hash2, &head_hash2);
    assert_eq!(proof, Some(vec![]));
    let proof = blockchain.get_block_proof(&head_hash1, &head_hash2);
    assert_eq!(proof, Some(vec![]));

    let unknown_hash = Blake2bHash::from([1u8; 32]);
    let proof = blockchain.get_block_proof(&unknown_hash, &head_hash2);
    assert_eq!(proof, None);
    let proof = blockchain.get_block_proof(&head_hash2, &unknown_hash);
    assert_eq!(proof, None);
    let proof = blockchain.get_block_proof(&unknown_hash, &unknown_hash);
    assert_eq!(proof, None);
}
