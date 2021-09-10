use std::collections::HashSet;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Weak};

use parking_lot::RwLock;

use crate::consensus_agent::ConsensusAgent;
use block::Block;
use blockchain::{AbstractBlockchain, Blockchain};
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::task::{Context, Poll};
use futures::{Future, FutureExt, StreamExt};
use hash::Blake2bHash;
use network_interface::{peer::Peer, request_response::RequestError};

/// Requests the head blocks for a set of peers.
/// Calculates the number of known/unknown blocks and a vector of unknown blocks.
pub struct HeadRequests<TPeer: Peer + 'static> {
    peers: Vec<Arc<ConsensusAgent<TPeer>>>,
    head_hashes: FuturesUnordered<BoxFuture<'static, (usize, Result<Blake2bHash, RequestError>)>>,
    head_blocks:
        FuturesUnordered<BoxFuture<'static, (Result<Option<Block>, RequestError>, TPeer::Id)>>,
    requested_hashes: HashSet<Blake2bHash>,
    blockchain: Arc<RwLock<Blockchain>>,
    num_known_blocks: usize,
    num_unknown_blocks: usize,
    unknown_blocks: Vec<(Block, TPeer::Id)>,
}

pub struct HeadRequestsResult<TPeer: Peer + 'static> {
    pub num_known_blocks: usize,
    pub num_unknown_blocks: usize,
    pub unknown_blocks: Vec<(Block, TPeer::Id)>,
}

impl<TPeer: Peer + 'static> HeadRequests<TPeer> {
    pub fn new(
        peers: Vec<Weak<ConsensusAgent<TPeer>>>,
        blockchain: Arc<RwLock<Blockchain>>,
    ) -> Self {
        let peers: Vec<_> = peers
            .into_iter()
            .filter_map(|peer| peer.upgrade())
            .collect();

        let head_hashes = peers
            .iter()
            .enumerate()
            .map(|(i, peer)| {
                let peer = Arc::clone(peer);
                async move { (i, peer.request_head().await) }.boxed()
            })
            .collect();

        HeadRequests {
            peers,
            head_hashes,
            head_blocks: Default::default(),
            requested_hashes: Default::default(),
            blockchain,
            num_known_blocks: 0,
            num_unknown_blocks: 0,
            unknown_blocks: Default::default(),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.head_hashes.is_empty() && self.head_blocks.is_empty()
    }
}

impl<TPeer: Peer + 'static> Future for HeadRequests<TPeer> {
    type Output = HeadRequestsResult<TPeer>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // We poll the hashes first.
        while let Poll::Ready(Some((i, result))) = self.head_hashes.poll_next_unpin(cx) {
            // If we got a result, check it and classify it as known block/unknown block.
            match result {
                Ok(hash) => {
                    if self
                        .blockchain
                        .read()
                        .get_block(&hash, false, None)
                        .is_some()
                    {
                        self.num_known_blocks += 1;
                    } else {
                        // Request unknown blocks from peer that gave it to us.
                        self.num_unknown_blocks += 1;
                        if !self.requested_hashes.contains(&hash) {
                            self.requested_hashes.insert(hash.clone());
                            let peer = Arc::clone(&self.peers[i]);
                            self.head_blocks.push(
                                async move { (peer.request_block(hash).await, peer.peer.id()) }
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
