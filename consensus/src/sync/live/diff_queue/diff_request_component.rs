use std::{ops, sync::Arc};

use futures::future::BoxFuture;
use nimiq_block::Block;
use nimiq_network_interface::network::{Network, SubscribeEvents};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_diff::TrieDiff, TreeProof};
use parking_lot::RwLock;

use super::{RequestPartialDiff, ResponsePartialDiff};
use crate::sync::peer_list::{PeerList, PeerListIndex};

/// Peer Tracking & Diff Request component.
pub struct DiffRequestComponent<N: Network> {
    network: Arc<N>,
    peers: Arc<RwLock<PeerList<N>>>,
    current_peer_index: PeerListIndex,
    network_event_rx: SubscribeEvents<N::PeerId>,
}

impl<N: Network> DiffRequestComponent<N> {
    pub fn new(
        network: Arc<N>,
        network_event_rx: SubscribeEvents<N::PeerId>,
        peers: Arc<RwLock<PeerList<N>>>,
    ) -> Self {
        DiffRequestComponent {
            network,
            peers,
            current_peer_index: PeerListIndex::default(),
            network_event_rx,
        }
    }

    pub fn add_peer(&mut self, peer_id: N::PeerId) -> bool {
        self.peers.write().add_peer(peer_id)
    }

    pub fn remove_peer(&mut self, peer_id: &N::PeerId) {
        self.peers.write().remove_peer(peer_id);
    }

    pub fn num_peers(&self) -> usize {
        self.peers.read().len()
    }

    pub fn len(&self) -> usize {
        self.peers.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        Arc::clone(&self.peers)
    }

    pub fn request_diff2(
        &mut self,
        range: ops::RangeTo<KeyNibbles>,
    ) -> impl FnMut(&Block) -> BoxFuture<'static, Result<TrieDiff, ()>> {
        let peers = Arc::clone(&self.peers);
        let mut starting_peer_index = self.current_peer_index.clone();
        self.current_peer_index.increment();
        let network = Arc::clone(&self.network);
        move |block| {
            let peers = Arc::clone(&peers);
            let mut current_peer_index = starting_peer_index.clone();
            starting_peer_index.increment();
            let network = Arc::clone(&network);
            let range = range.clone();
            let block_hash = block.hash();
            let block_diff_root = block.diff_root().clone();
            Box::pin(async move {
                let mut num_tries = 0;
                loop {
                    let peer_id = match peers.read().increment_and_get(&mut current_peer_index) {
                        Some(peer_id) => peer_id,
                        None => {
                            error!("couldn't fetch diff: no peers");
                            return Err(());
                        }
                    };
                    match network
                        .request(RequestPartialDiff {
                            block_hash: block_hash.clone(),
                            range: range.clone(),
                        }, peer_id)
                        .await
                    {
                        Ok(ResponsePartialDiff::PartialDiff(diff)) => {
                            if TreeProof::new(diff.0.iter()).root_hash() != block_diff_root {
                                error!("couldn't fetch diff: invalid diff");
                            }
                            return Ok(diff);
                        }
                        // TODO: remove peer, retry elsewhere
                        Ok(ResponsePartialDiff::IncompleteState) => error!("couldn't fetch diff: incomplete state"),
                        Ok(ResponsePartialDiff::UnknownBlockHash) => error!("couldn't fetch diff: unknown block hash"),
                        Err(e) => error!("couldn't fetch diff: {}", e),
                    }
                    num_tries += 1;
                    let max_tries = peers.read().len();
                    if num_tries >= max_tries {
                        error!(%num_tries, %max_tries, "couldn't fetch diff: maximum tries reached");
                        return Err(());
                    }
                }
            })
        }
    }
}
