use std::collections::HashSet;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};

use nimiq_block::Block;
use nimiq_blockchain::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network, request::RequestError};

use crate::messages::{RequestBlock, RequestHead};

/// Requests the head blocks for a set of peers.
/// Calculates the number of known/unknown blocks and a vector of unknown blocks.
pub struct HeadRequests<TNetwork: Network + 'static> {
    peers: Vec<TNetwork::PeerId>,
    head_hashes: FuturesUnordered<BoxFuture<'static, (usize, Result<Blake2bHash, RequestError>)>>,
    head_blocks: FuturesUnordered<
        BoxFuture<'static, (Result<Option<Block>, RequestError>, TNetwork::PeerId)>,
    >,
    requested_hashes: HashSet<Blake2bHash>,
    blockchain: BlockchainProxy,
    network: Arc<TNetwork>,
    num_known_blocks: usize,
    num_unknown_blocks: usize,
    unknown_blocks: Vec<(Block, TNetwork::PeerId)>,
    include_micro_bodies: bool,
}

pub struct HeadRequestsResult<TNetwork: Network + 'static> {
    pub num_known_blocks: usize,
    pub num_unknown_blocks: usize,
    pub unknown_blocks: Vec<(Block, TNetwork::PeerId)>,
}

impl<TNetwork: Network + 'static> HeadRequests<TNetwork> {
    pub fn new(
        peers: Vec<TNetwork::PeerId>,
        network: Arc<TNetwork>,
        blockchain: BlockchainProxy,
    ) -> Self {
        let head_hashes = peers
            .iter()
            .enumerate()
            .map(|(i, peer_id)| {
                let peer_id = *peer_id;
                let network = Arc::clone(&network);
                async move { (i, Self::request_head(network, peer_id).await) }.boxed()
            })
            .collect();

        let include_micro_bodies = matches!(blockchain, BlockchainProxy::Full(_));

        HeadRequests {
            peers,
            head_hashes,
            head_blocks: Default::default(),
            requested_hashes: Default::default(),
            blockchain,
            network,
            num_known_blocks: 0,
            num_unknown_blocks: 0,
            unknown_blocks: Default::default(),
            include_micro_bodies,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.head_hashes.is_empty() && self.head_blocks.is_empty()
    }

    async fn request_head(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
    ) -> Result<Blake2bHash, RequestError> {
        network
            .request::<RequestHead>(RequestHead {}, peer_id)
            .await
    }

    async fn request_block(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        hash: Blake2bHash,
        include_micro_bodies: bool,
    ) -> Result<Option<Block>, RequestError> {
        network
            .request::<RequestBlock>(
                RequestBlock {
                    hash,
                    include_micro_bodies,
                },
                peer_id,
            )
            .await
    }
}

impl<TNetwork: Network + 'static> Future for HeadRequests<TNetwork> {
    type Output = HeadRequestsResult<TNetwork>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // We poll the hashes first.
        while let Poll::Ready(Some((i, result))) = self.head_hashes.poll_next_unpin(cx) {
            // If we got a result, check it and classify it as known block/unknown block.
            match result {
                Ok(hash) => {
                    if self.blockchain.read().get_block(&hash, false, None).is_ok() {
                        self.num_known_blocks += 1;
                    } else {
                        // Request unknown blocks from peer that gave it to us.
                        self.num_unknown_blocks += 1;
                        if !self.requested_hashes.contains(&hash) {
                            self.requested_hashes.insert(hash.clone());
                            let network = Arc::clone(&self.network);
                            let peer_id = self.peers[i];
                            let include_micro_bodies = self.include_micro_bodies;
                            self.head_blocks.push(
                                async move {
                                    (
                                        Self::request_block(
                                            network,
                                            peer_id,
                                            hash,
                                            include_micro_bodies,
                                        )
                                        .await,
                                        peer_id,
                                    )
                                }
                                .boxed(),
                            );
                        }
                    }
                }
                Err(_) => {
                    trace!("Failed head hash request");
                } // We don't count failed requests.
            }
        }

        // Then poll blocks.
        while let Poll::Ready(Some(result)) = self.head_blocks.poll_next_unpin(cx) {
            match result {
                (Ok(Some(block)), peer_id) => {
                    self.unknown_blocks.push((block, peer_id));
                }
                _ => {
                    trace!("Failed block request");
                } // We don't do anything with failed requests.
            }
        }

        // We're done if both queues are empty.
        if self.is_finished() {
            return Poll::Ready(HeadRequestsResult {
                num_known_blocks: self.num_known_blocks,
                num_unknown_blocks: self.num_unknown_blocks,
                unknown_blocks: mem::take(&mut self.unknown_blocks),
            });
        }

        Poll::Pending
    }
}
