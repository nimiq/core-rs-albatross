use std::{str::FromStr, sync::Arc};

use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{sync::syncer_proxy::SyncerProxy, Consensus};
use nimiq_database::volatile::VolatileDatabase;
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_network_interface::network::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        fill_micro_blocks_with_txns, produce_macro_blocks, signing_key, voting_key, REWARD_KEY,
    },
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
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
            VolatileDatabase::new(20).unwrap(),
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
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
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
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
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
        .request_transaction_receipts_by_address(Address::from(&key_pair.public), 1, None)
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
