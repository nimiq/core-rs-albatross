use std::borrow::Borrow;
use std::cmp;
use std::collections::hash_map::{Keys, Values};
use std::collections::hash_set::Iter;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::iter::Iterator;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use parking_lot::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use rand::{rngs::OsRng, Rng};

use genesis::{NetworkId, NetworkInfo};
use macros::upgrade_weak;
use peer_address::address::{NetAddress, PeerAddress, PeerId};
use peer_address::protocol::{Protocol, ProtocolFlags};
use peer_address::services::ServiceFlags;
use utils::iterators::Alternate;
use utils::observer::Notifier;
use utils::time::systemtime_to_timestamp;
use utils::timers::Timers;

use crate::connection::close_type::CloseType;
use crate::error::Error;
use crate::network_config::{NetworkConfig, Seed};
use crate::peer_channel::PeerChannel;

use super::peer_address_seeder::{PeerAddressSeeder, PeerAddressSeederEvent};
use super::peer_address_state::{PeerAddressInfo, PeerAddressState};

pub struct PeerAddressBookState {
    info_by_address: HashMap<Arc<PeerAddress>, PeerAddressInfo>,
    ws_addresses: HashSet<Arc<PeerAddress>>,
    wss_addresses: HashSet<Arc<PeerAddress>>,
    rtc_addresses: HashSet<Arc<PeerAddress>>,
    address_by_peer_id: HashMap<PeerId, Arc<PeerAddress>>,
    addresses_by_net_address: HashMap<NetAddress, HashSet<Arc<PeerAddress>>>,
}

impl PeerAddressBookState {
    pub fn address_info_iter(&self) -> Values<Arc<PeerAddress>, PeerAddressInfo> {
        self.info_by_address.values()
    }

    pub fn address_iter(&self) -> Keys<Arc<PeerAddress>, PeerAddressInfo> {
        self.info_by_address.keys()
    }

    pub fn ws_address_iter(&self) -> Iter<Arc<PeerAddress>> {
        self.ws_addresses.iter()
    }

    pub fn wss_address_iter(&self) -> Iter<Arc<PeerAddress>> {
        self.wss_addresses.iter()
    }

    pub fn rtc_address_iter(&self) -> Iter<Arc<PeerAddress>> {
        self.rtc_addresses.iter()
    }

    pub fn address_iter_for_protocol_mask(&self, protocol_mask: ProtocolFlags) -> QueryIterator {
        if protocol_mask == ProtocolFlags::WSS {
            QueryIterator::Iter(self.wss_address_iter())
        } else if protocol_mask == ProtocolFlags::WS {
            QueryIterator::Iter(self.ws_address_iter())
        } else if protocol_mask == ProtocolFlags::WS | ProtocolFlags::WSS {
            QueryIterator::Alternate(Alternate::new(
                self.ws_address_iter(),
                self.wss_address_iter(),
            ))
        } else if protocol_mask == ProtocolFlags::RTC {
            QueryIterator::Iter(self.rtc_address_iter())
        } else if protocol_mask == ProtocolFlags::RTC | ProtocolFlags::WS {
            QueryIterator::Alternate(Alternate::new(
                self.rtc_address_iter(),
                self.ws_address_iter(),
            ))
        } else if protocol_mask == ProtocolFlags::RTC | ProtocolFlags::WSS {
            QueryIterator::Alternate(Alternate::new(
                self.rtc_address_iter(),
                self.wss_address_iter(),
            ))
        } else {
            QueryIterator::Keys(self.address_iter())
        }
    }

    pub fn known_addresses_nr_for_protocol_mask(&self, protocol_mask: ProtocolFlags) -> usize {
        if protocol_mask == ProtocolFlags::WSS {
            self.known_wss_addresses_count()
        } else if protocol_mask == ProtocolFlags::WS {
            self.known_ws_addresses_count()
        } else if protocol_mask == ProtocolFlags::WS | ProtocolFlags::WSS {
            self.known_ws_addresses_count() + self.known_wss_addresses_count()
        } else if protocol_mask == ProtocolFlags::RTC {
            self.known_rtc_addresses_count()
        } else if protocol_mask == ProtocolFlags::RTC | ProtocolFlags::WS {
            self.known_rtc_addresses_count() + self.known_ws_addresses_count()
        } else if protocol_mask == ProtocolFlags::RTC | ProtocolFlags::WSS {
            self.known_rtc_addresses_count() + self.known_wss_addresses_count()
        } else {
            self.known_addresses_count()
        }
    }

