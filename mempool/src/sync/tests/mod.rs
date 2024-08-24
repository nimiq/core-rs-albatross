#[cfg(test)]
mod tests {
    use std::{sync::Arc, task::Poll, time::Duration};

    use futures::{poll, StreamExt};
    use nimiq_blockchain::{Blockchain, BlockchainConfig};
    use nimiq_database::mdbx::MdbxDatabase;
    use nimiq_genesis::NetworkId;
    use nimiq_hash::{Blake2bHash, Hash};
    use nimiq_keys::{Address, KeyPair, SecureGenerate};
    use nimiq_network_interface::network::Network;
    use nimiq_network_mock::MockHub;
    use nimiq_test_log::test;
    use nimiq_test_utils::{
        consensus::consensus,
        test_rng::test_rng,
        test_transaction::{generate_transactions, TestAccount, TestTransaction},
    };
    use nimiq_time::sleep;
    use nimiq_transaction::Transaction;
    use nimiq_utils::{spawn, time::OffsetTime};
    use parking_lot::RwLock;
    use rand::rngs::StdRng;

    use crate::{
        mempool_state::MempoolState,
        mempool_transactions::TxPriority,
        sync::{
            messages::{
                RequestMempoolHashes, RequestMempoolTransactions, ResponseMempoolHashes,
                ResponseMempoolTransactions,
            },
            MempoolSyncer, MempoolTransactionType, MAX_HASHES_PER_REQUEST,
        },
    };

