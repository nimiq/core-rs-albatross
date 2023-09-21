use std::{collections::HashSet, fmt, ops::Index, slice::SliceIndex};

use nimiq_network_interface::network::Network;

/// A list of peers to be used while syncing.
/// This contains an ordered list of peers as well as a hashmap.
/// This data structure ensures both are maintained consistently.
#[derive(Debug)]
pub struct PeerList<N: Network> {
    peers_set: HashSet<N::PeerId>,
    peers: Vec<N::PeerId>,
}

/// Stores an index into a [`PeerList`].
///
/// [`PeerList`]s can change at any time, this index can be used with
/// [`PeerList::increment_and_get`] to get varying peers.
#[derive(Clone)]
pub struct PeerListIndex {
    index: usize,
}

impl Default for PeerListIndex {
    fn default() -> PeerListIndex {
        // This works because `usize::MAX.wrapping_add(1)` is `0`.
        PeerListIndex { index: usize::MAX }
    }
}

impl fmt::Debug for PeerListIndex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.index == usize::MAX {
            "none".fmt(f)
        } else {
            self.index.fmt(f)
        }
    }
}

impl fmt::Display for PeerListIndex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl PeerListIndex {
    pub fn new(index: usize) -> Self {
        PeerListIndex { index }
    }

    pub fn increment(&mut self) {
        self.index = self.index.wrapping_add(1);
    }
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

    pub fn index_of(&self, peer_id: &N::PeerId) -> Option<PeerListIndex> {
        self.peers
            .iter()
            .position(|id| id == peer_id)
            .map(PeerListIndex::new)
    }

    pub fn get(&self, peer_index: &PeerListIndex) -> Option<N::PeerId> {
        if self.peers.is_empty() {
            return None;
        }
        let index = peer_index.index % self.peers.len();
        Some(self.peers[index])
    }

    pub fn increment_and_get(&self, peer_index: &mut PeerListIndex) -> Option<N::PeerId> {
        if self.peers.is_empty() {
            return None;
        }
        peer_index.index = peer_index.index.wrapping_add(1) % self.peers.len();
        Some(self.peers[peer_index.index])
    }
}

impl<N: Network, I: SliceIndex<[N::PeerId]>> Index<I> for PeerList<N> {
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        self.peers.index(index)
    }
}