    pub fn get_info<P>(&self, peer_address: &P) -> Option<&PeerAddressInfo>
    where
        Arc<PeerAddress>: Borrow<P>,
        P: Hash + Eq,
    {
        self.info_by_address.get(peer_address)
    }

    pub fn get_info_mut<P>(&mut self, peer_address: &P) -> Option<&mut PeerAddressInfo>
    where
        Arc<PeerAddress>: Borrow<P>,
        P: Hash + Eq,
    {
        self.info_by_address.get_mut(peer_address)
    }

    pub fn get_by_peer_id(&self, peer_id: &PeerId) -> Option<Arc<PeerAddress>> {
        if let Some(peer_address) = self.address_by_peer_id.get(peer_id) {
            return Some(Arc::clone(peer_address));
        }
        None
    }

    pub fn get_channel_by_peer_id(&self, peer_id: &PeerId) -> Option<&PeerChannel> {
        if let Some(peer_address) = self.address_by_peer_id.get(peer_id) {
            if let Some(info) = self.info_by_address.get(peer_address) {
                if let Some(ref best_route_opt) = info.signal_router.best_route {
                    return Some(&best_route_opt.signal_channel);
                }
            }
        }
        None
    }

    fn add_to_store(&mut self, info: PeerAddressInfo) {
        // Index by peer id.
        self.address_by_peer_id.insert(
            info.peer_address.peer_id.clone(),
            Arc::clone(&info.peer_address),
        );

        // Index by protocol.
        match info.peer_address.protocol() {
            Protocol::Ws => {
                self.ws_addresses.insert(Arc::clone(&info.peer_address));
            }
            Protocol::Wss => {
                self.wss_addresses.insert(Arc::clone(&info.peer_address));
            }
            Protocol::Rtc => {
                self.rtc_addresses.insert(Arc::clone(&info.peer_address));
            }
            Protocol::Dumb => {} // Dumb addresses are ignored.
        };

        // Index peer address info by peer address.
        self.info_by_address
            .insert(Arc::clone(&info.peer_address), info);
    }

    fn remove_from_store(&mut self, peer_address: Arc<PeerAddress>) {
        // Never delete seed addresses, ban them instead for a couple of minutes.
        if let Some(info) = self.get_info(&peer_address) {
            if info.peer_address.is_seed() {
                self.ban(peer_address.clone(), DEFAULT_BAN_TIME);
                return;
            }
        }

        // Delete from peer id index.
        self.address_by_peer_id.remove(&peer_address.peer_id);

        // Delete from net address index.
        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            for net_address in &info.added_by {
                if let Some(addresses) = self.addresses_by_net_address.get_mut(net_address) {
                    addresses.remove(&peer_address);
                }
            }
        }

        // Remove from protocol index.
        match peer_address.protocol() {
            Protocol::Ws => {
                self.ws_addresses.remove(&peer_address);
            }
            Protocol::Wss => {
                self.wss_addresses.remove(&peer_address);
            }
            Protocol::Rtc => {
                self.rtc_addresses.remove(&peer_address);
            }
            _ => {}
        }

