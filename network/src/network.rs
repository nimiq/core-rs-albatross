use std::{
    cmp,
    sync::{Arc, Weak},
    time::Duration,
};

use async_trait::async_trait;
use atomic::Atomic;
use atomic::Ordering;
use futures_03::stream::BoxStream;
use parking_lot::RwLock;
use parking_lot::RwLockReadGuard;
use rand::rngs::OsRng;
use rand::Rng;
use thiserror::Error;
use tokio_stream::wrappers::BroadcastStream;

use beserial::{Deserialize, Serialize};
use blockchain::Blockchain;
use genesis::NetworkId;
use macros::upgrade_weak;
use network_interface::{
    network::{MsgAcceptance, PubsubId, Topic},
    prelude::{Network as NetworkInterface, NetworkEvent as NetworkEventI, Peer as PeerInterface},
};
use utils::mutable_once::MutableOnce;
use utils::observer::Notifier;
use utils::time::OffsetTime;
use utils::timers::Timers;

use crate::address::peer_address_book::PeerAddressBook;
use crate::connection::close_type::CloseType;
use crate::connection::connection_info::ConnectionState;
use crate::connection::connection_pool::ConnectionId;
use crate::connection::connection_pool::ConnectionPool;
use crate::connection::connection_pool::ConnectionPoolEvent;
use crate::error::Error;
use crate::network_config::NetworkConfig;
use crate::peer_address::address::PeerAddress;
use crate::peer_channel::PeerChannel;
use crate::peer_scorer::PeerScorer;
use crate::Peer;

#[derive(Debug)]
pub struct AlbatrossId<P> {
    propagation_source: P,
}

impl PubsubId<Arc<PeerAddress>> for AlbatrossId<Arc<PeerAddress>> {
    fn propagation_source(&self) -> Arc<PeerAddress> {
        self.propagation_source.clone()
    }
}

#[derive(Debug, Error)]
pub enum NetworkError {}

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

pub struct Network {
    pub network_config: Arc<NetworkConfig>,
    pub time: Arc<OffsetTime>,
    auto_connect: Atomic<bool>,
    backed_off: Atomic<bool>,
    backoff: Atomic<Duration>,
    pub addresses: Arc<PeerAddressBook>,
    pub connections: Arc<ConnectionPool>,
    scorer: Arc<RwLock<PeerScorer>>,
    timers: Timers<NetworkTimer>,
    pub notifier: RwLock<Notifier<'static, NetworkEvent>>,
    self_weak: MutableOnce<Weak<Network>>,
}

impl Network {
    const PEER_COUNT_RECYCLING_ACTIVE: usize = 1000;
    const RECYCLING_PERCENTAGE_MIN: f64 = 0.01;
    const RECYCLING_PERCENTAGE_MAX: f64 = 0.20;
    const CONNECTING_COUNT_MAX: usize = 2;
    const CONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
    //const CONNECT_BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);
    const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(5 * 60);
    const SCORE_INBOUND_EXCHANGE: f64 = 0.5;
    const CONNECT_THROTTLE: Duration = Duration::from_secs(1);
    const ADDRESS_REQUEST_CUTOFF: usize = 250;
    const ADDRESS_REQUEST_PEERS: usize = 2;

    pub const SIGNALING_ENABLED: bool = true;

