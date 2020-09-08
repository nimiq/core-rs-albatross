use std::{sync::Arc, time::Duration};

use rand::rngs::OsRng;
use rand::Rng;

use peer_address::{address::PeerAddress, protocol::Protocol, services::ServiceFlags};

use crate::address::peer_address_book::PeerAddressBookState;
use crate::{
    address::{peer_address_book::PeerAddressBook, peer_address_state::PeerAddressState},
    connection::{
        close_type::CloseType,
        connection_info::{ConnectionInfo, ConnectionState},
        connection_pool::{ConnectionId, ConnectionPool},
        network_agent::NetworkAgent,
    },
    network_config::NetworkConfig,
};
use parking_lot::RwLockReadGuard;

pub type Score = f64;

pub struct PeerScorer {
    network_config: Arc<NetworkConfig>,
    addresses: Arc<PeerAddressBook>,
    connections: Arc<ConnectionPool>,
    connection_scores: Vec<(ConnectionId, Score)>,
}

impl PeerScorer {
    const PEER_COUNT_MIN_FULL_WS_OUTBOUND: usize = 1; // FIXME: this is fixed to the "node.js" value since we don't support browsers in the Rust impl yet
    const PEER_COUNT_MIN_OUTBOUND: usize = 6; // FIXME: this is fixed to the "node.js" value since we don't support browsers in the Rust impl yet

    const PICK_SELECTION_SIZE: usize = 100;

    const MIN_AGE_FULL: Duration = Duration::from_secs(5 * 60); // 5 minutes
    const BEST_AGE_FULL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours

    const MIN_AGE_LIGHT: Duration = Duration::from_secs(2 * 60); // 2 minutes
    const BEST_AGE_LIGHT: Duration = Duration::from_secs(15 * 60); // 15 minutes
    const MAX_AGE_LIGHT: Duration = Duration::from_secs(6 * 60 * 60); // 6 hours

    const MIN_AGE_NANO: Duration = Duration::from_secs(60); // 1 minute
    const BEST_AGE_NANO: Duration = Duration::from_secs(5 * 60); // 5 minutes
    const MAX_AGE_NANO: Duration = Duration::from_secs(30 * 60); // 30 minutes

    const BEST_PROTOCOL_WS_DISTRIBUTION: f64 = 0.15; // 15%

    pub fn new(
        network_config: Arc<NetworkConfig>,
        addresses: Arc<PeerAddressBook>,
        connections: Arc<ConnectionPool>,
    ) -> Self {
        PeerScorer {
            network_config,
            addresses,
            connections,
            connection_scores: Vec::new(),
        }
    }

    pub fn pick_address(&self) -> Option<Arc<PeerAddress>> {
        let mut candidates = self.find_candidates(1000, false);
        if candidates.is_empty() {
            candidates = self.find_candidates(1000, true);
        }
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by(|a, b| a.1.cmp(&b.1));
        let rand_ind = OsRng.gen_range(0, usize::min(Self::PICK_SELECTION_SIZE, candidates.len()));
        match candidates.get(rand_ind) {
            Some((peer_address, _)) => Some(Arc::clone(peer_address)),
            None => None,
        }
    }

    fn find_candidates(
        &self,
        num_candidates: usize,
        allow_bad_peers: bool,
    ) -> Vec<(Arc<PeerAddress>, i32)> {
        let addresses_state = self.addresses.state();
        let address_iterator =
            addresses_state.address_iter_for_protocol_mask(self.network_config.protocol_mask());
        let num_addresses = addresses_state
            .known_addresses_nr_for_protocol_mask(self.network_config.protocol_mask());

        let (start_index, end_index) = if num_addresses > num_candidates {
            let start = OsRng.gen_range(0, num_addresses);
            (start, (start + num_candidates) % num_addresses)
        } else {
            (0, num_addresses)
        };
        let overflow = start_index > end_index;

        let mut candidates = Vec::new();
        for (index, address) in address_iterator.enumerate() {
            if !overflow && index < start_index {
                continue;
            }
            if !overflow && index >= end_index {
                break;
            }
            if overflow && (index >= end_index && index < start_index) {
                continue;
            }

            let score = self.score_address(address, allow_bad_peers, &addresses_state);
            if score >= 0 {
                candidates.push((Arc::clone(address), score));
                if candidates.len() >= num_candidates {
                    break;
                }
            }
        }
        candidates
    }

