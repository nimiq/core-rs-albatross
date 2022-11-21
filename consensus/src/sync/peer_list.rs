use std::{collections::HashSet, ops::Index, slice::SliceIndex};

use nimiq_network_interface::network::Network;

/// A list of peers to be used while syncing.
/// This contains an ordered list of peers as well as a hashmap.
/// This data structure ensures both are maintained consistently.
#[derive(Debug)]
pub struct PeerList<N: Network> {
    peers_set: HashSet<N::PeerId>,
    peers: Vec<N::PeerId>,
}

impl<N: Network> Default for PeerList<N> {
    fn default() -> Self {
        Self {
            peers_set: Default::default(),
            peers: Default::default(),
        }
    }
}

impl<N: Network> Clone for PeerList<N> {
    fn clone(&self) -> Self {
        Self {
            peers_set: self.peers_set.clone(),
            peers: self.peers.clone(),
        }
    }
}

impl<N: Network> PeerList<N> {
    pub fn add_peer(&mut self, peer_id: N::PeerId) -> bool {
        if self.peers_set.insert(peer_id) {
            self.peers.push(peer_id);
            return true;
        }
        false
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    pub fn has_peer(&self, peer_id: &N::PeerId) -> bool {
        self.peers_set.contains(peer_id)
    }

    pub fn peers(&self) -> &Vec<N::PeerId> {
        &self.peers
    }

    pub fn peers_set(&self) -> &HashSet<N::PeerId> {
        &self.peers_set
    }

    pub fn remove_peer(&mut self, peer_id: &N::PeerId) -> bool {
        if self.peers_set.remove(peer_id) {
            self.peers.retain(|element| element != peer_id);
            return true;
        }
        false
    }
}

impl<N: Network, I: SliceIndex<[N::PeerId]>> Index<I> for PeerList<N> {
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        self.peers.index(index)
    }
}