    pub fn new(
        blockchain: Arc<Blockchain>,
        network_config: NetworkConfig,
        time: Arc<OffsetTime>,
        network_id: NetworkId,
    ) -> Result<Arc<Self>, Error> {
        if !network_config.is_initialized() {
            return Err(Error::UninitializedPeerKey);
        }

        let net_config = Arc::new(network_config);
        let addresses = Arc::new(PeerAddressBook::new(net_config.clone(), network_id)?);
        let connections = ConnectionPool::new(addresses.clone(), net_config.clone(), blockchain)?;
        let this = Arc::new(Network {
            network_config: net_config.clone(),
            time,
            auto_connect: Atomic::new(false),
            backed_off: Atomic::new(false),
            backoff: Atomic::new(Self::CONNECT_BACKOFF_INITIAL),
            addresses: addresses.clone(),
            connections: connections.clone(),
            scorer: Arc::new(RwLock::new(PeerScorer::new(
                net_config,
                addresses,
                connections,
            ))),
            timers: Timers::new(),
            notifier: RwLock::new(Notifier::new()),
            self_weak: MutableOnce::new(Weak::new()),
        });
        unsafe { this.self_weak.replace(Arc::downgrade(&this)) };

        let weak = Arc::downgrade(&this);
        this.connections
            .notifier
            .write()
            .register(move |event: ConnectionPoolEvent| {
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

    pub fn initialize(&self) -> Result<(), Error> {
        PeerAddressBook::initialize(&self.addresses)?;
        self.connections.initialize()?;
        Ok(())
    }

    pub fn connect(&self) -> Result<(), Error> {
        self.auto_connect.store(true, Ordering::Relaxed);

        let connections = Arc::clone(&self.connections);
        let scorer = Arc::clone(&self.scorer);

        self.timers.set_interval(
            NetworkTimer::Housekeeping,
            move || {
                Self::housekeeping(Arc::clone(&connections), Arc::clone(&scorer));
            },
            Self::HOUSEKEEPING_INTERVAL,
        );

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
        self.notifier
            .read()
            .notify(NetworkEvent::PeerJoined(Arc::new(peer)));
    }

    fn on_peer_left(&self, peer: Peer) {
        self.update_time_offset();
        self.notifier
            .read()
            .notify(NetworkEvent::PeerLeft(Arc::new(peer)));
    }

    fn on_peers_changed(&self, this: Arc<Network>) {
        self.notifier.read().notify(NetworkEvent::PeersChanged);
        self.timers.reset_delay(
            NetworkTimer::PeersChanged,
            move || {
                this.check_peer_count();
            },
            Self::CONNECT_THROTTLE,
        );
    }

    fn on_recycling_request(&self) {
        self.scorer.write().recycle_connections(
            1,
            CloseType::PeerConnectionRecycledInboundExchange,
            "Peer connection recycled inbound exchange",
        );

        // set ability to exchange for new inbound connections
        self.connections.set_allow_inbound_exchange(
            match self.scorer.write().lowest_connection_score() {
                Some(lowest_connection_score) => {
                    lowest_connection_score < Self::SCORE_INBOUND_EXCHANGE
                }
                None => false,
            },
        );
    }

    fn on_connect_error(&self, this: Arc<Network>) {
        // Only set new delay if it doesn't already exist.
        if !self.timers.delay_exists(&NetworkTimer::ConnectError) {
            self.timers.set_delay(
                NetworkTimer::ConnectError,
                move || {
                    this.timers.clear_delay(&NetworkTimer::ConnectError);
                    this.check_peer_count();
                },
                Self::CONNECT_THROTTLE,
            );
        }
    }

    fn check_peer_count(&self) {
        if self.auto_connect.load(Ordering::Relaxed)
            && self.addresses.seeded()
            && !self.scorer.read().is_good_peer_set()
            && self.connections.connecting_count() < Self::CONNECTING_COUNT_MAX
        {
            // Pick a peer address that we are not connected to yet.
            let peer_addr_opt = self.scorer.read().pick_address();

            trace!("Connect to {:?}", peer_addr_opt);

            // We can't connect if we don't know any more addresses or only want connections to good peers.
            let only_good_peers =
                self.scorer.read().needs_good_peers() && !self.scorer.read().needs_more_peers();
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

                    let weak = self.self_weak.clone();
                    self.timers.reset_delay(
                        NetworkTimer::PeerCountCheck,
                        move || {
                            let this = upgrade_weak!(weak);
                            this.check_peer_count();
                        },
                        old_backoff,
                    );
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
                    self.addresses
                        .close(None, peer_address, CloseType::ConnectionFailed);
                }
                trace!("should be connected now");
            }
        }
        self.backoff
            .store(Self::CONNECT_BACKOFF_INITIAL, Ordering::Relaxed);
    }

    fn update_time_offset(&self) {
        let mut offsets = vec![0i64];
        let pool_state = self.connections.state();
        for connection_info in pool_state.connection_iter() {
            if connection_info.state() == ConnectionState::Established {
                if let Some(peer) = &connection_info.peer() {
                    offsets.push(peer.time_offset);
                }
            }
        }

        offsets.sort_by(|a, b| i64::cmp(a, b));

        let offsets_len = offsets.len();
        let time_offset = if offsets_len % 2 == 0 {
            (offsets[(offsets_len / 2) - 1] + offsets[(offsets_len / 2) - 1]) / 2
        } else {
            offsets[(offsets_len - 1) / 2]
        };

        self.time.set_offset(time_offset);
    }

    fn housekeeping(connections: Arc<ConnectionPool>, scorer: Arc<RwLock<PeerScorer>>) {
        scorer.write().score_connections();

        // Recycle.
        let peer_count = connections.peer_count();
        if peer_count > Self::PEER_COUNT_RECYCLING_ACTIVE {
            // recycle 1% at PEER_COUNT_RECYCLING_ACTIVE, 20% at PEER_COUNT_MAX
            let percentage_to_recycle = (peer_count as f64
                - Self::PEER_COUNT_RECYCLING_ACTIVE as f64)
                * (Self::RECYCLING_PERCENTAGE_MAX - Self::RECYCLING_PERCENTAGE_MIN)
                / (ConnectionPool::PEER_COUNT_MAX - Self::PEER_COUNT_RECYCLING_ACTIVE) as f64
                + Self::RECYCLING_PERCENTAGE_MIN as f64;
            let connections_to_recycle =
                f64::ceil(peer_count as f64 * percentage_to_recycle) as u32;
            scorer.write().recycle_connections(
                connections_to_recycle,
                CloseType::PeerConnectionRecycled,
                "Peer connection recycled",
            );
        }

        // Set ability to exchange for new inbound connections.
        connections.set_allow_inbound_exchange(match scorer.write().lowest_connection_score() {
            Some(lowest_connection_score) => lowest_connection_score < Self::SCORE_INBOUND_EXCHANGE,
            None => false,
        });

        // Request fresh addresses.
        Self::refresh_addresses(connections, scorer);
    }

    fn refresh_addresses(connections: Arc<ConnectionPool>, scorer: Arc<RwLock<PeerScorer>>) {
        let connection_scores =
            RwLockReadGuard::map(scorer.read(), |scorer| scorer.connection_scores());
        let mut randrng = OsRng;
        if !connection_scores.is_empty() {
            let state = connections.state();
            let cutoff = cmp::min(
                (state.peer_count_ws + state.peer_count_wss) * 2,
                Self::ADDRESS_REQUEST_CUTOFF,
            );
            let len = cmp::min(connection_scores.len(), cutoff);

            for _ in 0..cmp::min(Self::ADDRESS_REQUEST_PEERS, connection_scores.len()) {
                let index = randrng.gen_range(0, len);
                let (id, _): &(ConnectionId, f64) = connection_scores.get(index).unwrap(); // Cannot fail, since len is at most the real length.
                let peer_connection = state
                    .get_connection(*id)
                    .expect("ConnectionInfo for scored connection is missing");

                trace!(
                    "Requesting addresses from {} (score idx {})",
                    peer_connection
                        .peer_address()
                        .expect("ConnectionInfo for scored connection is missing its PeerAddress"),
                    index
                );

                let agent = peer_connection
                    .network_agent()
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
                    trace!(
                        "Requesting addresses from {} (score idx {})",
                        peer_connection.peer_address().expect(
                            "ConnectionInfo for scored connection is missing its PeerAddress"
                        ),
                        index
                    );

                    let agent = peer_connection
                        .network_agent()
                        .expect("ConnectionInfo for scored connection is missing its NetworkAgent");
                    agent.write().request_addresses(None);
                }
            } else {
                error!("No peers to connect to!")
            }
        }
    }