        if let Some(info) = self.get_info(&peer_address) {
            // Don't delete bans.
            if info.state == PeerAddressState::Banned {
                return;
            }

            // Delete the address.
            self.info_by_address.remove(&peer_address);
        }
    }

    /// Delete all RTC-only routes that are signalable over the given peer.
    fn remove_by_signal_channel(&mut self, channel: Arc<PeerChannel>) {
        // XXX inefficient linear scan
        let addresses = self
            .address_iter()
            .filter(|&peer_address| peer_address.protocol() == Protocol::Rtc)
            .cloned()
            .collect::<Vec<Arc<PeerAddress>>>();

        for peer_address in addresses {
            let info = self
                .info_by_address
                .get_mut(&peer_address)
                .expect("Since this was returned by `address_iter()` it should never be None");
            info.signal_router.delete_route(channel.clone());
            if !info.signal_router.has_route() {
                self.remove_from_store(peer_address);
            }
        }
    }

    fn track_by_net_address(
        &mut self,
        peer_address: Arc<PeerAddress>,
        net_address: Option<Arc<NetAddress>>,
    ) {
        if let Some(net_address) = net_address {
            self.addresses_by_net_address
                .entry(*net_address.as_ref())
                .or_insert_with(HashSet::new)
                .insert(peer_address.clone());

            if let Some(info) = self.info_by_address.get_mut(&peer_address) {
                info.added_by.insert(net_address);
            }
        }
    }

    fn ban(&mut self, peer_address: Arc<PeerAddress>, duration: Duration) {
        if self.info_by_address.get(&peer_address).is_none() {
            self.add_to_store(PeerAddressInfo::new(Arc::clone(&peer_address)));
        }

        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            info.state = PeerAddressState::Banned;
            info.banned_until = Some(Instant::now() + duration);

            // Drop all routes to this peer.
            info.signal_router.delete_all_routes();
        }
    }

    pub fn is_banned(&self, peer_address: &Arc<PeerAddress>) -> bool {
        if let Some(info) = self.get_info(peer_address) {
            if info.state == PeerAddressState::Banned {
                // XXX Never consider seed peers to be banned. This allows us to use
                // the banning mechanism to prevent seed peers from being picked when
                // they are down, but still allows recovering seed peers' inbound
                // connections to succeed.
                return !info.peer_address.is_seed();
            }
        }
        false
    }

    pub fn known_addresses_count(&self) -> usize {
        self.info_by_address.len()
    }
    pub fn known_ws_addresses_count(&self) -> usize {
        self.ws_addresses.len()
    }
    pub fn known_wss_addresses_count(&self) -> usize {
        self.wss_addresses.len()
    }
    pub fn known_rtc_addresses_count(&self) -> usize {
        self.rtc_addresses.len()
    }
}

pub struct PeerAddressBook {
    state: RwLock<PeerAddressBookState>,
    seeded: AtomicBool,
    network_config: Arc<NetworkConfig>,
    network_id: NetworkId,
    timers: Timers<PeerAddressBookTimer>,
    change_lock: Mutex<()>,
    pub notifier: Notifier<'static, PeerAddressBookEvent>,
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
enum PeerAddressBookTimer {
    ExternalSeeding,
    Housekeeping,
}

pub enum PeerAddressBookEvent {
    Added(Vec<PeerAddress>),
    Seeded,
}

impl PeerAddressBook {
    pub fn new(network_config: Arc<NetworkConfig>, network_id: NetworkId) -> Result<Self, Error> {
        let this = Self {
            state: RwLock::new(PeerAddressBookState {
                info_by_address: HashMap::new(),
                ws_addresses: HashSet::new(),
                wss_addresses: HashSet::new(),
                rtc_addresses: HashSet::new(),
                address_by_peer_id: HashMap::new(),
                addresses_by_net_address: HashMap::new(),
            }),
            seeded: AtomicBool::new(false),
            network_id,
            network_config,
            timers: Timers::new(),
            change_lock: Mutex::new(()),
            notifier: Notifier::new(),
        };

        // Init hardcoded seed peers.
        this.add(
            None,
            NetworkInfo::from_network_id(network_id)
                .seed_peers()
                .clone(),
        );

        // Init seed peers from config file.
        let additional_seeds: Vec<PeerAddress> = this
            .network_config
            .additional_seeds()
            .iter()
            .filter_map(|seed| match seed {
                Seed::Peer(peer_uri) => Some(peer_uri.as_seed_peer_address().expect(
                    "This should be checked before adding the seed peer to network_config",
                )),
                Seed::List(_) => None,
            })
            .collect();
        this.add(None, additional_seeds);

        Ok(this)
    }

