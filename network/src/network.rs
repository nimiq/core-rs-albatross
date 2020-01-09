use std::cmp;
use std::sync::{Arc, Weak};
use std::time::Duration;

use atomic::Atomic;
use atomic::Ordering;
use macros::upgrade_weak;
use parking_lot::RwLock;
use parking_lot::RwLockReadGuard;
use rand::Rng;
use rand::rngs::OsRng;

use blockchain_base::AbstractBlockchain;
use network_primitives::networks::NetworkId;
use network_primitives::time::NetworkTime;
use utils::mutable_once::MutableOnce;
use utils::observer::Notifier;
use utils::timers::Timers;

use crate::address::peer_address_book::PeerAddressBook;
use crate::connection::close_type::CloseType;
use crate::connection::connection_info::ConnectionState;
use crate::connection::connection_pool::ConnectionId;
use crate::connection::connection_pool::ConnectionPool;
use crate::connection::connection_pool::ConnectionPoolEvent;
use crate::error::Error;
use crate::network_config::NetworkConfig;
use crate::Peer;
use crate::peer_scorer::PeerScorer;

#[derive(Debug, Ord, PartialOrd, PartialEq, Eq, Hash)]
enum NetworkTimer {
    Housekeeping,
    PeersChanged,
    ConnectError,
    PeerCountCheck,
}

pub enum NetworkEvent {
    PeerJoined(Arc<Peer>),
    PeerLeft(Arc<Peer>),
    PeersChanged,
}

pub struct Network<B: AbstractBlockchain + 'static> {
    pub network_config: Arc<NetworkConfig>,
    pub network_time: Arc<NetworkTime>,
    auto_connect: Atomic<bool>,
    backed_off: Atomic<bool>,
    backoff: Atomic<Duration>,
    pub addresses: Arc<PeerAddressBook>,
    pub connections: Arc<ConnectionPool<B>>,
    scorer: Arc<RwLock<PeerScorer<B>>>,
    timers: Timers<NetworkTimer>,
    pub notifier: RwLock<Notifier<'static, NetworkEvent>>,
    self_weak: MutableOnce<Weak<Network<B>>>,
}

