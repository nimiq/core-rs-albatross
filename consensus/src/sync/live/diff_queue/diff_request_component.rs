use std::{ops, sync::Arc};

use futures::future::BoxFuture;
use nimiq_block::Block;
use nimiq_network_interface::network::{Network, SubscribeEvents};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_diff::TrieDiff, TreeProof};
use parking_lot::RwLock;

use super::{RequestPartialDiff, ResponsePartialDiff};
use crate::sync::peer_list::PeerList;

/// Peer Tracking & Diff Request component.
pub struct DiffRequestComponent<N: Network> {
    network: Arc<N>,
    peers: Arc<RwLock<PeerList<N>>>,
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
        &self,
        range: ops::RangeTo<KeyNibbles>,
    ) -> impl FnMut(&Block) -> BoxFuture<'static, Result<TrieDiff, ()>> {
        let peers = Arc::clone(&self.peers);
        let network = Arc::clone(&self.network);
        move |block| {
            let peers = Arc::clone(&peers);
            let network = Arc::clone(&network);
            let range = range.clone();
            let block_hash = block.hash();
            let block_diff_root = block.diff_root().clone();
            Box::pin(async move {
                let peer_id = {
                    let peers = peers.read();
                    // TODO: select peer
                    peers.peers()[0]
                };
                match network
                    .request(RequestPartialDiff { block_hash, range }, peer_id)
                    .await
                {
                    Ok(ResponsePartialDiff::PartialDiff(diff)) => {
                        if TreeProof::new(diff.0.iter()).root_hash() != block_diff_root {
                            error!("couldn't fetch diff: invalid diff");
                            return Err(());
                        }
                        Ok(diff)
                    }
                    // TODO: remove peer, retry elsewhere
                    Ok(ResponsePartialDiff::IncompleteState) => {
                        error!("couldn't fetch diff: incomplete state");
                        Err(())
                    }
                    Ok(ResponsePartialDiff::UnknownBlockHash) => {
                        error!("couldn't fetch diff: unknown block hash");
                        Err(())
                    }
                    Err(e) => {
                        error!("couldn't fetch diff: {}", e);
                        Err(())
                    }
                }
            })
        }
    }
}