    /// Initialises async stuff.
    pub fn initialize(this: &Arc<Self>) -> Result<(), Error> {
        // Setup housekeeping interval.
        let weak = Arc::downgrade(this);
        this.timers.set_interval(
            PeerAddressBookTimer::Housekeeping,
            move || {
                let this = upgrade_weak!(weak);
                this.housekeeping();
            },
            HOUSEKEEPING_INTERVAL,
        );

        // Collect more seed peers from seed lists.
        let weak = Arc::downgrade(this);
        this.timers.set_delay(
            PeerAddressBookTimer::ExternalSeeding,
            move || {
                let this = upgrade_weak!(weak);
                this.seeded.store(true, Ordering::Release);
                this.notifier.notify(PeerAddressBookEvent::Seeded);
            },
            SEEDING_TIMEOUT,
        );

        let seeder = PeerAddressSeeder::new();
        let weak = Arc::downgrade(this);
        seeder
            .notifier
            .lock()
            .register(move |e: &PeerAddressSeederEvent| {
                let this = upgrade_weak!(weak);
                match e {
                    PeerAddressSeederEvent::Seeds(seeds) => {
                        trace!("Adding new seeds from remote seed list");
                        this.add(None, seeds.to_vec());
                    }
                    PeerAddressSeederEvent::End => {
                        this.seeded.store(true, Ordering::Release);
                        this.notifier.notify(PeerAddressBookEvent::Seeded);
                    }
                }
            });
        seeder.collect(this.network_id, this.network_config.clone());

        Ok(())
    }

    pub fn query(
        &self,
        protocol_mask: ProtocolFlags,
        service_mask: ServiceFlags,
        max_addresses: u16,
    ) -> Vec<Arc<PeerAddress>> {
        let max_addresses = max_addresses as usize; // Internally, we need a usize.

        let state = self.state.read();

        let iterator = state.address_iter_for_protocol_mask(protocol_mask);
        let num_addresses = state.known_addresses_nr_for_protocol_mask(protocol_mask);

        // Pick a random start index if we have a lot of addresses.
        let start_index = if num_addresses > max_addresses {
            OsRng.gen_range(0, num_addresses)
        } else {
            0
        };

        // XXX inefficient linear scan
        iterator
            .cycle()
            .skip(start_index)
            .take(cmp::min(max_addresses, num_addresses))
            .filter(|&peer_address| {
                if let Some(info) = state.info_by_address.get(peer_address) {
                    // Never return banned or failed addresses.
                    if info.state == PeerAddressState::Banned
                        || info.state == PeerAddressState::Failed
                    {
                        return false;
                    }

                    // Never return seed peers.
                    if peer_address.is_seed() {
                        return false;
                    }

                    // Only return addresses matching the protocol mask.
                    if !protocol_mask.contains(ProtocolFlags::from(peer_address.protocol())) {
                        return false;
                    }

                    // Only return addresses matching the service mask.
                    // TODO Is that the behaviour we'd like to see?
                    if !service_mask.intersects(peer_address.services) {
                        return false;
                    }

                    // Exclude RTC addresses that are already at MAX_DISTANCE.
                    if peer_address.protocol() == Protocol::Rtc
                        && peer_address.distance >= MAX_DISTANCE
                    {
                        return false;
                    }

                    // Never return addresses that are too old.
                    if peer_address.exceeds_age() {
                        return false;
                    }

                    return true;
                }
                false
            })
            .cloned()
            .collect()
    }

    pub fn add(&self, channel: Option<Arc<PeerChannel>>, peer_addresses: Vec<PeerAddress>) {
        let guard = self.change_lock.lock();

        let mut new_addresses: Vec<PeerAddress> = Vec::new();
        for peer_address in peer_addresses {
            if self.add_single(channel.clone(), peer_address.clone()) {
                trace!("Added new peer: {}", peer_address.as_uri());
                new_addresses.push(peer_address);
            }
        }

        // Drop the guard before notifying.
        drop(guard);

        self.notifier
            .notify(PeerAddressBookEvent::Added(new_addresses));
    }

