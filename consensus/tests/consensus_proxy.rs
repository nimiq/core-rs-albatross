use std::{str::FromStr, sync::Arc};

use futures::StreamExt;
use nimiq_blockchain::{interface::HistoryInterface, BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{
    messages::RequestTransactionsProof, sync::syncer_proxy::SyncerProxy, BlsCache, Consensus,
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_network_interface::{network::Network, request::Handle};
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks_with_txns, produce_macro_blocks, signing_key, voting_key, REWARD_KEY,
};
use nimiq_transaction::{
    historic_transaction::HistoricTransactionData, ExecutedTransaction, TransactionFormat,
};
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};

#[test(tokio::test)]
async fn test_request_transactions_by_address() {
    let mut hub = MockHub::default();

    // Create one node with a full epoch. The first batch will have one tx per block.
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());
    fill_micro_blocks_with_txns(&producer, &blockchain1, 1, 1);
    // Produce one epoch, such that the transactions are in a finalized epoch
    let num_macro_blocks = (Policy::batches_per_epoch() + 1) as usize;
    produce_macro_blocks(&producer, &blockchain1, num_macro_blocks);

    let net1 = Arc::new(hub.new_network());
    let zkp_prover1 =
        ZKPComponent::new(BlockchainProxy::from(&blockchain1), Arc::clone(&net1), None)
            .await
            .proxy();
    let blockchain1_proxy = BlockchainProxy::from(&blockchain1);

    let syncer1 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        Arc::new(Mutex::new(BlsCache::new_test())),
        net1.subscribe_events(),
    )
    .await;

    let _consensus1 = Consensus::from_network(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        syncer1,
        zkp_prover1.clone(),
    );

    // Setup another node that will sync with the previous one.
    let net2 = Arc::new(hub.new_network());
    let syncer2 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net2),
        Arc::new(Mutex::new(BlsCache::new_test())),
        net2.subscribe_events(),
    )
    .await;
    let consensus2 = Consensus::from_network(
        blockchain1_proxy.clone(),
        Arc::clone(&net2),
        syncer2,
        zkp_prover1,
    );
    let consensus_proxy = consensus2.proxy();
    net1.dial_mock(&net2);

    // Fetching all the transactions of the epoch.
    let key_pair = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());

    let receipts = consensus_proxy
        .request_transaction_receipts_by_address(
            Address::from(&key_pair.public),
            1,
            None,
            None,
            false,
        )
        .await;
    assert!(receipts.is_ok());
    let res = consensus_proxy
        .prove_transactions_from_receipts(
            receipts
                .unwrap()
                .into_iter()
                .map(|r| (r.0, Some(r.1)))
                .collect(),
            1,
        )
        .await;
    assert!(res.is_ok());

    let txs = res.unwrap();

    assert_eq!(
        txs.iter()
            .filter(|tx| {
                if let HistoricTransactionData::Basic(ExecutedTransaction::Ok(transaction)) =
                    &tx.data
                {
                    return transaction.format() == TransactionFormat::Basic;
                }
                false
            })
            .count() as u32,
        // There should be one basic transaction in each micro block of the first batch
        Policy::blocks_per_batch() - 1
    );
}