    pub fn peer_count(&self) -> usize {
        self.connections.peer_count()
    }

    pub fn set_allow_inbound_connections(&self, allow_inbound_connections: bool) {
        self.connections
            .set_allow_inbound_connections(allow_inbound_connections);
    }

    pub fn scorer(&self) -> RwLockReadGuard<PeerScorer> {
        self.scorer.read()
    }
}

#[async_trait]
impl NetworkInterface for Network {
    type PeerType = PeerChannel;
    type AddressType = String;
    type Error = NetworkError;
    type PubsubId = AlbatrossId<Arc<PeerAddress>>;

    fn get_peer_updates(
        &self,
    ) -> (
        Vec<Arc<Self::PeerType>>,
        BroadcastStream<NetworkEventI<Self::PeerType>>,
    ) {
        unimplemented!();
    }

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>> {
        self.connections
            .state()
            .connection_iter()
            .iter()
            .filter_map(|&info| info.peer_channel())
            .collect::<Vec<_>>()
    }

    fn get_peer(
        &self,
        peer_id: <Self::PeerType as PeerInterface>::Id,
    ) -> Option<Arc<Self::PeerType>> {
        self.connections
            .state()
            .get_connection_by_peer_address(&peer_id)
            .map(|info| info.peer_channel())
            .flatten()
    }

    fn subscribe_events(&self) -> BroadcastStream<NetworkEventI<Self::PeerType>> {
        unimplemented!()
    }

    async fn subscribe<'a, T>(
        &self,
    ) -> Result<BoxStream<'a, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn unsubscribe<'a, T>(&self) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn publish<T>(&self, _item: <T as Topic>::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn validate_message(
        &self,
        _id: Self::PubsubId,
        _acceptance: MsgAcceptance,
    ) -> Result<bool, Self::Error> {
        unimplemented!()
    }

    async fn dht_get<K, V>(&self, _k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync,
    {
        unimplemented!()
    }

    async fn dht_put<K, V>(&self, _k: &K, _v: &V) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync,
    {
        unimplemented!()
    }

    async fn dial_peer(
        &self,
        _peer_id: <Self::PeerType as PeerInterface>::Id,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    async fn dial_address(&self, _address: String) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn get_local_peer_id(&self) -> <Self::PeerType as PeerInterface>::Id {
        unimplemented!()
    }
}