    fn add_single(&self, channel: Option<Arc<PeerChannel>>, peer_address: PeerAddress) -> bool {
        // Ignore our own address.
        if self.network_config.peer_address() == peer_address {
            return false;
        }

        // Ignore address if it is too old.
        // NOTE: `exceeds_age()` is always false for seed addresses
        if peer_address.exceeds_age() {
            return false;
        }

        // Ignore address if its timestamp is too far in the future.
        if peer_address.timestamp > systemtime_to_timestamp(SystemTime::now() + MAX_TIMESTAMP_DRIFT)
        {
            return false;
        }

        let mut addr_arc = Arc::new(peer_address);
        let mut state = self.state.write();
        // Increment distance values of RTC addresses.
        if addr_arc.protocol() == Protocol::Rtc {
            let peer_address = Arc::get_mut(&mut addr_arc)
                .expect("We should always have access to peer address here");

            peer_address.distance += 1;

            // Ignore address if it exceeds max distance.
            if peer_address.distance > MAX_DISTANCE {
                // Drop any route to this peer over the current channel. This may prevent loops.
                if let Some(info) = state.info_by_address.get_mut(&addr_arc) {
                    info.signal_router
                        .delete_route(channel.expect("RTC peers should always have a channel"));
                }
                return false;
            }
        }

        // Get the (reliable) netAddress of the peer that sent us this address.
        let net_address = channel.clone().and_then(|channel| {
            if let Some(net_address) = channel.address_info.net_address() {
                if net_address.is_reliable() {
                    return Some(net_address);
                }
            }
            None
        });

        // Check if we already know this address.
        let mut changed = false;
        if let Some(info) = state.info_by_address.get_mut(&addr_arc) {
            // Ignore address if it is banned.
            if info.state == PeerAddressState::Banned {
                return false;
            }

            // Never update seed peers.
            if info.peer_address.is_seed() {
                return false;
            }

            // Never erase NetAddresses and never overwrite reliable addresses.
            if addr_arc.net_address.is_pseudo() && info.peer_address.net_address.is_reliable() {
                let peer_address = Arc::get_mut(&mut addr_arc);
                if let Some(peer_address) = peer_address {
                    peer_address.net_address = info.peer_address.net_address;
                } else {
                    unreachable!();
                }
            }

            // Update address if it has a more recent timestamp.
            if info.peer_address.timestamp < addr_arc.timestamp {
                info.peer_address = addr_arc.clone();
                changed = true;
            }
        } else {
            // New address, check max book size.
            if state.info_by_address.len() >= MAX_SIZE {
                return false;
            }

            // Check max size per protocol.
            match addr_arc.ty.protocol() {
                Protocol::Ws => {
                    if state.ws_addresses.len() >= MAX_SIZE_WS {
                        return false;
                    }
                }
                Protocol::Wss => {
                    if state.wss_addresses.len() >= MAX_SIZE_WSS {
                        return false;
                    }
                }
                Protocol::Rtc => {
                    if state.rtc_addresses.len() >= MAX_SIZE_RTC {
                        return false;
                    }
                }
                Protocol::Dumb => {} // Dumb addresses are only part of global limit.
            }

            // If we know the IP address of the sender, check that we don't exceed the maximum number of addresses per IP.
            if let Some(ref net_address) = net_address {
                let addresses = state.addresses_by_net_address.get(net_address);
                if let Some(addresses) = addresses {
                    if addresses.len() >= MAX_SIZE_PER_IP {
                        return false;
                    }
                }
            }

            // Add new peerAddressState.
            let new_info = PeerAddressInfo::new(addr_arc.clone());
            state.add_to_store(new_info);
            changed = true;
        }

        // Add route.
        if addr_arc.protocol() == Protocol::Rtc {
            if let Some(info) = state.info_by_address.get_mut(&addr_arc) {
                changed = info.signal_router.add_route(
                    channel.expect("RTC peers should always have a channel"),
                    addr_arc.distance,
                    addr_arc.timestamp,
                ) || changed;
            }
        }

        // Track which IP address send us this address.
        state.track_by_net_address(addr_arc, net_address);

        changed
    }

    /// Called when a connection to this peerAddress has been established.
    /// The connection might have been initiated by the other peer, so address may not be known previously.
    /// If it is already known, it has been updated by a previous version message.
    pub fn established(&self, channel: Arc<PeerChannel>, peer_address: Arc<PeerAddress>) {
        let _guard = self.change_lock.lock();

        // Make sure that there is always a PeerAddressInfo for this peer_address
        let mut state = self.state.write();
        if state
            .info_by_address
            .get(&Arc::clone(&peer_address))
            .is_none()
        {
            state.add_to_store(PeerAddressInfo::new(Arc::clone(&peer_address)));
        }

        // Get the (reliable) netAddress of the peer that sent us this address.
        let net_address: Option<Arc<NetAddress>> =
            channel.address_info.net_address().and_then(|net_address| {
                if net_address.is_reliable() {
                    Some(net_address)
                } else {
                    None
                }
            });
        state.track_by_net_address(peer_address.clone(), net_address);

        let info = state
            .info_by_address
            .get_mut(&peer_address)
            .expect("Code above guarantees that this will never be None");

        info.state = PeerAddressState::Established;
        info.last_connected = Some(SystemTime::now());
        info.failed_attempts = 0;
        info.banned_until = None;
        info.ban_backoff = INITIAL_FAILED_BACKOFF;

        if !info.peer_address.is_seed() {
            info.peer_address = Arc::clone(&peer_address);
        }

        // Add route.
        if peer_address.protocol() == Protocol::Rtc {
            info.signal_router
                .add_route(channel, peer_address.distance, peer_address.timestamp);
        }
    }