    fn score_address(
        &self,
        peer_address: &Arc<PeerAddress>,
        allow_bad_peers: bool,
        address_state: &RwLockReadGuard<PeerAddressBookState>,
    ) -> i32 {
        let peer_address_sopt = address_state.get_info(peer_address);
        match peer_address_sopt {
            None => 0,
            Some(peer_address_info) => {
                // Filter addresses that we cannot connect to (needed to filter out dumb peers).
                if !self.network_config.can_connect(peer_address.protocol()) {
                    return -1;
                }

                // Filter addresses not matching our accepted services.
                if (peer_address.services & self.network_config.services().accepted)
                    == ServiceFlags::NONE
                {
                    return -1;
                }

                // Filter addresses that are too old.
                if peer_address.exceeds_age() {
                    return -1;
                }

                // A channel to that peer address is CONNECTING, CONNECTED, NEGOTIATING OR ESTABLISHED
                if self
                    .connections
                    .state()
                    .get_connection_by_peer_address(peer_address)
                    .is_some()
                {
                    return -1;
                }

                // If we need more good peers, only allow good peers unless allowBadPeers is true.
                if self.needs_good_peers() && (!self.is_good_peer(peer_address) && !allow_bad_peers)
                {
                    return -1;
                }

                // Give all peers the same base score. Penalize peers with failed connection attempts.
                let score = 1;
                match peer_address_info.state {
                    PeerAddressState::Banned => -1,
                    PeerAddressState::New | PeerAddressState::Tried => score,
                    PeerAddressState::Failed => {
                        // Don't pick failed addresses when they have failed the maximum number of times.
                        (1 - ((peer_address_info.failed_attempts + 1) as i32
                            / peer_address_info.max_failed_attempts() as i32))
                            * score
                    }
                    _ => -1,
                }
            }
        }
    }

    pub fn is_good_peer_set(&self) -> bool {
        !self.needs_good_peers() && !self.needs_more_peers()
    }

    pub fn needs_good_peers(&self) -> bool {
        self.connections.state().get_peer_count_full_ws_outbound()
            < Self::PEER_COUNT_MIN_FULL_WS_OUTBOUND
    }

    pub fn needs_more_peers(&self) -> bool {
        self.connections.state().get_peer_count_outbound() < Self::PEER_COUNT_MIN_OUTBOUND
    }

    pub fn is_good_peer(&self, peer_address: &Arc<PeerAddress>) -> bool {
        peer_address.services.is_full_node()
            && (peer_address.protocol() == Protocol::Ws || peer_address.protocol() == Protocol::Wss)
    }

    pub fn score_connections(&mut self) {
        let mut connection_scores: Vec<(ConnectionId, Score)> = Vec::new();

        let state = self.connections.state();
        let distribution: f64 =
            (state.peer_count_ws as f64 + state.peer_count_wss as f64) / state.peer_count() as f64;
        let peer_count_full_ws_outbound = state.get_peer_count_full_ws_outbound();
        let connections: Vec<(ConnectionId, &ConnectionInfo)> = state.id_and_connection_iter();

        for connection in connections {
            if connection.1.state() == ConnectionState::Established
                && connection.1.age_established()
                    > self.get_min_age(connection.1.peer_address().expect("No peer address"))
            {
                let score =
                    Self::score_connection(connection.1, distribution, peer_count_full_ws_outbound);
                connection_scores.push((connection.0, score));
            }
        }

        connection_scores.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        self.connection_scores = connection_scores
    }

    pub fn recycle_connections(&mut self, mut count: u32, ty: CloseType, reason: &str) {
        while count > 0 && !self.connection_scores.is_empty() {
            let connection_id = self
                .connection_scores
                .pop()
                .map(|(connection_id, _)| connection_id)
                .unwrap();
            let state = self.connections.state();
            let connection_info = state
                .get_connection(connection_id)
                .expect("Missing connection");

            if connection_info.state() == ConnectionState::Established {
                connection_info
                    .peer_channel()
                    .expect("Missing PeerChannel")
                    .close(ty); // FIXME: what about `reason`?
                debug!("Closed connection with reason: {}", reason);
                count -= 1;
            }
        }
    }

