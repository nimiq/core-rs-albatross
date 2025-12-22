//! Interface for sending requests to the consensus component.
//!
//! Used by other modules to resolve blocks or subscribe to consensus events.

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
use nimiq_primitives::{key_nibbles::KeyNibbles, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, RawTransactionHash},
    ControlTransaction, ControlTransactionTopic, Transaction, TransactionTopic,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::wrappers::BroadcastStream;

use super::{ConsensusRequest, ResolveBlockError, ResolveBlockRequest};
use crate::{
    consensus::{remote_data_store::RemoteDataStore, SyncProgress},
    messages::{
        AddressNotification, AddressSubscriptionOperation, AddressSubscriptionTopic,
        RequestBlocksProof, RequestKeccak256TransactionProof, RequestSubscribeToAddress,
        RequestTransactionReceiptsByAddress, RequestTransactionsProof, ResponseBlocksProof,
        ResponseKeccak256TransactionProof,
    },
    ConsensusEvent,
};

/// This struct is used to track the progress of the consensus sync
/// and return it to the user.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusSyncStatus {
    pub is_established: bool,
    pub synced_validity_window: bool,
    pub current_block: u32,
    pub remaining_blocks: u32,
    pub state_sync_progress: u32,
}

/// Implements the logic for handling consensus-related requests.
///
/// The consensus proxy provides methods for interacting with peers, syncing nodes,
/// retrieving blockchain data, and managing subscriptions.
/// Used as an external interface to the internal consensus logic.
pub struct ConsensusProxy<N: Network> {
    pub blockchain: BlockchainProxy,
    pub network: Arc<N>,
    pub(crate) sync_progress: Arc<SyncProgress>,
    pub(crate) established_flag: Arc<AtomicBool>,
    pub(crate) synced_validity_window_flag: Arc<AtomicBool>,
    pub(crate) events: broadcast::Sender<ConsensusEvent>,
    pub(crate) request: mpsc::Sender<ConsensusRequest<N>>,
}

impl<N: Network> Clone for ConsensusProxy<N> {
    fn clone(&self) -> Self {
        Self {
            blockchain: self.blockchain.clone(),
            network: Arc::clone(&self.network),
            sync_progress: Arc::clone(&self.sync_progress),
            established_flag: Arc::clone(&self.established_flag),
            synced_validity_window_flag: Arc::clone(&self.synced_validity_window_flag),
            events: self.events.clone(),
            request: self.request.clone(),
        }
    }
}
impl<N: Network> ConsensusProxy<N> {
    pub async fn send_transaction(&self, tx: Transaction) -> Result<(), N::Error> {
        match ControlTransaction::try_from(tx) {
            Ok(ctx) => self.network.publish::<ControlTransactionTopic>(ctx).await,
            Err(err) => {
                self.network
                    .publish::<TransactionTopic>(err.into_inner())
                    .await
            }
        }
    }

    /// Returns the current sync status of the consensus.
    pub fn get_sync_status(&self) -> ConsensusSyncStatus {
        let is_established = self.is_established();
        let synced_validity_window = self.blockchain.read().can_enforce_validity_window();
        let current_block = self
            .sync_progress
            .current_block_number
            .load(Ordering::Acquire);
        let mut remaining_blocks = self
            .sync_progress
            .remaining_blocks_to_sync
            .load(Ordering::Acquire);
        let mut state_sync_progress = self
            .sync_progress
            .live_sync_progress
            .load(Ordering::Acquire);

        // If consensus is established, we set the remaining blocks to 0 and the state sync progress to 100
        if is_established {
            remaining_blocks = 0;
            state_sync_progress = 100;
        }

        ConsensusSyncStatus {
            is_established,
            synced_validity_window,
            current_block,
            remaining_blocks,
            state_sync_progress,
        }
    }

