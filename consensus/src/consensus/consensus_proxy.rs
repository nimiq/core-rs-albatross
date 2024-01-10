use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use futures::stream::BoxStream;
use nimiq_account::{Account, Staker, Validator};
use nimiq_block::Block;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_network_interface::{
    network::Network,
    peer_info::Services,
    request::{OutboundRequestError, RequestError},
};
use nimiq_primitives::{account::AccountType, key_nibbles::KeyNibbles, policy::Policy};
use nimiq_transaction::{
    historic_transaction::HistoricTransaction, ControlTransactionTopic, Transaction,
    TransactionTopic,
};
use tokio::sync::{broadcast::Sender as BroadcastSender, mpsc::Sender as MpscSender};
use tokio_stream::wrappers::BroadcastStream;

use super::ConsensusRequest;
use crate::{
    consensus::remote_data_store::RemoteDataStore,
    messages::{
        AddressNotification, AddressSubscriptionOperation, AddressSubscriptionTopic,
        RequestBlocksProof, RequestSubscribeToAddress, RequestTransactionReceiptsByAddress,
        RequestTransactionsProof, ResponseBlocksProof,
    },
    ConsensusEvent,
};

pub struct ConsensusProxy<N: Network> {
    pub blockchain: BlockchainProxy,
    pub network: Arc<N>,
    pub(crate) established_flag: Arc<AtomicBool>,
    pub(crate) events: BroadcastSender<ConsensusEvent>,
    pub(crate) request: MpscSender<ConsensusRequest>,
}

impl<N: Network> Clone for ConsensusProxy<N> {
    fn clone(&self) -> Self {
        Self {
            blockchain: self.blockchain.clone(),
            network: Arc::clone(&self.network),
            established_flag: Arc::clone(&self.established_flag),
            events: self.events.clone(),
            request: self.request.clone(),
        }
    }
}

impl<N: Network> ConsensusProxy<N> {
    pub async fn send_transaction(&self, tx: Transaction) -> Result<(), N::Error> {
        if tx.sender_type == AccountType::Staking || tx.recipient_type == AccountType::Staking {
            return self.network.publish::<ControlTransactionTopic>(tx).await;
        }
        self.network.publish::<TransactionTopic>(tx).await
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }

    pub fn subscribe_events(&self) -> BroadcastStream<ConsensusEvent> {
        BroadcastStream::new(self.events.subscribe())
    }

    /// Subscribe to remote address notification events
    pub async fn subscribe_address_notifications(
        &self,
    ) -> BoxStream<(AddressNotification, N::PubsubId)> {
        let txn_stream = self
            .network
            .subscribe_subtopic::<AddressSubscriptionTopic>(
                self.network.get_local_peer_id().to_string(),
            )
            .await;

        txn_stream.unwrap()
    }

    pub async fn request_transaction_receipts_by_address(
        &self,
        address: Address,
        min_peers: usize,
        max: Option<u16>,
    ) -> Result<Vec<(Blake2bHash, u32)>, RequestError> {
        let mut obtained_receipts = HashSet::new();

        // We obtain a list of connected peers that could satisfy our request and perform the request to each one:
        for peer_id in self
            .get_peers_for_service(Services::TRANSACTION_INDEX, min_peers)
            .await?
        {
            log::debug!(
                peer_id = %peer_id,
                "Performing txn receipts by address request to peer",
            );
            let response = self
                .network
                .request::<RequestTransactionReceiptsByAddress>(
                    RequestTransactionReceiptsByAddress {
                        address: address.clone(),
                        max,
                    },
                    peer_id,
                )
                .await;

            match response {
                Ok(response) => {
                    log::debug!(
                        "Obtained txn receipts response, length {} ",
                        response.receipts.len()
                    );
                    obtained_receipts.extend(response.receipts);
                }
                Err(error) => {
                    // If there was a request error with this peer we log an error
                    log::error!(peer=%peer_id, err=%error,"There was an error requesting transaction receipts from peer");
                }
            }
        }

        let mut receipts: Vec<_> = obtained_receipts.into_iter().collect();
        receipts.sort_unstable_by_key(|receipt| receipt.1);
        receipts.reverse(); // Return newest receipts (highest block_number) first

        Ok(receipts)
    }

    pub async fn request_transactions_by_address(
        &self,
        address: Address,
        since_block_height: u32,
        ignored_hashes: Vec<Blake2bHash>,
        min_peers: usize,
        max: Option<u16>,
    ) -> Result<Vec<HistoricTransaction>, RequestError> {
        let receipts: Vec<_> = self
            .request_transaction_receipts_by_address(address, min_peers, max)
            .await?
            .into_iter()
            .filter(|(hash, block_number)| {
                block_number > &since_block_height && !ignored_hashes.contains(hash)
            })
            .map(|(hash, block_number)| (hash, Some(block_number)))
            .collect();

        if receipts.is_empty() {
            return Ok(vec![]);
        }

        self.prove_transactions_from_receipts(receipts, min_peers)
            .await
    }