/// Regression test for a transaction-proof integrity issue: a proof attests only that a transaction
/// is part of the chain history, it carries no notion of *which* transactions were requested. A
/// malicious peer can therefore answer a lookup for transaction `A` with a perfectly valid
/// inclusion proof for an unrelated transaction `B` that genuinely is in the chain.
///
/// `prove_transactions_from_receipts` must discard any proven transaction whose hash (or, when a
/// block number was requested, whose block number) does not match what we actually asked for.
#[test(tokio::test)]
async fn test_rejects_proof_for_unrequested_transaction() {
    let mut hub = MockHub::default();

    // Create a node with a full (finalized) epoch. Each micro block of the first batch has one tx.
    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            MdbxDatabase::new_volatile(Default::default()).unwrap(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            Arc::new(OffsetTime::new()),
        )
        .unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());
    fill_micro_blocks_with_txns(&producer, &blockchain, 1, 1);
    let num_macro_blocks = (Policy::batches_per_epoch() + 1) as usize;
    produce_macro_blocks(&producer, &blockchain, num_macro_blocks);

    // Pick a real transaction `B` from the first micro block. The malicious peer will serve a
    // genuine inclusion proof for `B` no matter which transaction is actually requested.
    let block_b = Policy::genesis_block_number() + 1;
    let tx_b = blockchain
        .read()
        .history_store
        .get_block_transactions(block_b, None)
        .into_iter()
        .find(|tx| matches!(tx.data, HistoricTransactionData::Basic(_)))
        .expect("first micro block should contain a basic transaction");
    let hash_b: Blake2bHash = tx_b.tx_hash().into();

    // The malicious peer's network. We deliberately do NOT spawn a full consensus on it, so we can
    // install our own handler that always proves `B`, regardless of what was requested.
    let malicious_net = Arc::new(hub.new_network());
    {
        let mut requests = malicious_net.receive_requests::<RequestTransactionsProof>();
        let malicious_net = Arc::clone(&malicious_net);
        let blockchain = Arc::clone(&blockchain);
        let hash_b = hash_b.clone();
        tokio::spawn(async move {
            while let Some((_request, request_id, peer_id)) = requests.next().await {
                // Ignore the requested hashes and always answer with a valid proof for `B`.
                let response = <RequestTransactionsProof as Handle<
                    MockNetwork,
                    Arc<RwLock<Blockchain>>,
                >>::handle(
                    &RequestTransactionsProof {
                        hashes: vec![hash_b.clone()],
                        block_number: None,
                    },
                    peer_id,
                    &blockchain,
                );
                malicious_net
                    .respond::<RequestTransactionsProof>(request_id, response)
                    .await
                    .ok();
            }
        });
    }

    // The victim node shares the same blockchain (so the chain-binding checks pass) and queries the
    // malicious peer.
    let victim_net = Arc::new(hub.new_network());
    let blockchain_proxy = BlockchainProxy::from(&blockchain);
    let zkp_prover = ZKPComponent::new(blockchain_proxy.clone(), Arc::clone(&victim_net), None)
        .await
        .proxy();
    let syncer = SyncerProxy::new_history(
        blockchain_proxy.clone(),
        Arc::clone(&victim_net),
        Arc::new(Mutex::new(BlsCache::new_test())),
        victim_net.subscribe_events(),
    )
    .await;
    let consensus = Consensus::from_network(
        blockchain_proxy.clone(),
        Arc::clone(&victim_net),
        syncer,
        zkp_prover,
    );
    let consensus_proxy = consensus.proxy();
    malicious_net.dial_mock(&victim_net);

    // 1. Ask for an unrelated hash without a block number (the web client `getTransaction` path).
    //    The peer answers with a valid proof for `B`, which must be rejected.
    let unrelated_hash = Blake2bHash::default();
    assert_ne!(unrelated_hash, hash_b);
    let result = consensus_proxy
        .prove_transactions_from_receipts(vec![(unrelated_hash, None)], 1)
        .await
        .expect("request should succeed");
    assert!(
        result.is_empty(),
        "peer substituted an unrelated transaction for the requested one"
    );

    // 2. Ask for `B`'s hash but bound to the wrong block number. The proof for `B` (in its real
    //    block) must be rejected because the requested block number does not match.
    let wrong_block = block_b + 1;
    let result = consensus_proxy
        .prove_transactions_from_receipts(vec![(hash_b.clone(), Some(wrong_block))], 1)
        .await
        .expect("request should succeed");
    assert!(
        result.is_empty(),
        "peer proof was accepted for a mismatching block number"
    );

    // 3. Sanity check: a correct lookup for `B` (right hash, right block) still succeeds.
    let result = consensus_proxy
        .prove_transactions_from_receipts(vec![(hash_b.clone(), Some(block_b))], 1)
        .await
        .expect("request should succeed");
    assert_eq!(result.len(), 1);
    assert_eq!(Blake2bHash::from(result[0].tx_hash()), hash_b);
}