    /// Called when a connection to this peerAddress is closed.
    pub fn close(
        &self,
        channel: Option<Arc<PeerChannel>>,
        peer_address: Arc<PeerAddress>,
        ty: CloseType,
    ) {
        let _guard = self.change_lock.lock();

        let mut state = self.state.write();
        if let Some(info) = state.info_by_address.get_mut(&peer_address) {
            // Register the type of disconnection.
            info.close(ty);

            if ty.is_failing_type() {
                info.failed_attempts += 1;

                if info.failed_attempts >= info.max_failed_attempts() {
                    // Remove address only if we have tried the maximum number of backoffs.
                    if info.ban_backoff >= MAX_FAILED_BACKOFF {
                        state.remove_from_store(Arc::clone(&peer_address));
                    } else {
                        info.banned_until = Some(Instant::now() + info.ban_backoff);
                        info.ban_backoff = cmp::min(MAX_FAILED_BACKOFF, info.ban_backoff * 2);
                    }
                }
            }

            // Delete all addresses that were signalable over the disconnected peer.
            if let Some(channel) = channel {
                state.remove_by_signal_channel(channel);
            }

            if ty.is_banning_type() {
                state.ban(Arc::clone(&peer_address), DEFAULT_BAN_TIME);
            }

            // Immediately delete dumb addresses, since we cannot connect to those anyway.
            if peer_address.protocol() == Protocol::Dumb {
                state.remove_from_store(peer_address);
            }
        }
    }

    /// Called when a message has been returned as unroutable.
    pub fn unroutable(&self, channel: Arc<PeerChannel>, peer_address: Arc<PeerAddress>) {
        let _guard = self.change_lock.lock();

        let mut state = self.state.write();
        let mut peer_address_to_remove = None;
        if let Some(info) = state.info_by_address.get_mut(&peer_address) {
            if let Some(best_route) = &info.signal_router.best_route {
                if channel != best_route.signal_channel {
                    return;
                }
            } else {
                return;
            }

            info.signal_router.delete_best_route();
            if !info.signal_router.has_route() {
                peer_address_to_remove = Some(Arc::clone(&info.peer_address));
            }
        }

        if let Some(peer_address_to_remove) = peer_address_to_remove {
            state.remove_from_store(peer_address_to_remove);
        }
    }

    fn housekeeping(&self) {
        let guard = self.change_lock.lock();

        let mut state = self.state.write();
        let now = Instant::now();
        let mut unbanned_addresses: Vec<PeerAddress> = Vec::new();

        let mut to_remove_from_store = Vec::new();

        for (peer_address, info) in state.info_by_address.iter_mut() {
            match info.state {
                PeerAddressState::New | PeerAddressState::Tried | PeerAddressState::Failed => {
                    // Delete all new peer addresses that are older than MAX_AGE.
                    if peer_address.exceeds_age() {
                        to_remove_from_store.push(peer_address.clone());
                        continue;
                    }

                    // Reset failed attempts after banned_until has expired.
                    if info.state == PeerAddressState::Failed
                        && info.failed_attempts >= info.max_failed_attempts()
                    {
                        if let Some(banned_until) = info.banned_until {
                            if banned_until <= now {
                                info.banned_until = None;
                                info.failed_attempts = 0;
                                unbanned_addresses.push(peer_address.as_ref().clone());
                            }
                        }
                    }
                }
                PeerAddressState::Banned => {
                    if let Some(banned_until) = info.banned_until {
                        if banned_until <= now {
                            // Don't remove seed addresses, unban them.
                            if peer_address.is_seed() {
                                // Restore banned seed addresses to the NEW state.
                                info.state = PeerAddressState::New;
                                info.failed_attempts = 0;
                                info.banned_until = None;
                                unbanned_addresses.push(peer_address.as_ref().clone());
                            } else {
                                // Delete expired bans.
                                to_remove_from_store.push(peer_address.clone());
                            }
                        }
                    }
                }
                PeerAddressState::Established => {
                    // Also update timestamp for RTC connections
                    if let Some(ref mut best_route) = info.signal_router.best_route {
                        best_route.timestamp = systemtime_to_timestamp(SystemTime::now());
                    }
                }
                //_ => {
                // TODO What about peers who are stuck connecting? Can this happen?
                // Do nothing for CONNECTING peers.
                //},
            }
        }

        // Remove addresses.
        for peer_address in to_remove_from_store.drain(..) {
            state.remove_from_store(peer_address);
        }

        // Drop the guard before notifying.
        drop(state);
        drop(guard);

        if !unbanned_addresses.is_empty() {
            self.notifier
                .notify(PeerAddressBookEvent::Added(unbanned_addresses));
        }
    }