    /// Returns true if consensus is established.
    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }

    /// Returns true if the node is ready to start the validator/mempool.
    pub fn is_ready_for_validation(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
            && self.synced_validity_window_flag.load(Ordering::Acquire)
    }

    /// Subsribes to consensus events: Established or Lost.
    pub fn subscribe_events(&self) -> BroadcastStream<ConsensusEvent> {
        BroadcastStream::new(self.events.subscribe())
    }

    /// Subscribe to remote address notification events
    pub async fn subscribe_address_notifications(
        &self,
    ) -> BoxStream<'_, (AddressNotification, N::PubsubId)> {
        let txn_stream = self
            .network
            .subscribe_subtopic::<AddressSubscriptionTopic>(
                self.network.get_local_peer_id().to_string(),
            )
            .await;

        txn_stream.unwrap()
    }

    /// Requests transaction receipts for a given address from multiple peers.
    ///
    /// The request can be limited by a maximum number of receipts (`max`),
    /// a starting point (`start_at`, hash), and whether to include pre-genesis data.
    ///
    /// Fails if not enough peers are available to satisfy `min_peers`,
    /// or if no valid response is received after contacting multiple peers.
    ///
    /// See [`RequestError`] and its variants for possible failure causes.
    pub async fn request_transaction_receipts_by_address(
        &self,
        address: Address,
        min_peers: usize,
        max: Option<u16>,
        start_at: Option<Blake2bHash>,
        include_pre_genesis: bool,
    ) -> Result<Vec<(Blake2bHash, u32)>, RequestError> {
        let mut obtained_receipts = HashSet::new();

        // If we need to include pre-genesis data, we set the appropriate services flag
        let services = if include_pre_genesis {
            Services::PRE_GENESIS_TRANSACTIONS | Services::TRANSACTION_INDEX
        } else {
            Services::TRANSACTION_INDEX
        };

        // We obtain a list of connected peers that could satisfy our request and perform the request to each one:
        for peer_id in self.get_peers_for_service(services, min_peers).await? {
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
                        start_at: start_at.clone(),
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

    /// Requests a transaction by hash and block number from multiple peers.
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

    /// Requests a transaction by hash from multiple peers.
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

    /// Requests a Keccak256-based Merkle proof for a transaction from multiple peers.
    ///
    /// This function requests a Keccak256 transaction proof for the given transaction hash
    /// at the specified macro block number. The proof can be used to verify transaction
    /// inclusion using Ethereum-compatible tools and libraries.
    pub async fn request_keccak256_transaction_proof(
        &self,
        tx_hash: Blake2bHash,
        macro_block_number: u32,
        min_peers: usize,
    ) -> Result<ResponseKeccak256TransactionProof, RequestError> {
        // Verify that the block number corresponds to a macro block
        if !Policy::is_macro_block_at(macro_block_number) {
            return Err(RequestError::OutboundRequest(OutboundRequestError::Other(
                format!(
                    "Block number {} is not a macro block number",
                    macro_block_number
                ),
            )));
        }

        // Get peers that support Keccak256 history
        let peers = self
            .get_peers_for_service(Services::KECCAK256_PROOFS, min_peers)
            .await?;

        // Try each peer until we get a valid response
        for peer_id in peers {
            log::debug!(
                peer_id = %peer_id,
                tx_hash = %tx_hash,
                macro_block_number = %macro_block_number,
                "Requesting Keccak256 transaction proof from peer",
            );

            let response = self
                .network
                .request::<RequestKeccak256TransactionProof>(
                    RequestKeccak256TransactionProof {
                        hash: tx_hash.clone(),
                        block_number: macro_block_number,
                    },
                    peer_id,
                )
                .await;

            match response {
                Ok(Ok(proof_response)) => {
                    log::debug!(
                        peer_id = %peer_id,
                        "Successfully obtained Keccak256 transaction proof from peer",
                    );
                    return Ok(proof_response);
                }
                Ok(Err(error)) => {
                    log::debug!(
                        peer_id = %peer_id,
                        %error,
                        "Peer could not provide Keccak256 transaction proof",
                    );
                    // Continue to next peer
                }
                Err(error) => {
                    log::debug!(
                        peer_id = %peer_id,
                        %error,
                        "Error requesting Keccak256 transaction proof from peer",
                    );
                    // Continue to next peer
                }
            }
        }

        // If we get here, all peers failed
        Err(RequestError::OutboundRequest(
            OutboundRequestError::NoReceiver,
        ))
    }

    /// Returns peers that support all requested services (e.g. full blocks, history, mempool).
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

    /// Requests and verifies historic transactions from peers based on receipt data, and returns a list of verified transactions,
    /// sorted by block number in descending order.
    pub async fn prove_transactions_from_receipts(
        &self,
        receipts: Vec<(Blake2bHash, Option<u32>)>,
        min_peers: usize,
    ) -> Result<Vec<HistoricTransaction>, RequestError> {
        let blockchain = self.blockchain.read();
        let election_head = blockchain.election_head().clone();
        let checkpoint_head = blockchain.macro_head().clone();
        let current_head_hash = blockchain.head_hash();
        let current_block_number = blockchain.block_number();

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

        // Map of the transaction hashes we are requesting to their (optionally) requested block
        // number. This is used to ensure that a peer only answers with transactions that we
        // actually asked for. A proof merely attests that a transaction is part of the chain
        // history (it carries no notion of which hashes were requested), so without this check a
        // malicious peer could answer a lookup for transaction A with a perfectly valid inclusion
        // proof for an unrelated transaction B that genuinely is in the chain, and we would accept
        // B as the result for A.
        let requested_transactions: HashMap<RawTransactionHash, Option<u32>> = receipts
            .iter()
            .map(|(hash, block_number)| (RawTransactionHash::from(hash.clone()), *block_number))
            .collect();

        let full_node_cutoff = election_head.block_number() - Policy::blocks_per_epoch() + 1;
        let can_query_full_nodes = receipts
            .iter()
            .all(|(_, block_number)| block_number.unwrap_or(0) > full_node_cutoff);
        let mut peer_required_service = if can_query_full_nodes {
            Services::FULL_BLOCKS
        } else {
            Services::TRANSACTION_INDEX
        };
        let include_pregenesis = receipts
            .iter()
            .any(|(_, block_number)| block_number.unwrap_or(0) < Policy::genesis_block_number());
        if include_pregenesis {
            peer_required_service |= Services::PRE_GENESIS_TRANSACTIONS;
        }

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
                    if Policy::is_election_block_at(*block_number) {
                        // 1st Case: Transactions in election blocks
                        hashes_by_block
                            .entry(Some(*block_number))
                            .or_insert(vec![])
                            .push(hash.clone());
                    } else if block_number < &election_head.block_number() {
                        // 2nd Case: Transactions from finalized epochs (but not in election blocks)
                        hashes_by_block
                            .entry(Some(Policy::election_block_after(*block_number)))
                            .or_insert(vec![])
                            .push(hash.clone());
                    } else if block_number <= &checkpoint_head.block_number() {
                        // 3rd Case: Transactions from a finalized batch in the current epoch
                        hashes_by_block
                            .entry(Some(checkpoint_head.block_number()))
                            .or_insert(vec![])
                            .push(hash.clone());
                    } else {
                        // 4th Case: Transactions from the current batch
                        hashes_by_block
                            .entry(Some(current_block_number))
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

                            if response.block.block_number() == Policy::genesis_block_number() {
                                let genesis_hash = self.blockchain.read().get_genesis_hash();

                                if genesis_hash == response.block.hash() {
                                    already_proven = true;
                                } else {
                                    log::warn!(peer = %peer_id, "The genesis hash from the peer does not match our own");
                                    continue;
                                }
                            } else if election_head.hash() == block_hash
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
                        } else if response.block.hash() != current_head_hash {
                            log::debug!(block_number = %response.block.block_number(), peer=%peer_id, "BlockProof does not correspond to expected block");
                            continue;
                        }

                        if verification_result {
                            for tx in response.proof.history {
                                let tx_hash = tx.tx_hash();

                                // Only accept transactions that we actually requested. This
                                // prevents a peer from substituting a valid proof for an unrelated
                                // transaction in response to our lookup.
                                let Some(&requested_block_number) =
                                    requested_transactions.get(&tx_hash)
                                else {
                                    log::warn!(peer = %peer_id, tx_hash = %*tx_hash, "Peer returned a proof for a transaction we did not request; ignoring it");
                                    continue;
                                };

                                // If the lookup was bound to a specific block number, the proven
                                // transaction must have occurred in exactly that block.
                                if requested_block_number.is_some_and(|bn| tx.block_number != bn) {
                                    log::warn!(peer = %peer_id, tx_hash = %*tx_hash, expected = ?requested_block_number, got = tx.block_number, "Peer returned a proof for a requested transaction but in an unexpected block; ignoring it");
                                    continue;
                                }

                                verified_transactions.insert(tx_hash, tx);
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

    /// Subscribes to the given addresses on one or more peers.
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
            Err(RequestError::OutboundRequest(
                OutboundRequestError::NoReceiver,
            ))
        }
    }

    /// Unsubscribes to the given addresses on one or more peers.
    pub async fn unsubscribe_from_addresses(
        &self,
        addresses: Vec<Address>,
        min_peers: usize,
    ) -> Result<(), RequestError> {
        // Unsubscribe given addresses from all peers
        // Note: this does not mean that we will fully unsubscribe from a peer,
        // we will unsubscribe only from the addresses that were supplied to this function
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

    /// Attempts to resolve a block with `block_hash` header hash at the given `block_height`.
    /// The first resolution attempt is performed with the peer specified by `first_peer_id`.
    ///
    /// This function fails, if the consensus cannot accept more requests or if the consensus drops
    /// the request on its side, generally indicating that it is no longer of use.
    pub async fn resolve_block(
        self,
        block_number: u32,
        block_hash: Blake2bHash,
        first_peer_id: N::PeerId,
    ) -> Result<Block, ResolveBlockError<N>> {
        // Create the oneshot sender whose receiver this fn will await and whose
        // sender will be given to the consensus proper to resolve the call.
        let (response_sender, receiver) = oneshot::channel();

        // Create the request structure.
        let request = ResolveBlockRequest {
            block_number,
            block_hash,
            first_peer_id,
            response_sender,
        };

        // Send the request to the consensus. If the send fails the resolve block fails.
        self.request
            .send(ConsensusRequest::ResolveBlock(request))
            .await
            .map_err(ResolveBlockError::<N>::SendError)?;

        // Wait for the consensus to resolve the request. The only error case is when the sender of
        // the channel drops in which case the resolve block request will fail.
        receiver.await.map_err(ResolveBlockError::ReceiveError)?
    }
}