    pub async fn request_transaction_by_hash_and_block_number(
        &self,
        tx_hash: Blake2bHash,
        block_number: u32,
        min_peers: usize,
    ) -> Result<HistoricTransaction, RequestError> {
        let receipts = vec![(tx_hash, Some(block_number))];
        let mut txs = self
            .prove_transactions_from_receipts(receipts, min_peers)
            .await?;
        match txs.pop() {
            Some(tx) => Ok(tx),
            None => Err(RequestError::OutboundRequest(
                OutboundRequestError::NoReceiver,
            )),
        }
    }

    pub async fn request_transaction_by_hash(
        &self,
        tx_hash: Blake2bHash,
        min_peers: usize,
    ) -> Result<HistoricTransaction, RequestError> {
        let receipts = vec![(tx_hash, None)];
        let mut txs = self
            .prove_transactions_from_receipts(receipts, min_peers)
            .await?;
        match txs.pop() {
            Some(tx) => Ok(tx),
            None => Err(RequestError::OutboundRequest(
                OutboundRequestError::NoReceiver,
            )),
        }
    }

    async fn get_peers_for_service(
        &self,
        services: Services,
        min_peers: usize,
    ) -> Result<Vec<<N as Network>::PeerId>, RequestError> {
        // First we tell the network to provide us with a vector that contains all the connected peers that support such services
        // Note: If the network could not provide enough peers that satisfy our requirement, then an error would be returned
        self.network
            .get_peers_by_services(services, min_peers)
            .await
            .map_err(|error| {
                log::error!(
                    err = %error,
                    "The request couldn't be fulfilled"
                );

                RequestError::OutboundRequest(OutboundRequestError::SendError)
            })
    }

