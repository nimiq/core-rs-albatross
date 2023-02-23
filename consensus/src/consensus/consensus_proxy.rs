use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast::Sender as BroadcastSender;
use tokio_stream::wrappers::BroadcastStream;

use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_keys::Address;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    peer_info::Services,
    request::{OutboundRequestError, RequestError},
};
use nimiq_primitives::account::AccountType;
use nimiq_transaction::{
    extended_transaction::ExtendedTransaction, ControlTransactionTopic, Transaction,
    TransactionTopic,
};

use crate::messages::{RequestTransactionsByAddress, RequestTransactionsProof};
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
            let response = self
                .network
                .request::<RequestTransactionsByAddress>(
                    RequestTransactionsByAddress {
                        address: address.clone(),
                        max,
                    },
                    peer_id,
                )
                .await;

            match response {
                Ok(response) => {
                    // Now we request proofs for each transaction we requested.
                    for transaction in response.transactions {
                        // If the transaction was already verified, then we don't need to verify it again
                        if verified_transactions.contains_key(&transaction.tx_hash()) {
                            continue;
                        }

                        let response = self
                            .network
                            .request::<RequestTransactionsProof>(
                                RequestTransactionsProof {
                                    hashes: vec![transaction.tx_hash()],
                                    block_number: transaction.block_number,
                                },
                                peer_id,
                            )
                            .await;
                        match response {
                            Ok(proof_response) => {
                                // We verify the transaction using the proof
                                if let Some(proof) = proof_response.proof {
                                    if let Some(block) = proof_response.block {
                                        // TODO: We are currently assuming that the provided block was included in the chain
                                        // but we also need some additional information to prove the block is part of the chain.
                                        let verification_result = proof
                                            .verify(block.history_root().clone())
                                            .map_or(false, |result| result);

                                        if verification_result {
                                            verified_transactions
                                                .insert(transaction.tx_hash(), transaction);
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

        Ok(verified_transactions.into_values().collect())
    }
}