    pub fn seeded(&self) -> bool {
        self.seeded.load(Ordering::Acquire)
    }

    pub fn known_addresses_count(&self) -> usize {
        self.state.read().info_by_address.len()
    }
    pub fn known_ws_addresses_count(&self) -> usize {
        self.state.read().ws_addresses.len()
    }
    pub fn known_wss_addresses_count(&self) -> usize {
        self.state.read().wss_addresses.len()
    }
    pub fn known_rtc_addresses_count(&self) -> usize {
        self.state.read().rtc_addresses.len()
    }

    pub fn is_banned(&self, peer_address: &Arc<PeerAddress>) -> bool {
        self.state.read().is_banned(peer_address)
    }

    pub fn state(&self) -> RwLockReadGuard<PeerAddressBookState> {
        self.state.read()
    }

    pub fn state_mut(&self) -> RwLockWriteGuard<PeerAddressBookState> {
        self.state.write()
    }
}

#[derive(Clone)]
pub enum QueryIterator<'a> {
    Keys(Keys<'a, Arc<PeerAddress>, PeerAddressInfo>),
    Iter(Iter<'a, Arc<PeerAddress>>),
    Alternate(Alternate<Iter<'a, Arc<PeerAddress>>, Iter<'a, Arc<PeerAddress>>>),
}

impl<'a> Iterator for QueryIterator<'a> {
    type Item = &'a Arc<PeerAddress>;

    #[inline]
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        match self {
            QueryIterator::Keys(ref mut keys) => keys.next(),
            QueryIterator::Iter(ref mut iter) => iter.next(),
            QueryIterator::Alternate(ref mut alternate) => alternate.next(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            QueryIterator::Keys(ref keys) => keys.size_hint(),
            QueryIterator::Iter(ref iter) => iter.size_hint(),
            QueryIterator::Alternate(ref alternate) => alternate.size_hint(),
        }
    }

    #[inline]
    fn count(self) -> usize {
        match self {
            QueryIterator::Keys(keys) => keys.count(),
            QueryIterator::Iter(iter) => iter.count(),
            QueryIterator::Alternate(alternate) => alternate.count(),
        }
    }
}

pub const MAX_DISTANCE: u8 = 4;
pub const MAX_FAILED_ATTEMPTS_WS: u32 = 3;
pub const MAX_FAILED_ATTEMPTS_RTC: u32 = 2;

const MAX_TIMESTAMP_DRIFT: Duration = Duration::from_secs(60 * 10);
// 10 minutes
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(60);
// 1 minute
const DEFAULT_BAN_TIME: Duration = Duration::from_secs(60 * 10);
// 10 minutes
pub const INITIAL_FAILED_BACKOFF: Duration = Duration::from_secs(30);
// 30 seconds
pub const MAX_FAILED_BACKOFF: Duration = Duration::from_secs(60 * 10); // 10 minutes

const MAX_SIZE_WS: usize = 10000;
// TODO different for browser
const MAX_SIZE_WSS: usize = 10000;
const MAX_SIZE_RTC: usize = 10000;
const MAX_SIZE: usize = 20500;
// Includes dumb peers
const MAX_SIZE_PER_IP: usize = 250;

const SEEDING_TIMEOUT: Duration = Duration::from_secs(3); // 3 seconds