    fn score_connection(
        connection_info: &ConnectionInfo,
        distribution: f64,
        peer_count_full_ws_outbound: usize,
    ) -> Score {
        // Connection age
        let score_age = Self::score_connection_age(connection_info);

        // Connection type (inbound/outbound)
        let score_outbound = if connection_info
            .network_connection()
            .expect("Missing network connection")
            .outbound()
        {
            0.0
        } else {
            1.0
        };

        let peer_address = connection_info
            .peer_address()
            .expect("Missing peer address");

        // Node type (full/light/nano)
        let score_type: Score;
        if peer_address.services.is_full_node() {
            score_type = 1.0;
        } else if peer_address.services.is_light_node() {
            score_type = 0.5;
        } else {
            score_type = 0.0;
        }

        // Protocol: Prefer WebSocket over WebRTC over Dumb.
        let score_protocol: Score = match peer_address.protocol() {
            Protocol::Wss | Protocol::Ws => {
                // Boost WebSocket score when low on WebSocket connections.
                if distribution < Self::BEST_PROTOCOL_WS_DISTRIBUTION
                    || peer_count_full_ws_outbound <= Self::PEER_COUNT_MIN_FULL_WS_OUTBOUND
                {
                    1.0
                } else {
                    0.6
                }
            }
            Protocol::Rtc => 0.3,
            Protocol::Dumb => 0.0,
        };

        // Connection speed, based on ping-pong latency median
        let median_latency = connection_info.statistics().latency_median();
        let score_speed: f64 = if median_latency > 0.0
            && median_latency < NetworkAgent::PING_TIMEOUT.as_secs() as f64
        {
            1.0 - median_latency / NetworkAgent::PING_TIMEOUT.as_secs() as f64
        } else {
            0.0
        };

        0.15 * score_age
            + 0.25 * score_outbound
            + 0.2 * score_type
            + 0.2 * score_protocol
            + 0.2 * score_speed
    }

    fn score_by_age(age: u128, best_age: u128, max_age: u128) -> Score {
        f64::max(
            f64::min(1. - (age as f64 - best_age as f64) / max_age as f64, 1.),
            0.,
        )
    }

    fn score_connection_age(connection_info: &ConnectionInfo) -> Score {
        let age = connection_info.age_established().as_millis();
        let services = connection_info
            .peer_address()
            .expect("No peer address")
            .services;

        if services.is_full_node() {
            (age as f64 / (2.0 * (Self::BEST_AGE_FULL.as_millis()) as f64) + 0.5) as Score
        } else if services.is_light_node() {
            Self::score_by_age(
                age,
                Self::BEST_AGE_LIGHT.as_millis(),
                Self::MAX_AGE_LIGHT.as_millis(),
            ) as Score
        } else {
            Self::score_by_age(
                age,
                Self::BEST_AGE_NANO.as_millis(),
                Self::MAX_AGE_NANO.as_millis(),
            ) as Score
        }
    }

    fn get_min_age(&self, peer_address: Arc<PeerAddress>) -> Duration {
        if peer_address.services.is_full_node() {
            Self::MIN_AGE_FULL
        } else if peer_address.services.is_light_node() {
            Self::MIN_AGE_LIGHT
        } else {
            Self::MIN_AGE_NANO
        }
    }

    pub fn lowest_connection_score(&mut self) -> Option<Score> {
        while !self.connection_scores.is_empty() {
            let connection_id = self
                .connection_scores
                .last()
                .map(|(connection_id, _)| *connection_id)
                .unwrap();
            let state = self.connections.state();
            let connection_info = state
                .get_connection(connection_id)
                .expect("Missing connection");

            if connection_info.state() == ConnectionState::Established {
                self.connection_scores.pop();
            }
        }

        match self.connection_scores.last() {
            None => None,
            Some(tuple) => Some(tuple.1),
        }
    }

    pub fn connection_scores(&self) -> &Vec<(ConnectionId, Score)> {
        &self.connection_scores
    }
}
