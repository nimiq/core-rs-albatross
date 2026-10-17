//! Regression test for the transaction verification cache.
//!
//! The cache is keyed by `Transaction::hash`, which excludes the `proof`, so a block
//! producer can substitute the proof of a transaction pending in honest mempools and
//! have cache-backed nodes skip the signature check. A forged proof must be rejected on
//! every node, regardless of what its cache holds.

use std::{str::FromStr, sync::Arc};

use nimiq_block::BlockError;
use nimiq_blockchain::TransactionVerificationCache;
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Ed25519Signature, KeyPair, PrivateKey};
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool};
use nimiq_primitives::transaction::TransactionError;
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{generate_transactions, REWARD_KEY},
};
use nimiq_transaction::SignatureProof;

fn is_invalid_proof(result: &Result<PushResult, PushError>) -> bool {
    matches!(
        result,
        Err(PushError::InvalidBlock(BlockError::InvalidTransaction(
            TransactionError::InvalidProof
        )))
    )
}

#[test(tokio::test)]
async fn forged_proof_is_rejected_even_when_the_honest_twin_is_cached() {
    let cached_node = TemporaryBlockProducer::new();
    let uncached_node = TemporaryBlockProducer::new();
    let key_pair = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());
    let (network_id, version) = {
        let blockchain = cached_node.blockchain.read();
        (blockchain.network_id, blockchain.protocol_version())
    };

    let honest = generate_transactions(
        &key_pair,
        cached_node.blockchain.read().block_number(),
        network_id,
        1,
        1,
    )
    .remove(0);
    assert!(honest.verify(network_id, version).is_ok());

    // Same signed content, invalid proof: shares the honest transaction's hash.
    let mut forged = honest.clone();
    forged.proof =
        SignatureProof::from_ed25519(key_pair.public, Ed25519Signature::from([0x11u8; 64]))
            .serialize_to_vec();
    forged.reset_verification_state();
    assert_eq!(forged.hash::<Blake2bHash>(), honest.hash::<Blake2bHash>());
    assert_eq!(
        forged.verify(network_id, version),
        Err(TransactionError::InvalidProof)
    );

    // Install a mempool holding the honest transaction as the cache, as a validator does.
    let mempool = Arc::new(Mempool::new(
        Arc::clone(&cached_node.blockchain),
        MempoolConfig::default(),
    ));
    mempool.add_transaction(honest.clone(), None).unwrap();
    assert!(mempool.contains_transaction_by_hash(&honest.hash::<Blake2bHash>()));
    cached_node.blockchain.write().tx_verification_cache =
        Arc::clone(&mempool) as Arc<dyn TransactionVerificationCache>;

    let forged_block = cached_node.next_block_no_push_with_txs(vec![], false, vec![forged]);

    // A node that never cached the honest transaction rejects the forged proof.
    let uncached_result = uncached_node.push(forged_block.clone());
    assert!(
        is_invalid_proof(&uncached_result),
        "control node should reject the forged block, got {uncached_result:?}"
    );

    // The cache-backed node must reject it too.
    let cached_result = cached_node.push(forged_block);
    assert!(
        is_invalid_proof(&cached_result),
        "cache-backed node accepted a forged transaction proof: {cached_result:?}"
    );

    // The Mempool must recognize the honest transaction as verified.
    assert!(mempool.is_verified(&honest, version));

    // The cache must not simply be disabled: an honest block is still accepted.
    let honest_block = cached_node.next_block_no_push_with_txs(vec![], false, vec![honest]);
    assert_eq!(cached_node.push(honest_block), Ok(PushResult::Extended));
}
