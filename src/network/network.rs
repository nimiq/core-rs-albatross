use atomic::Atomic;
use atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use parking_lot::RwLock;

use crate::consensus::base::blockchain::Blockchain;
use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::connection::close_type::CloseType;
use crate::network::connection::connection_pool::ConnectionPool;
use crate::network::connection::connection_pool::ConnectionPoolEvent;
use crate::network::connection::connection_info::ConnectionState;
use crate::network::network_config::NetworkConfig;
use crate::network::NetworkTime;
use crate::network::Peer;
use crate::network::peer_scorer::PeerScorer;
use crate::utils::timers::Timers;

pub struct Network {
    network_config: Arc<NetworkConfig>,
    network_time: Arc<NetworkTime>,
    auto_connect: Arc<Atomic<bool>>,
    backed_off: Arc<Atomic<bool>>,
    backoff: Arc<Atomic<Duration>>,
    addresses: Arc<RwLock<PeerAddressBook>>,
    connections: Arc<RwLock<ConnectionPool>>,
    scorer: Arc<RwLock<PeerScorer>>,
    timers: Arc<Timers<String>>
}

impl Network {
    const PEER_COUNT_MAX: usize = 4000;
    const PEER_COUNT_RECYCLING_ACTIVE: usize = 1000;
    const RECYCLING_PERCENTAGE_MIN: f32 = 0.01;
    const RECYCLING_PERCENTAGE_MAX: f32 = 0.20;
    const CONNECTING_COUNT_MAX: usize = 2;
    const CONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
    const CONNECT_BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);
    const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(5 * 60);
    const SCORE_INBOUND_EXCHANGE: f32 = 0.5;
    const CONNECT_THROTTLE: Duration = Duration::from_secs(1);

    pub fn new(blockchain: Arc<Blockchain<'static>>, network_config: Arc<NetworkConfig>, network_time: Arc<NetworkTime>) -> Arc<Self> {
        let addresses = Arc::new(RwLock::new(PeerAddressBook::new(network_config.clone())));
        let network = Arc::new(Network {
            network_config: Arc::clone(&network_config),
            network_time,
            auto_connect: Arc::new(Atomic::new(false)),
            backed_off: Arc::new(Atomic::new(false)),
            backoff: Arc::new(Atomic::new(Network::CONNECT_BACKOFF_INITIAL)),
            addresses: Arc::clone(&addresses),
            connections: ConnectionPool::new(Arc::clone(&addresses), Arc::clone(&network_config), blockchain),
            scorer: Arc::new(RwLock::new(PeerScorer::new(Arc::clone(&addresses)))),
            timers: Arc::new(Timers::new())
        });

        let network_clone = Arc::clone(&network);

        network.connections.write().notifier.register(move |event: &ConnectionPoolEvent| {
            let network = Arc::clone(&network_clone);
            match event {
                ConnectionPoolEvent::PeerJoined(peer) => {
                    Network::on_peer_joined(network, peer);
                },
                ConnectionPoolEvent::PeerLeft(peer) => {
                    Network::on_peer_left(network, peer);
                },
                ConnectionPoolEvent::PeersChanged => {
                    Network::on_peers_changed(network);
                },
                ConnectionPoolEvent::RecyclingRequest => {
                    Network::on_recycling_request(network);
                },
                ConnectionPoolEvent::ConnectError(_, _) => {
                    Network::on_connect_error(network);
                },
                default => {}
            }
        });

        network
    }

    pub fn connect(&mut self) {
        self.auto_connect.store(true, Ordering::Relaxed);

        let connections = Arc::clone(&self.connections);
        let scorer = Arc::clone(&self.scorer);

        self.timers.set_interval("network-housekeeping".to_string(), move || {
            Network::housekeeping(Arc::clone(&connections), Arc::clone(&scorer));
        }, Network::HOUSEKEEPING_INTERVAL);

        // Start connecting to peers.
        self.check_peer_count();
    }

    pub fn disconnect(&mut self) {
        self.auto_connect.store(false, Ordering::Relaxed);

        self.timers.clear_interval(&"network-housekeeping".to_string());

        self.connections.read().disconnect();
        self.connections.write().allow_inbound_exchange = false;
    }

    fn on_peer_joined(network: Arc<Network>, peer: &Peer) {
        network.update_time_offset();
    }

    fn on_peer_left(network: Arc<Network>, peer: &Peer) {
        network.update_time_offset();
    }

    fn on_peers_changed(network: Arc<Network>) {
        let network_clone2 = Arc::clone(&network);
        network.timers.set_delay("network-peers-changed-timeout".to_string(), move || {
            network_clone2.check_peer_count();
        }, Instant::now() + Network::CONNECT_THROTTLE);
    }

    fn on_recycling_request(network: Arc<Network>) {
        network.scorer.write().recycle_connections(1, CloseType::PeerConnectionRecycledInboundExchange, "Peer connection recycled inbound exchange");

        // set ability to exchange for new inbound connections
        network.connections.write().allow_inbound_exchange = match network.scorer.read().lowest_connection_score() {
            Some(lowest_connection_score) => lowest_connection_score < Network::SCORE_INBOUND_EXCHANGE,
            None => false
        };
    }

    fn on_connect_error(network: Arc<Network>) {
        let network_clone2 = Arc::clone(&network);
        network.timers.set_delay("network-connect-error-timeout".to_string(), move || {
            network_clone2.check_peer_count();
        }, Instant::now() + Network::CONNECT_THROTTLE);
    }

    fn check_peer_count(&self) {
        if self.auto_connect.load(Ordering::Relaxed) && self.addresses.read().seeded() && !self.scorer.read().is_good_peer_set() && self.connections.read().connecting_count < Network::CONNECTING_COUNT_MAX {
            // Pick a peer address that we are not connected to yet.
            let peer_addr_opt = self.scorer.read().pick_address();

            // We can't connect if we don't know any more addresses or only want connections to good peers.
            let only_good_peers = self.scorer.read().needs_good_peers() && !self.scorer.read().needs_more_peers();
            let mut no_fitting_peer_available = peer_addr_opt.is_none();
            if !no_fitting_peer_available && only_good_peers {
                if let Some(peer_addr) = &peer_addr_opt {
                    no_fitting_peer_available = !self.scorer.read().is_good_peer(Arc::clone(peer_addr));
                }
            }
            if no_fitting_peer_available {

                if !self.backed_off.load(Ordering::Relaxed) {
                    self.backed_off.store(true, Ordering::Relaxed);
                    let old_backoff = self.backoff.load(Ordering::Relaxed);
                    Duration::min(Network::CONNECT_BACKOFF_MAX, old_backoff * 2);

                    let self_clone = Network::clone(self);
                    self.timers.set_delay("network-bla".to_string(), move || {
                        self_clone.check_peer_count();
                    }, Instant::now() + old_backoff);
                }

                if self.connections.read().count() == 0 {
                    // We are not connected to any peers (anymore) and don't know any more addresses to connect to.

                    // Tell listeners that we are disconnected. This is primarily useful for tests.
                    // TODO

                    // Allow inbound connections. This is important for the first seed node on the network which
                    // will never establish a consensus and needs to accept incoming connections eventually.
                    self.connections.write().allow_inbound_connections = true;
                }
                return;
            }

            // Connect to this address.
            if let Some(peer_address) = peer_addr_opt {
                if !self.connections.write().connect_outbound(Arc::clone(&peer_address)) {
                    self.addresses.write().close(None, peer_address, CloseType::ConnectionFailed);
                }
            }
        }
        self.backoff.store(Network::CONNECT_BACKOFF_INITIAL, Ordering::Relaxed);
    }

    fn update_time_offset(&self) {
        let mut offsets = Vec::new();
        offsets.push(0i64);
        for connection_info in self.connections.read().connection_iter() {
            if connection_info.state() == ConnectionState::Established {
                if let Some(peer) = &connection_info.peer() {
                    offsets.push(peer.time_offset);
                }
            }
        }

        offsets.sort_by(|a, b| { i64::cmp(a, b) } );

        let offsets_len = offsets.len();
        let time_offset = if offsets.len() % 2 == 0 {
            (offsets[(offsets.len() / 2) - 1] + offsets[(offsets.len() / 2) - 1]) / 2
        } else {
            offsets[(offsets.len() - 1) / 2]
        };

        self.network_time.set_offset(time_offset);
    }

    fn housekeeping(connections: Arc<RwLock<ConnectionPool>>, scorer: Arc<RwLock<PeerScorer>>) {
        // TODO

        // recycle
        let peer_count = connections.read().peer_count();
        if peer_count < Network::PEER_COUNT_RECYCLING_ACTIVE {
            // recycle 1% at PEER_COUNT_RECYCLING_ACTIVE, 20% at PEER_COUNT_MAX
            let percentage_to_recycle = (peer_count - Network::PEER_COUNT_RECYCLING_ACTIVE) as f32 * (Network::RECYCLING_PERCENTAGE_MAX - Network::RECYCLING_PERCENTAGE_MIN) / (Network::PEER_COUNT_MAX - Network::PEER_COUNT_RECYCLING_ACTIVE) as f32 + Network::RECYCLING_PERCENTAGE_MIN as f32;
            let connections_to_recycle = f32::ceil(peer_count as f32 * percentage_to_recycle) as u32;
            scorer.write().recycle_connections(connections_to_recycle, CloseType::PeerConnectionRecycled, "Peer connection recycled");
        }

        // set ability to exchange for new inbound connections
        connections.write().allow_inbound_exchange = match scorer.read().lowest_connection_score() {
            Some(lowest_connection_score) => lowest_connection_score < Network::SCORE_INBOUND_EXCHANGE,
            None => false
        };

        // Request fresh addresses.
        Network::refresh_addresses(connections, scorer);
    }

    fn refresh_addresses(connections: Arc<RwLock<ConnectionPool>>, scorer: Arc<RwLock<PeerScorer>>) {
        unimplemented!()
    }

    pub fn peer_count(&self) -> usize {
        return self.connections.read().peer_count();
    }
}

impl Clone for Network {
    fn clone(&self) -> Network {
        Network {
            network_config: Arc::clone(&self.network_config),
            network_time: Arc::clone(&self.network_time),
            auto_connect: Arc::clone(&self.auto_connect),
            backed_off: Arc::clone(&self.backed_off),
            backoff: Arc::clone(&self.backoff),
            addresses: Arc::clone(&self.addresses),
            connections: Arc::clone(&self.connections),
            scorer: Arc::clone(&self.scorer),
            timers: Arc::clone(&self.timers)
        }
    }
}
