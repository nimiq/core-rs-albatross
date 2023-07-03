use std::{ops, sync::Arc};

use futures::future::BoxFuture;
use nimiq_block::Block;
use nimiq_network_interface::network::Network;
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_diff::TrieDiff, TreeProof};
use parking_lot::RwLock;

use super::{RequestPartialDiff, ResponsePartialDiff};
use crate::sync::peer_list::{PeerList, PeerListIndex};

pub struct DiffRequestComponent<N: Network> {
    network: Arc<N>,
    peers: Arc<RwLock<PeerList<N>>>,
    current_peer_index: PeerListIndex,
}

impl<N: Network> DiffRequestComponent<N> {
    pub fn new(network: Arc<N>, peers: Arc<RwLock<PeerList<N>>>) -> Self {
        DiffRequestComponent {
            network,
            peers,
            current_peer_index: PeerListIndex::default(),
        }
    }

    pub fn request_diff(
        &mut self,
        range: ops::RangeTo<KeyNibbles>,
    ) -> impl FnMut(&Block) -> BoxFuture<'static, Result<TrieDiff, ()>> {
        let mut starting_peer_index = self.current_peer_index.clone();
        self.current_peer_index.increment();

        let peers = Arc::clone(&self.peers);
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
                        .request(
                            RequestPartialDiff {
                                block_hash: block_hash.clone(),
                                range: range.clone(),
                            },
                            peer_id,
                        )
                        .await
                    {
                        Ok(ResponsePartialDiff::PartialDiff(diff)) => {
                            if TreeProof::new(diff.0.iter()).root_hash() == block_diff_root {
                                return Ok(diff);
                            }
                            error!(%peer_id, "couldn't fetch diff: invalid diff");
                        }
                        // TODO: remove peer, retry elsewhere
                        Ok(ResponsePartialDiff::IncompleteState) => {
                            error!(%peer_id, "couldn't fetch diff: incomplete state")
                        }
                        Ok(ResponsePartialDiff::UnknownBlockHash) => {
                            error!(%peer_id, "couldn't fetch diff: unknown block hash")
                        }
                        Err(error) => error!(%peer_id, ?error, "couldn't fetch diff: {}", error),
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

    pub fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        Arc::clone(&self.peers)
    }
}
