use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nimiq_blockchain_interface::AbstractBlockchain;
use tokio::sync::broadcast::Sender as BroadcastSender;
use tokio_stream::wrappers::BroadcastStream;

use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_keys::Address;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    peer_info::Services,
    request::{OutboundRequestError, RequestError},
};
use nimiq_primitives::{account::AccountType, policy::Policy};
use nimiq_transaction::{
    extended_transaction::ExtendedTransaction, ControlTransactionTopic, Transaction,
    TransactionTopic,
};

use crate::messages::{RequestTransactionReceiptsByAddress, RequestTransactionsProof};
use crate::ConsensusEvent;

pub struct ConsensusProxy<N: Network> {
    pub blockchain: BlockchainProxy,
    pub network: Arc<N>,
    pub(crate) established_flag: Arc<AtomicBool>,
    pub(crate) events: BroadcastSender<ConsensusEvent>,
}

impl<N: Network> Clone for ConsensusProxy<N> {
    fn clone(&self) -> Self {
        Self {
            blockchain: self.blockchain.clone(),
            network: Arc::clone(&self.network),
            established_flag: Arc::clone(&self.established_flag),
            events: self.events.clone(),
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

    pub async fn request_transactions_by_address(
        &self,
        address: Address,
        min_peers: usize,
        max: Option<u16>,
    ) -> Result<Vec<ExtendedTransaction>, RequestError> {
        // First we tell the network to provide us with a vector that contains all the connected peers that support such services
        // Note: If the network could not provide enough peers that satisfies our requirement, then an error would be returned
        let peers = self
            .network
            .get_peers_by_services(Services::TRANSACTION_INDEX, min_peers)
            .await
            .map_err(|error| {
                log::error!(
                    err = %error,
                    "The transactions by address request couldn't be fulfilled"
                );

                RequestError::OutboundRequest(OutboundRequestError::SendError)
            })?;

        let mut verified_transactions = HashMap::new();

        // At this point we obtained a list of connected peers that could satisfy our request,
        // so we perform the request to each of those peers:
        for peer_id in peers {
            log::debug!(
                peer_id = %peer_id,
                "Performing txns by address request to peer",
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

                    let blockchain = self.blockchain.read();

                    // Group transaction hashes by the block number that proves those transactions to reduce the number of requests
                    // There are three categories of block numbers:
                    //  - Finalized epochs: we use the election block number that finalized the respective epoch
                    //  - Finalized batch in the current epoch: We use the latest checkpoint block number
                    //  - Current batch: We use the current head to prove those transactions

                    // This is the structure where we group transactions by their proving block number
                    let mut hashes_by_block = HashMap::new();
                    let election_head_number = blockchain.election_head().block_number();
                    let checkpoint_head_number = blockchain.macro_head().block_number();
                    let current_head_number = blockchain.head().block_number();

                    for (hash, block_number) in response.receipts {
                        // If the transaction was already verified, then we don't need to verify it again
                        if verified_transactions.contains_key(&hash) {
                            continue;
                        }

                        if block_number <= election_head_number {
                            // First Case: Transactions from finalized epochs
                            hashes_by_block
                                .entry(Policy::election_block_after(block_number))
                                .or_insert(vec![])
                                .push(hash);
                        } else if block_number <= checkpoint_head_number {
                            // Second Case: Transactions from a finalized batch in the current epoch
                            hashes_by_block
                                .entry(checkpoint_head_number)
                                .or_insert(vec![])
                                .push(hash);
                        } else {
                            // Third Case: Transanctions from the current batch
                            hashes_by_block
                                .entry(current_head_number)
                                .or_insert(vec![])
                                .push(hash);
                        }
                    }

                    // We drop the blockchain lock because is no longer needed while we request proofs
                    drop(blockchain);

                    // Now we request proofs for each block and its hashes, according to its classification
                    for (block_number, hashes) in hashes_by_block {
                        log::debug!(
                            block_number=%block_number,
                            "Performing txn proof requests for block number",
                        );
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
                            Ok(proof_response) => {
                                // We verify the transaction using the proof
                                if let Some(proof) = proof_response.proof {
                                    if let Some(block) = proof_response.block {
                                        log::debug!(peer=%peer_id,"New txns proof and block from peer");

                                        // TODO: We are currently assuming that the provided block was included in the chain
                                        // but we also need some additional information to prove the block is part of the chain.
                                        let verification_result = proof
                                            .verify(block.history_root().clone())
                                            .map_or(false, |result| result);

                                        if verification_result {
                                            for tx in proof.history {
                                                verified_transactions.insert(tx.tx_hash(), tx);
                                            }
                                        } else {
                                            // The proof didn't verify so we disconnect from this peer
                                            log::debug!(peer=%peer_id,"Disconnecting from peer because the transaction proof didn't verify");
                                            self.network
                                                .disconnect_peer(peer_id, CloseReason::Other)
                                                .await;
                                            break;
                                        }
                                    } else {
                                        // If we receive a proof but we do not recieve a block, we disconnect from the peer
                                        log::debug!(peer=%peer_id,"Disconnecting from peer due to an inconsistency in the transaction proof response");
                                        self.network
                                            .disconnect_peer(peer_id, CloseReason::Other)
                                            .await;
                                        break;
                                    }
                                } else {
                                    log::debug!(peer=%peer_id, "We requested a transaction proof but the peer didn't provide any");
                                }
                            }
                            Err(error) => {
                                // If there was a request error with this peer we don't request anymore proofs from it
                                log::error!(peer=%peer_id, err=%error,"There was an error requesting transactions proof from peer");
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    // If there was a request error with this peer we log an error
                    log::error!(peer=%peer_id, err=%error,"There was an error requesting transactions from peer");
                }
            }
        }

        // Sort transactions by block_number
        let mut transactions: Vec<ExtendedTransaction> =
            verified_transactions.into_values().collect();
        transactions.sort_unstable_by_key(|ext_tx| ext_tx.block_number);
        transactions.reverse(); // Return newest transaction (highest block_number) first

        Ok(transactions)
    }
}