    pub async fn prove_transactions_from_receipts(
        &self,
        receipts: Vec<(Blake2bHash, Option<u32>)>,
        min_peers: usize,
    ) -> Result<Vec<HistoricTransaction>, RequestError> {
        let blockchain = self.blockchain.read();
        let election_head = blockchain.election_head();
        let checkpoint_head = blockchain.macro_head();
        let current_head = blockchain.head();
        let current_block_number = current_head.block_number();

        // We drop the blockchain lock because it's no longer needed while we request proofs
        drop(blockchain);

        if receipts
            .iter()
            .any(|(_, block_number)| block_number.unwrap_or(0) > current_block_number)
        {
            log::error!(
                head = current_block_number,
                "Can't proof a transaction from the future"
            );
            return Err(RequestError::OutboundRequest(OutboundRequestError::Other(
                "Can't proof a transaction from the future".to_string(),
            )));
        }

        let mut verified_transactions = HashMap::new();

        let full_node_cutoff = election_head.block_number() - Policy::blocks_per_epoch() + 1;
        let can_query_full_nodes = receipts
            .iter()
            .all(|(_, block_number)| block_number.unwrap_or(0) > full_node_cutoff);
        let peer_required_service = if can_query_full_nodes {
            Services::FULL_BLOCKS
        } else {
            Services::TRANSACTION_INDEX
        };

        // We obtain a list of connected peers that could satisfy our request and perform the request to each one:
        for peer_id in self
            .get_peers_for_service(peer_required_service, min_peers)
            .await?
        {
            // This is the structure where we group transactions by their proving block number
            let mut hashes_by_block = HashMap::new();

            for (hash, block_number) in &receipts {
                // If the transaction was already verified, then we don't need to verify it again
                if verified_transactions.contains_key(&hash.clone().into()) {
                    continue;
                }

                // There are essentially two different cases that we need to handle
                if let Some(block_number) = block_number {
                    // Case A: We are provided a block number, so we need to determine which is the best proving block
                    // There are three sub-categories of block numbers:
                    //  - Finalized epochs: we use the election block number that finalized the respective epoch
                    //  - Finalized batch in the current epoch: We use the latest checkpoint block number
                    //  - Current batch: We use the current head to prove those transactions
                    if block_number <= &election_head.block_number() {
                        // First Case: Transactions from finalized epochs
                        hashes_by_block
                            .entry(Some(Policy::election_block_after(*block_number)))
                            .or_insert(vec![])
                            .push(hash.clone());
                    } else if block_number <= &checkpoint_head.block_number() {
                        // Second Case: Transactions from a finalized batch in the current epoch
                        hashes_by_block
                            .entry(Some(checkpoint_head.block_number()))
                            .or_insert(vec![])
                            .push(hash.clone());
                    } else {
                        // Third Case: Transactions from the current batch
                        hashes_by_block
                            .entry(Some(current_head.block_number()))
                            .or_insert(vec![])
                            .push(hash.clone());
                    }
                } else {
                    // Case B: We are not provided a block_number
                    hashes_by_block
                        .entry(None)
                        .or_insert(vec![])
                        .push(hash.clone());
                }
            }

            if hashes_by_block.is_empty() {
                break;
            }

            // Now we request proofs for each block and its hashes, according to its classification
            for (block_number, hashes) in hashes_by_block {
                if let Some(block_number) = block_number {
                    log::debug!(
                    block_number=%block_number,
                    "Performing txn proof request for block number");
                } else {
                    log::debug!("Performing txn proof request without block number");
                }

                let response = self
                    .network
                    .request::<RequestTransactionsProof>(
                        RequestTransactionsProof {
                            hashes,
                            block_number,
                        },
                        peer_id,
                    )
                    .await;
                match response {
                    Ok(Ok(response)) => {
                        // We verify the transaction using the proof
                        log::debug!(peer = %peer_id, block = %response.block, "New txns proof and block from peer");
                        let mut verification_result = response
                            .proof
                            .verify(response.block.history_root().clone())
                            .unwrap_or(false);

                        if !verification_result {
                            // If the proof didn't verify, we continue with another peer
                            log::warn!(peer = %peer_id, "The transaction history proof from this peer did not verify");
                            continue;
                        }

                        // Verify that the transaction proof fits to the chain
                        if response.block.block_number() <= election_head.block_number() {
                            let block_hash = response.block.hash();
                            let mut already_proven = false;
                            if election_head.hash() == block_hash
                                || election_head.header.parent_election_hash == block_hash
                            {
                                already_proven = true;
                            } else if let Some(ref interlink) = election_head.header.interlink {
                                already_proven = interlink.contains(&block_hash);
                            }

                            if !already_proven {
                                // Request block inclusion proofs for txns of previous epochs
                                let block_proof = match self
                                    .network
                                    .request::<RequestBlocksProof>(
                                        RequestBlocksProof {
                                            election_head: election_head.block_number(),
                                            blocks: vec![response.block.block_number()],
                                        },
                                        peer_id,
                                    )
                                    .await
                                {
                                    Ok(Ok(ResponseBlocksProof { proof })) => proof,
                                    Ok(Err(error)) => {
                                        log::debug!(%error, peer = %peer_id, "Error on remote side while requesting block proof");
                                        continue;
                                    }
                                    Err(error) => {
                                        log::debug!(%error, peer = %peer_id, "Error requesting block proof");
                                        continue;
                                    }
                                };

                                // Verify that the block is part of the chain using the block inclusion proof
                                if let Block::Macro(macro_block) = response.block {
                                    verification_result = verification_result
                                        && block_proof
                                            .is_block_proven(&election_head, &macro_block);
                                } else {
                                    log::debug!(peer = %peer_id, "Macro block expected in tx proof response");
                                    continue;
                                }
                            }
                        } else if response.block.block_number() <= checkpoint_head.block_number() {
                            // Check that the transaction inclusion proof actually proofs inclusion in the block we know
                            if response.block.hash() != checkpoint_head.hash() {
                                log::debug!(peer = %peer_id, "BlockProof does not correspond to expected checkpoint block");
                                continue;
                            }
                        } else if response.block.hash() != current_head.hash() {
                            log::debug!(block_number = %response.block.block_number(), peer=%peer_id, "BlockProof does not correspond to expected block");
                            continue;
                        }

                        if verification_result {
                            for tx in response.proof.history {
                                verified_transactions.insert(tx.tx_hash(), tx);
                            }
                        } else {
                            // The proof didn't verify so we continue with another peer
                            log::warn!(peer = %peer_id, "The transaction block proof from this peer did not verify");
                        }
                    }
                    Ok(Err(error)) => {
                        log::debug!(peer = %peer_id, %error, "We requested a transaction proof but the peer couldn't provide any");
                    }
                    Err(error) => {
                        // If there was a request error with this peer we don't request anymore proofs from it
                        log::error!(peer = %peer_id, %error, "There was an error requesting transaction proof from peer");
                        break;
                    }
                }
            }
        }

        // Sort transactions by block_number
        let mut transactions: Vec<_> = verified_transactions.into_values().collect();
        transactions.sort_unstable_by_key(|hist_tx| hist_tx.block_number);
        transactions.reverse(); // Return newest transaction (highest block_number) first

        Ok(transactions)
    }