    #[test(tokio::test)]
    async fn it_can_discover_mempool_hashes_and_get_transactions_from_other_peers() {
        // Generate test transactions
        let num_transactions = MAX_HASHES_PER_REQUEST + 10;
        let mut rng = test_rng(true);
        let mempool_transactions = generate_test_transactions(num_transactions, &mut rng);

        // Turn test transactions into transactions
        let (txns, _): (Vec<Transaction>, usize) =
            generate_transactions(mempool_transactions, true);
        let hashes: Vec<Blake2bHash> = txns.iter().map(|txn| txn.hash()).collect();

        // Create an empty blockchain
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ));

        // Setup empty Mempool State
        let state = Arc::new(RwLock::new(MempoolState::new(100, 100)));

        // Setup network
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Dial peer
        net1.dial_mock(&net2);
        let peer_ids = net1.get_peers();

        // Setup consensus proxy
        let consensus = consensus(Arc::clone(&blockchain), Arc::clone(&net1)).await;
        let proxy = consensus.proxy();

        // Setup streams to respond to requests
        let hash_stream = net2.receive_requests::<RequestMempoolHashes>();
        let txns_stream = net2.receive_requests::<RequestMempoolTransactions>();

        let hashes_repsonse = ResponseMempoolHashes {
            hashes: hashes.clone(),
        };

        let net2_clone = Arc::clone(&net2);
        let hashes_listener_future =
            hash_stream.for_each(move |(_request, request_id, _peer_id)| {
                let test_response = hashes_repsonse.clone();
                let net2 = Arc::clone(&net2_clone);
                async move {
                    net2.respond::<RequestMempoolHashes>(request_id, test_response.clone())
                        .await
                        .unwrap();
                }
            });

        let net2_clone = Arc::clone(&net2);
        let txns_listener_future = txns_stream.for_each(move |(_request, request_id, _peer_id)| {
            let test_response = ResponseMempoolTransactions {
                transactions: txns.clone(),
            };
            let net2 = Arc::clone(&net2_clone);
            async move {
                net2.respond::<RequestMempoolTransactions>(request_id, test_response.clone())
                    .await
                    .unwrap();
            }
        });

        // Spawn the request responders
        spawn(hashes_listener_future);
        spawn(txns_listener_future);

        // Create a new mempool syncer
        let mut syncer = MempoolSyncer::new(
            peer_ids,
            MempoolTransactionType::Regular,
            Arc::clone(&blockchain),
            proxy,
            Arc::clone(&state),
        );

        // Poll to send out requests to peers
        let _ = poll!(syncer.next());
        sleep(Duration::from_millis(200)).await;

        // Poll to check request responses
        let _ = poll!(syncer.next());

        // Verify that all the hashes of the test transactions we generated, have been received by the syncer
        hashes.iter().for_each(|hash| {
            assert!(syncer.unknown_hashes.contains_key(hash));
        });
        assert_eq!(syncer.unknown_hashes.len(), num_transactions);

        // Poll to request the transactions and we should have at least 1 transaction by now
        sleep(Duration::from_millis(200)).await;
        match poll!(syncer.next()) {
            Poll::Ready(txn) => assert!(txn.is_some()),
            Poll::Pending => unreachable!(),
        }

        // Verify that we received all the transactions we have asked for
        // num_transactions - 1 here because we pop_front immediately on a Poll::Ready()
        assert_eq!(syncer.transactions.len(), num_transactions - 1);
    }

    #[test(tokio::test)]
    async fn it_ignores_transactions_we_already_have_locally() {
        // Generate test transactions
        let num_transactions = 10;
        let mut rng = test_rng(true);
        let known_test_transactions = generate_test_transactions(num_transactions, &mut rng);

        // Turn test transactions into transactions
        let (known_txns, _): (Vec<Transaction>, usize) =
            generate_transactions(known_test_transactions, true);
        let known_hashes: Vec<Blake2bHash> = known_txns.iter().map(|txn| txn.hash()).collect();

        // Setup empty Mempool State
        let state = Arc::new(RwLock::new(MempoolState::new(100, 100)));

        // Load known hashes into the mempool state
        let mut handle = state.write();
        known_txns.iter().for_each(|txn| {
            handle.regular_transactions.insert(&txn, TxPriority::Medium);
        });
        assert_eq!(handle.regular_transactions.len(), known_hashes.len());
        drop(handle);

        // Create an empty blockchain
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ));

        // Setup network
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Dial peer
        net1.dial_mock(&net2);
        let peer_ids = net1.get_peers();

        // Setup consensus proxy
        let consensus = consensus(Arc::clone(&blockchain), Arc::clone(&net1)).await;
        let proxy = consensus.proxy();

        // Setup stream to respond to requests
        let hash_stream = net2.receive_requests::<RequestMempoolHashes>();

        let hashes_repsonse = ResponseMempoolHashes {
            hashes: known_hashes.clone(),
        };

        let net2_clone = Arc::clone(&net2);
        let hashes_listener_future =
            hash_stream.for_each(move |(_request, request_id, _peer_id)| {
                let test_response = hashes_repsonse.clone();
                let net2 = Arc::clone(&net2_clone);
                async move {
                    net2.respond::<RequestMempoolHashes>(request_id, test_response.clone())
                        .await
                        .unwrap();
                }
            });

        // Spawn the request responder
        spawn(hashes_listener_future);

        // Create a new mempool syncer
        let mut syncer = MempoolSyncer::new(
            peer_ids,
            MempoolTransactionType::Regular,
            Arc::clone(&blockchain),
            proxy,
            Arc::clone(&state),
        );

        // Poll to send out requests to peers
        let _ = poll!(syncer.next());
        sleep(Duration::from_millis(200)).await;

        // Poll to check request responses
        let _ = poll!(syncer.next());

        // All the hashes we received are already in the mempool state so should be skipped
        assert!(syncer.unknown_hashes.is_empty());
    }

    #[test(tokio::test)]
    async fn it_can_dynamically_add_a_new_peer() {
        // Generate test transactions
        let num_transactions = 10;
        let mut rng = test_rng(true);
        let mempool_transactions = generate_test_transactions(num_transactions, &mut rng);

        // Turn test transactions into transactions
        let (txns, _): (Vec<Transaction>, usize) =
            generate_transactions(mempool_transactions, true);
        let hashes: Vec<Blake2bHash> = txns.iter().map(|txn| txn.hash()).collect();

        // Create an empty blockchain
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ));

        // Setup empty Mempool State
        let state = Arc::new(RwLock::new(MempoolState::new(100, 100)));

        // Setup network
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());

        // Setup consensus proxy
        let consensus = consensus(Arc::clone(&blockchain), Arc::clone(&net1)).await;
        let proxy = consensus.proxy();

        // Create a new mempool syncer with 0 peers to sync with
        let mut syncer = MempoolSyncer::new(
            vec![],
            MempoolTransactionType::Regular,
            Arc::clone(&blockchain),
            proxy,
            Arc::clone(&state),
        );

        // Poll to send out requests to peers and verify nothing has been received
        let _ = poll!(syncer.next());
        sleep(Duration::from_millis(200)).await;
        assert!(syncer.unknown_hashes.is_empty());

        // Add new peer and dial it
        let net2 = Arc::new(hub.new_network());
        net1.dial_mock(&net2);

        // Setup stream to respond to requests
        let hash_stream = net2.receive_requests::<RequestMempoolHashes>();

        let hashes_repsonse = ResponseMempoolHashes {
            hashes: hashes.clone(),
        };

        let net2_clone = Arc::clone(&net2);
        let hashes_listener_future =
            hash_stream.for_each(move |(_request, request_id, _peer_id)| {
                let test_response = hashes_repsonse.clone();
                let net2 = Arc::clone(&net2_clone);
                async move {
                    net2.respond::<RequestMempoolHashes>(request_id, test_response.clone())
                        .await
                        .unwrap();
                }
            });

        // Spawn the request responder
        spawn(hashes_listener_future);

        // Dynamically add new peer to mempool syncer
        syncer.add_peer(net2.peer_id());

        let _ = poll!(syncer.next());
        sleep(Duration::from_millis(200)).await;

        // We have received unknown hashes from the added peer
        let _ = poll!(syncer.next());
        assert_eq!(syncer.unknown_hashes.len(), num_transactions);
    }

    fn generate_test_transactions(num: usize, mut rng: &mut StdRng) -> Vec<TestTransaction> {
        let mut mempool_transactions = vec![];

        for _ in 0..num {
            let kp1 = KeyPair::generate(&mut rng);
            let addr1 = Address::from(&kp1.public);
            let account1 = TestAccount {
                address: addr1,
                keypair: kp1,
            };

            let kp2 = KeyPair::generate(&mut rng);
            let addr2 = Address::from(&kp2.public);
            let account2 = TestAccount {
                address: addr2,
                keypair: kp2,
            };

            let mempool_transaction = TestTransaction {
                fee: 0,
                value: 10,
                recipient: account1,
                sender: account2,
            };
            mempool_transactions.push(mempool_transaction);
        }

        mempool_transactions
    }
}
