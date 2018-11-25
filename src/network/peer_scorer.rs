use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::peer_address_state::PeerAddressState;
use crate::network::connection::close_type::CloseType;
use crate::network::connection::connection_pool::ConnectionPool;
use crate::network::network_config::NetworkConfig;
use crate::network::ProtocolFlags;
use crate::utils::services::ServiceFlags;

use std::sync::Arc;
use rand::Rng;
use rand::OsRng;

pub struct PeerScorer {
    network_config: Arc<NetworkConfig>,
    addresses: Arc<PeerAddressBook>,
    connections: Arc<ConnectionPool>
}

impl PeerScorer {
    const PEER_COUNT_MIN_OUTBOUND: usize = 6;
    const PICK_SELECTION_SIZE: usize = 100;

    pub fn new(network_config: Arc<NetworkConfig>, addresses: Arc<PeerAddressBook>, connections: Arc<ConnectionPool>) -> Self {
        return PeerScorer {
            network_config,
            addresses,
            connections
        }
    }

    pub fn pick_address(&self) -> Option<Arc<PeerAddress>> {
        let mut candidates = self.find_candidates(1000, false);
        if candidates.len() == 0 {
            candidates = self.find_candidates(1000, true);
        }
        candidates.sort_by(|a, b| { a.1.cmp(&b.1) });
        let mut randrng: OsRng = OsRng::new().unwrap();
        let rand_ind = randrng.gen_range(0, usize::min(PeerScorer::PICK_SELECTION_SIZE, candidates.len()));
        match candidates.get(rand_ind) {
            Some((peer_address, _)) => return Some(Arc::clone(peer_address)),
            None => return None
        }
    }

    fn find_candidates(&self, num_candidates: usize, allow_bad_peers: bool) -> Vec<(Arc<PeerAddress>, i32)> {
        let addresses_state = self.addresses.state();
        let address_iterator = addresses_state.address_iter_for_protocol_mask(self.network_config.protocol_mask().clone());
        let num_addresses = addresses_state.known_addresses_nr_for_protocol_mask(self.network_config.protocol_mask().clone());

        let mut start_index = 0;
        let mut end_index = 0;
        if num_addresses > num_candidates {
            let mut randrng: OsRng = OsRng::new().unwrap();
            start_index = randrng.gen_range(0, num_candidates);
            end_index = (start_index + num_candidates) % num_addresses;
        }
        let overflow = start_index > end_index;

        let mut candidates = Vec::new();
        let mut index  = 0;
        for address in address_iterator {
            if !overflow && index < start_index { continue; }
            if !overflow && index >= end_index { break; }
            if overflow && (index >= end_index && index < start_index) { continue; }

            let score = self.score_addresses(address);
            if score >= 0 {
                candidates.push( (Arc::clone(address), score));
                if candidates.len() >= num_candidates {
                    break;
                }
            }
            index += 1;
        }
        return candidates;
    }

    fn score_addresses(&self, peer_address: &Arc<PeerAddress>) -> i32 {
        let address_state = self.addresses.state();
        let peer_address_sopt = address_state.get_info(peer_address);
        match peer_address_sopt {
            None => 0,
            Some(peer_address_info) => {
                // Filter addresses that we cannot connect to (needed to filter out dumb peers).
                if !self.network_config.can_connect(peer_address.protocol()) {
                    return -1;
                }

                // Filter addresses not matching our accepted services.
                if (peer_address.services & self.network_config.services().accepted) == ServiceFlags::NONE {
                    return -1;
                }

                // Filter addresses that are too old.
                if peer_address.exceeds_age() {
                    return -1;
                }

                // A channel to that peer address is CONNECTING, CONNECTED, NEGOTIATING OR ESTABLISHED
                if let Some(_) = self.connections.state().get_connection_by_peer_address(peer_address) {
                    return -1;
                }

                // If we need more good peers, only allow good peers unless allowBadPeers is true.
                if self.needs_good_peers() && !self.is_good_peer(peer_address) {
                    return -1;
                }

                // Give all peers the same base score. Penalize peers with failed connection attempts.
                let score = 1;
                match peer_address_info.state {
                    PeerAddressState::Banned => -1,
                    PeerAddressState::New | PeerAddressState::Tried => score,
                    PeerAddressState::Failed => {
                        // Don't pick failed addresses when they have failed the maximum number of times.
                        (1 - ((peer_address_info.failed_attempts + 1) as i32 / peer_address_info.max_failed_attempts() as i32)) * score
                    },
                    default => -1
                }
            }
        }
    }

    pub fn is_good_peer_set(&self) -> bool {
        false
    }

    pub fn needs_good_peers(&self) -> bool {
        false
    }

    pub fn needs_more_peers(&self) -> bool {
        true
    }

    pub fn is_good_peer(&self, peer_address: &Arc<PeerAddress>) -> bool {
        true
    }

    pub fn score_connections(&self) {

    }

    pub fn recycle_connections(&self, count: u32, ty: CloseType, reason: &str) {

    }

    pub fn lowest_connection_score(&self) -> Option<f32> {
        None
    }
}