    /// Gets a set of accounts given their addresses. The returned type is a
    /// BTreeMap of addresses to an optional `Account`. If an account was not
    /// found, then `None` is returned in its corresponding entry.
    pub async fn request_accounts_by_addresses(
        &self,
        addresses: Vec<Address>,
        min_peers: usize,
    ) -> Result<BTreeMap<Address, Option<Account>>, RequestError> {
        let mut keys = HashMap::<KeyNibbles, Address>::from_iter(
            addresses
                .iter()
                .map(|address| (KeyNibbles::from(address), address.clone())),
        );
        let accounts: BTreeMap<KeyNibbles, Option<Account>> = RemoteDataStore::get_trie(
            Arc::clone(&self.network),
            self.blockchain.clone(),
            &keys.keys().cloned().collect::<Vec<KeyNibbles>>(),
            min_peers,
        )
        .await?;

        let accounts = accounts
            .iter()
            .map(|(key, account)| {
                (
                    keys.remove(key)
                        .expect("Key must be in the proven accounts"),
                    account.clone(),
                )
            })
            .collect();
        Ok(accounts)
    }

    /// Gets a set of validators given their addresses. The returned type is a
    /// BTreeMap of addresses to an optional `Validator`. If a validator was not
    /// found, then `None` is returned in its corresponding entry.
    pub async fn request_validators_by_addresses(
        &self,
        addresses: Vec<Address>,
        min_peers: usize,
    ) -> Result<BTreeMap<Address, Option<Validator>>, RequestError> {
        let remote_ds = RemoteDataStore {
            network: Arc::clone(&self.network),
            blockchain: self.blockchain.clone(),
            min_peers,
        };
        remote_ds.get_validators(addresses).await
    }

    /// Gets a set of stakers given their addresses. The returned type is a
    /// BTreeMap of addresses to an optional `Staker`. If a staker was not
    /// found, then `None` is returned in its corresponding entry.
    pub async fn request_stakers_by_addresses(
        &self,
        addresses: Vec<Address>,
        min_peers: usize,
    ) -> Result<BTreeMap<Address, Option<Staker>>, RequestError> {
        let remote_ds = RemoteDataStore {
            network: Arc::clone(&self.network),
            blockchain: self.blockchain.clone(),
            min_peers,
        };
        remote_ds.get_stakers(addresses).await
    }

    pub async fn subscribe_to_addresses(
        &self,
        addresses: Vec<Address>,
        min_peers: usize,
        peer_id: Option<N::PeerId>,
    ) -> Result<(), RequestError> {
        // If we are provided a peer_id we perform the request only to this specific peer
        let peers = if let Some(peer_id) = peer_id {
            if self
                .network
                .peer_provides_services(peer_id, Services::FULL_BLOCKS)
            {
                // Providing the specific peer can be used in cases where the light client receives notifications that a new peer joined the network
                // and then it wants to subscribe to this specific peer.
                vec![peer_id]
            } else {
                vec![]
            }
        } else {
            // We tell the network to provide us with a vector that contains all the connected peers that support such services.
            self.get_peers_for_service(Services::FULL_BLOCKS, min_peers)
                .await?
        };

        let mut success = false;

        // Subscribe to all peers that could provide the necessary services
        for peer_id in peers {
            let response = self
                .network
                .request::<RequestSubscribeToAddress>(
                    RequestSubscribeToAddress {
                        operation: AddressSubscriptionOperation::Subscribe,
                        addresses: addresses.clone(),
                    },
                    peer_id,
                )
                .await;

            match response {
                Ok(Ok(())) => {
                    // Done, we are subscribed at least to one peer, continue with the next one
                    success = true;
                    continue;
                }
                Ok(Err(_)) => {
                    // If there was en error subscribing to a peer, we just continue with the next one
                    // Here we could do something with the specific error conditions of the failed subscription
                    continue;
                }
                Err(_) => {
                    // Try with the next peer
                    continue;
                }
            }
        }
        if success {
            Ok(())
        } else {
            Err(
                nimiq_network_interface::request::RequestError::OutboundRequest(
                    OutboundRequestError::NoReceiver,
                ),
            )
        }
    }

    pub async fn unsubscribe_from_addresses(
        &self,
        addresses: Vec<Address>,
        min_peers: usize,
    ) -> Result<(), RequestError> {
        // Unsubscribe given addresses from all peers
        // Note: this does not mean that we will fully unsubscribe  from a peer,
        // we will unsubscribe  only from the addresses that were supplied to this function
        for peer_id in self
            .get_peers_for_service(Services::FULL_BLOCKS, min_peers)
            .await?
        {
            let _ = self
                .network
                .request::<RequestSubscribeToAddress>(
                    RequestSubscribeToAddress {
                        operation: AddressSubscriptionOperation::Unsubscribe,
                        addresses: addresses.clone(),
                    },
                    peer_id,
                )
                .await;

            // We don't care about the response, we just unsubscribe addresses from peers
        }
        Ok(())
    }
}