impl<B: AbstractBlockchain> Network<B> {
    const PEER_COUNT_MAX: usize = 4000;
    const PEER_COUNT_RECYCLING_ACTIVE: usize = 1000;
    const RECYCLING_PERCENTAGE_MIN: f64 = 0.01;
    const RECYCLING_PERCENTAGE_MAX: f64 = 0.20;
    const CONNECTING_COUNT_MAX: usize = 2;
    const CONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
    const CONNECT_BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);
    const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(5 * 60);
    const SCORE_INBOUND_EXCHANGE: f64 = 0.5;
    const CONNECT_THROTTLE: Duration = Duration::from_secs(1);
    const ADDRESS_REQUEST_CUTOFF: usize = 250;
    const ADDRESS_REQUEST_PEERS: usize = 2;

    pub const SIGNALING_ENABLED: bool = true;

    pub fn new(blockchain: Arc<B>, network_config: NetworkConfig, network_time: Arc<NetworkTime>, network_id: NetworkId) -> Result<Arc<Self>, Error> {
        if !network_config.is_initialized() {
            return Err(Error::UninitializedPeerKey);
        }

        let net_config = Arc::new(network_config);
        let addresses = Arc::new(PeerAddressBook::new(net_config.clone(), network_id)?);
        let connections = ConnectionPool::new(addresses.clone(), net_config.clone(), blockchain)?;
        let this = Arc::new(Network {
            network_config: net_config.clone(),
            network_time,
            auto_connect: Atomic::new(false),
            backed_off: Atomic::new(false),
            backoff: Atomic::new(Self::CONNECT_BACKOFF_INITIAL),
            addresses: addresses.clone(),
            connections: connections.clone(),
            scorer: Arc::new(RwLock::new(PeerScorer::new(net_config, addresses, connections.clone()))),
            timers: Timers::new(),
            notifier: RwLock::new(Notifier::new()),
            self_weak: MutableOnce::new(Weak::new()),
        });
        unsafe { this.self_weak.replace(Arc::downgrade(&this)) };

        let weak = Arc::downgrade(&this);
        this.connections.notifier.write().register(move |event: ConnectionPoolEvent| {
            let this = upgrade_weak!(weak);
            match event {
                ConnectionPoolEvent::PeerJoined(peer) => this.on_peer_joined(peer),
                ConnectionPoolEvent::PeerLeft(peer) => this.on_peer_left(peer),
                ConnectionPoolEvent::PeersChanged => this.on_peers_changed(this.clone()),
                ConnectionPoolEvent::RecyclingRequest => this.on_recycling_request(),
                ConnectionPoolEvent::ConnectError(_, _) => this.on_connect_error(this.clone()),
                _ => {}
            }
        });

        if this.network_config.instant_inbound {
            this.connections.set_allow_inbound_connections(true);
        }

        Ok(this)
    }

    pub async fn initialize(&self) -> Result<(), Error> {
        PeerAddressBook::initialize(&self.addresses)?;
        self.connections.initialize().await?;
        Ok(())
    }

    pub fn connect(&self) -> Result<(), Error> {
        self.auto_connect.store(true, Ordering::Relaxed);

        let connections = Arc::clone(&self.connections);
        let scorer = Arc::clone(&self.scorer);

        self.timers.set_interval(NetworkTimer::Housekeeping, move || {
            Self::housekeeping(Arc::clone(&connections), Arc::clone(&scorer));
        }, Self::HOUSEKEEPING_INTERVAL);

        // Start connecting to peers.
        self.check_peer_count();
        Ok(())
    }

    pub fn disconnect(&self) {
        self.auto_connect.store(false, Ordering::Relaxed);

        self.timers.clear_interval(&NetworkTimer::Housekeeping);

        self.connections.disconnect();
        self.connections.set_allow_inbound_exchange(false);
    }

    fn on_peer_joined(&self, peer: Peer) {
        self.update_time_offset();
        self.notifier.read().notify(NetworkEvent::PeerJoined(Arc::new(peer)));
    }

    fn on_peer_left(&self, peer: Peer) {
        self.update_time_offset();
        self.notifier.read().notify(NetworkEvent::PeerLeft(Arc::new(peer)));
    }

    fn on_peers_changed(&self, this: Arc<Network<B>>) {
        self.notifier.read().notify(NetworkEvent::PeersChanged);
        self.timers.reset_delay(NetworkTimer::PeersChanged, move || {
            this.check_peer_count();
        }, Self::CONNECT_THROTTLE);
    }

    fn on_recycling_request(&self) {
        self.scorer.write().recycle_connections(1, CloseType::PeerConnectionRecycledInboundExchange, "Peer connection recycled inbound exchange");

        // set ability to exchange for new inbound connections
        self.connections.set_allow_inbound_exchange(match self.scorer.write().lowest_connection_score() {
            Some(lowest_connection_score) => lowest_connection_score < Self::SCORE_INBOUND_EXCHANGE,
            None => false
        });
    }

    fn on_connect_error(&self, this: Arc<Network<B>>) {
        // Only set new delay if it doesn't already exist.
        if !self.timers.delay_exists(&NetworkTimer::ConnectError) {
            self.timers.set_delay(NetworkTimer::ConnectError, move || {
                this.timers.clear_delay(&NetworkTimer::ConnectError);
                this.check_peer_count();
            }, Self::CONNECT_THROTTLE);
        }
    }

    fn check_peer_count(&self) {
        if self.auto_connect.load(Ordering::Relaxed)
            && self.addresses.seeded()
            && !self.scorer.read().is_good_peer_set()
            && self.connections.connecting_count() < Self::CONNECTING_COUNT_MAX {

            // Pick a peer address that we are not connected to yet.
            let peer_addr_opt = self.scorer.read().pick_address();

            trace!("Connect to {:?}", peer_addr_opt);

            // We can't connect if we don't know any more addresses or only want connections to good peers.
            let only_good_peers = self.scorer.read().needs_good_peers() && !self.scorer.read().needs_more_peers();
            let mut no_matching_peer_available = peer_addr_opt.is_none();
            if !no_matching_peer_available && only_good_peers {
                if let Some(peer_addr) = &peer_addr_opt {
                    no_matching_peer_available = !self.scorer.read().is_good_peer(peer_addr);
                }
            }

            if no_matching_peer_available {
                if !self.backed_off.load(Ordering::Relaxed) {
                    self.backed_off.store(true, Ordering::Relaxed);
                    let old_backoff = self.backoff.load(Ordering::Relaxed);
                    Duration::min(Self::CONNECT_BACKOFF_MAX, old_backoff * 2);

                    let weak = self.self_weak.clone();
                    self.timers.reset_delay(NetworkTimer::PeerCountCheck, move || {
                        let this = upgrade_weak!(weak);
                        this.check_peer_count();
                    }, old_backoff);
                }

                if self.connections.count() == 0 {
                    // We are not connected to any peers (anymore) and don't know any more addresses to connect to.

                    // Tell listeners that we are disconnected. This is primarily useful for tests.
                    // TODO

                    // Allow inbound connections. This is important for the first seed node on the network which
                    // will never establish a consensus and needs to accept incoming connections eventually.
                    self.connections.set_allow_inbound_connections(true);
                }
                return;
            }

            // Connect to this address.
            if let Some(peer_address) = peer_addr_opt {
                trace!("Connect outbound: {}", peer_address);
                if !self.connections.connect_outbound(Arc::clone(&peer_address)) {
                    trace!("Connect outbound failed");
                    self.addresses.close(None, peer_address, CloseType::ConnectionFailed);
                }
                trace!("should be connected now");
            }
        }
        self.backoff.store(Self::CONNECT_BACKOFF_INITIAL, Ordering::Relaxed);
    }

    fn update_time_offset(&self) {
        let mut offsets = Vec::new();
        offsets.push(0i64);
        let pool_state = self.connections.state();
        for connection_info in pool_state.connection_iter() {
            if connection_info.state() == ConnectionState::Established {
                if let Some(peer) = &connection_info.peer() {
                    offsets.push(peer.time_offset);
                }
            }
        }

        offsets.sort_by(|a, b| { i64::cmp(a, b) } );

        let offsets_len = offsets.len();
        let time_offset = if offsets_len % 2 == 0 {
            (offsets[(offsets_len / 2) - 1] + offsets[(offsets_len / 2) - 1]) / 2
        } else {
            offsets[(offsets_len - 1) / 2]
        };

        self.network_time.set_offset(time_offset);
    }

    fn housekeeping(connections: Arc<ConnectionPool<B>>, scorer: Arc<RwLock<PeerScorer<B>>>) {
        scorer.write().score_connections();

        // Recycle.
        let peer_count = connections.peer_count();
        if peer_count > Self::PEER_COUNT_RECYCLING_ACTIVE {
            // recycle 1% at PEER_COUNT_RECYCLING_ACTIVE, 20% at PEER_COUNT_MAX
            let percentage_to_recycle = (peer_count as f64 - Self::PEER_COUNT_RECYCLING_ACTIVE as f64) * (Self::RECYCLING_PERCENTAGE_MAX - Self::RECYCLING_PERCENTAGE_MIN) / (Self::PEER_COUNT_MAX - Self::PEER_COUNT_RECYCLING_ACTIVE) as f64 + Self::RECYCLING_PERCENTAGE_MIN as f64;
            let connections_to_recycle = f64::ceil(peer_count as f64 * percentage_to_recycle) as u32;
            scorer.write().recycle_connections(connections_to_recycle, CloseType::PeerConnectionRecycled, "Peer connection recycled");
        }

        // Set ability to exchange for new inbound connections.
        connections.set_allow_inbound_exchange(match scorer.write().lowest_connection_score() {
            Some(lowest_connection_score) => lowest_connection_score < Self::SCORE_INBOUND_EXCHANGE,
            None => false
        });

        // Request fresh addresses.
        Self::refresh_addresses(connections, scorer);
    }

    fn refresh_addresses(connections: Arc<ConnectionPool<B>>, scorer: Arc<RwLock<PeerScorer<B>>>) {
        let connection_scores = RwLockReadGuard::map(scorer.read(), |scorer| scorer.connection_scores());
        let mut randrng = OsRng;
        if !connection_scores.is_empty() {
            let state = connections.state();
            let cutoff = cmp::min(
                (state.peer_count_ws + state.peer_count_wss) * 2,
                Self::ADDRESS_REQUEST_CUTOFF
            );
            let len = cmp::min(
                connection_scores.len(),
                cutoff
            );

            for _ in 0..cmp::min(Self::ADDRESS_REQUEST_PEERS, connection_scores.len()) {
                let index = randrng.gen_range(0, len);
                let (id, _): &(ConnectionId, f64) = connection_scores.get(index).unwrap(); // Cannot fail, since len is at most the real length.
                let peer_connection = state.get_connection(*id)
                    .expect("ConnectionInfo for scored connection is missing");

                trace!("Requesting addresses from {} (score idx {})", peer_connection.peer_address()
                    .expect("ConnectionInfo for scored connection is missing its PeerAddress"), index);

                let agent = peer_connection.network_agent()
                    .expect("ConnectionInfo for scored connection is missing its NetworkAgent");
                agent.write().request_addresses(None);
            }
        } else {
            // Drop lock on connection_scores since it is empty.
            drop(connection_scores);
            if connections.count() > 0 {
                let index = randrng.gen_range(0, cmp::min(connections.count(), 10));

                let state = connections.state();
                let mut peer_connection = None;
                for (i, conn) in state.connection_iter().into_iter().enumerate() {
                    if conn.state() == ConnectionState::Established {
                        peer_connection = Some(conn);
                    }
                    if i >= index && peer_connection.is_some() {
                        break;
                    }
                }

                if let Some(peer_connection) = peer_connection {
                    trace!("Requesting addresses from {} (score idx {})", peer_connection.peer_address()
                        .expect("ConnectionInfo for scored connection is missing its PeerAddress"), index);

                    let agent = peer_connection.network_agent()
                        .expect("ConnectionInfo for scored connection is missing its NetworkAgent");
                    agent.write().request_addresses(None);
                }
            }
            else {
                error!("No peers to connect to!")
            }
        }
    }

    pub fn peer_count(&self) -> usize {
        self.connections.peer_count()
    }

    pub fn set_allow_inbound_connections(&self, allow_inbound_connections: bool) {
        self.connections.set_allow_inbound_connections(allow_inbound_connections);
    }

    pub fn scorer(&self) -> RwLockReadGuard<PeerScorer<B>> {
        self.scorer.read()
    }
}
