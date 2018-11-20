use std::cmp;
use std::collections::hash_map::Entry;
use std::collections::hash_map::Keys;
use std::collections::hash_set::Iter;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::iter::Iterator;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use rand::{OsRng, Rng};

use crate::consensus::networks::{get_network_info, NetworkId};
use crate::network::{NetworkTime, ProtocolFlags};
use crate::network::address::net_address::NetAddress;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::peer_address_state::PeerAddressInfo;
use crate::network::address::peer_address_state::PeerAddressState;
use crate::network::address::PeerId;
use crate::network::connection::close_type::CloseType;
use crate::network::network_config::NetworkConfig;
use crate::network::peer_channel::PeerChannel;
use crate::network::Protocol;
use crate::utils;
use crate::utils::iterators::Alternate;
use crate::utils::services::ServiceFlags;
use crate::utils::systemtime_to_timestamp;
use crate::utils::timers::Timers;

pub struct PeerAddressBook {
    info_by_address: HashMap<Arc<PeerAddress>, PeerAddressInfo>,
    ws_addresses: HashSet<Arc<PeerAddress>>,
    wss_addresses: HashSet<Arc<PeerAddress>>,
    rtc_addresses: HashSet<Arc<PeerAddress>>,
    address_by_peer_id: HashMap<PeerId, Arc<PeerAddress>>,
    addresses_by_net_address: HashMap<NetAddress, HashSet<Arc<PeerAddress>>>,
    seeded: bool,
    network_config: Arc<NetworkConfig>,
    timers: Timers<PeerAddressBookTimer>,
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
enum PeerAddressBookTimer {
    Housekeeping,
}

impl PeerAddressBook {
    pub fn new(network_config: Arc<NetworkConfig>) -> Self {
        let mut ret = PeerAddressBook {
            info_by_address: HashMap::new(),
            ws_addresses: HashSet::new(),
            wss_addresses: HashSet::new(),
            rtc_addresses: HashSet::new(),
            address_by_peer_id: HashMap::new(),
            addresses_by_net_address: HashMap::new(),
            seeded: false,
            network_config,
            timers: Timers::new(),
        };

        // Init hardcoded seed peers.
        if let Some(network_info) = get_network_info(NetworkId::Main) {
            for peer_address in network_info.seed_peers.iter() {
                ret.add_single(None, peer_address.clone());
            }
        }

        // Setup housekeeping interval.
        ret.timers.set_interval(PeerAddressBookTimer::Housekeeping, || {
            // TODO Call housekeeping.
        }, HOUSEKEEPING_INTERVAL);

        return ret;
    }

    pub fn address_iter(&self) -> Keys<Arc<PeerAddress>, PeerAddressInfo> {
        return self.info_by_address.keys();
    }

    pub fn ws_address_iter(&self) -> Iter<Arc<PeerAddress>> {
        return self.ws_addresses.iter();
    }

    pub fn wss_address_iter(&self) -> Iter<Arc<PeerAddress>> {
        return self.wss_addresses.iter();
    }

    pub fn rtc_address_iter(&self) -> Iter<Arc<PeerAddress>> {
        return self.rtc_addresses.iter();
    }

    pub fn get_info(&self, peer_address: &Arc<PeerAddress>) -> Option<&PeerAddressInfo> {
        return self.info_by_address.get(peer_address);
    }

    pub fn get_by_peer_id(&self, peer_id: &PeerId) -> Option<Arc<PeerAddress>> {
        if let Some(peer_address) = self.address_by_peer_id.get(peer_id) {
            return Some(Arc::clone(&peer_address));
        }
        return None;
    }

    pub fn get_channel_by_peer_id(&self, peer_id: &PeerId) -> Option<&PeerChannel> {
        if let Some(peer_address) = self.address_by_peer_id.get(peer_id) {
            if let Some(info) = self.info_by_address.get(peer_address) {
                if let Some(ref best_route_opt) = info.signal_router.best_route {
                    return Some(&best_route_opt.signal_channel);
                }
            }
        }
        return None;
    }

    pub fn query(&self, protocol_mask: ProtocolFlags, service_mask: ServiceFlags, max_addresses: u16) -> Vec<Arc<PeerAddress>> {
        let max_addresses = max_addresses as usize; // Internally, we need a usize.

        let iterator;
        let num_addresses;

        if protocol_mask == ProtocolFlags::WSS {
            num_addresses = self.known_wss_addresses_count();
            iterator = QueryIterator::Iter(self.wss_address_iter());
        } else if protocol_mask == ProtocolFlags::WS {
            num_addresses = self.known_ws_addresses_count();
            iterator = QueryIterator::Iter(self.ws_address_iter());
        } else if protocol_mask == ProtocolFlags::WS | ProtocolFlags::WSS {
            num_addresses = self.known_ws_addresses_count() + self.known_wss_addresses_count();
            iterator = QueryIterator::Alternate(Alternate::new(self.ws_address_iter(), self.wss_address_iter()));
        } else if protocol_mask == ProtocolFlags::RTC {
            num_addresses = self.known_rtc_addresses_count();
            iterator = QueryIterator::Iter(self.rtc_address_iter());
        } else if protocol_mask == ProtocolFlags::RTC | ProtocolFlags::WS {
            num_addresses = self.known_rtc_addresses_count() + self.known_ws_addresses_count();
            iterator = QueryIterator::Alternate(Alternate::new(self.rtc_address_iter(), self.ws_address_iter()));
        } else if protocol_mask == ProtocolFlags::RTC | ProtocolFlags::WSS {
            num_addresses = self.known_rtc_addresses_count() + self.known_wss_addresses_count();
            iterator = QueryIterator::Alternate(Alternate::new(self.rtc_address_iter(), self.wss_address_iter()));
        } else {
            num_addresses = self.known_addresses_count();
            iterator = QueryIterator::Keys(self.address_iter());
        }

        let mut start_index = 0;
        // Pick a random start index if we have a lot of addresses.
        if num_addresses > max_addresses {
            let mut cspring: OsRng = OsRng::new().unwrap();
            start_index = cspring.gen_range(0, num_addresses);
        }

        // XXX inefficient linear scan
        iterator.cycle().skip(start_index).take(max_addresses)
            .filter(|&peer_address| {
                if let Some(info) = self.info_by_address.get(peer_address) {
                    // Never return banned or failed addresses.
                    if info.state == PeerAddressState::Banned || info.state == PeerAddressState::Failed {
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
                    if service_mask.intersects(peer_address.services) {
                        return false;
                    }

                    // TODO Exclude RTC addresses that are already at MAX_DISTANCE.

                    // Never return addresses that are too old.
                    if peer_address.exceeds_age() {
                        return false;
                    }

                    return true;
                }
                return false;
            })
            .map(|peer_address| peer_address.clone())
            .collect()
    }

    pub fn add(&mut self, channel: Option<&PeerChannel>, peer_addresses: Vec<PeerAddress>) {
        for peer_address in peer_addresses {
            self.add_single(channel, peer_address);
        }

        // TODO Tell listeners that we have learned new addresses.
    }

    fn add_single(&mut self, channel: Option<&PeerChannel>, mut peer_address: PeerAddress) -> bool {
        // Ignore our own address.
        if self.network_config.peer_address() == peer_address {
            return false;
        }

        // Ignore address if it is too old.
        // Special case: allow seed addresses (timestamp == 0) via null channel.
        if peer_address.exceeds_age() {
            if let Some(_) = channel {
                return false;
            }
        }

        // Ignore address if its timestamp is too far in the future.
        if peer_address.timestamp > systemtime_to_timestamp(SystemTime::now() + MAX_TIMESTAMP_DRIFT) {
            return false;
        }

        // Increment distance values of RTC addresses.
        // TODO

        // Get the (reliable) netAddress of the peer that sent us this address.
        let net_address = channel.and_then(|channel| {
            if let Some(net_address) = channel.address_info.net_address() {
                if net_address.is_reliable() {
                    return Some(net_address);
                }
            }
            None
        });

        // Check if we already know this address.
        let mut addr_arc = Arc::new(peer_address);
        let mut known_address: Option<Arc<PeerAddress>> = None;
        let mut changed = false;
        if let Some(info) = self.info_by_address.get_mut(&addr_arc) {
            // Update address.
            known_address = Some(info.peer_address.clone());

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
                    peer_address.net_address = info.peer_address.net_address.clone();
                }
            }

            // Update address if it has a more recent timestamp.
            if info.peer_address.timestamp < addr_arc.timestamp {
                info.peer_address = addr_arc.clone();
            }
        } else {
            // New address, check max book size.
            if self.info_by_address.len() >= MAX_SIZE {
                return false;
            }

            // Check max size per protocol.
            match addr_arc.ty.protocol() {
                Protocol::Ws =>
                    if self.ws_addresses.len() >= MAX_SIZE_WS {
                        return false;
                    },
                Protocol::Wss =>
                    if self.wss_addresses.len() >= MAX_SIZE_WSS {
                        return false;
                    },
                Protocol::Rtc =>
                    if self.rtc_addresses.len() >= MAX_SIZE_RTC {
                        return false;
                    },
                Protocol::Dumb => {}, // Dumb addresses are only part of global limit.
            }

            // If we know the IP address of the sender, check that we don't exceed the maximum number of addresses per IP.
            if let Some(ref net_address) = net_address {
                let states = self.addresses_by_net_address.get(&net_address);
                if let Some(states) = states {
                    if states.len() >= MAX_SIZE_PER_IP {
                        return false;
                    }
                }
            }

            // Add new peerAddressState.
            let new_info = PeerAddressInfo::new(addr_arc.clone());
            self.add_to_store(new_info);
            changed = true;
        }

        // TODO Add route.


        // Track which IP address send us this address.
        self.track_by_net_address(addr_arc, net_address);

        return changed;
    }

    fn track_by_net_address(&mut self, peer_address: Arc<PeerAddress>, net_address: Option<Arc<NetAddress>>) {
        // TODO
//        if let Some(net_address) = net_address {
//            if let Some(info) = self.info_by_address.get_mut(&peer_address) {
//                // TODO Store added_by
//            }
//
//            self.addresses_by_net_address.entry(net_address.as_ref().clone())
//                .or_insert_with(HashSet::new)
//                .insert(peer_address);
//        }
    }

    fn add_to_store(&mut self, info: PeerAddressInfo) {
        // Index by peerId.
        self.address_by_peer_id.insert(info.peer_address.peer_id.clone(), Arc::clone(&info.peer_address));

        match info.peer_address.protocol() {
            Protocol::Ws => {
                self.ws_addresses.insert(Arc::clone(&info.peer_address));
            },
            Protocol::Wss => {
                self.wss_addresses.insert(Arc::clone(&info.peer_address));
            },
            Protocol::Rtc => {
                self.rtc_addresses.insert(Arc::clone(&info.peer_address));
            },
            Protocol::Dumb => { } // Dumb addresses are ignored.
        };

        self.info_by_address.insert(Arc::clone(&info.peer_address), info);
    }

    /// Called when a connection to this peerAddress has been established.
    /// The connection might have been initiated by the other peer, so address may not be known previously.
    /// If it is already known, it has been updated by a previous version message.
    pub fn established(&mut self, channel: &PeerChannel, peer_address: Arc<PeerAddress>) {
        if self.info_by_address.get(&Arc::clone(&peer_address)).is_none() {
            self.add_to_store(PeerAddressInfo::new(Arc::clone(&peer_address)));
        }

        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            // Get the (reliable) netAddress of the peer that sent us this address.
            let net_address: Option<Arc<NetAddress>> = channel.address_info.net_address().and_then(|net_address| {
                if net_address.is_reliable() {
                    Some(net_address)
                } else {
                    None
                }
            });
            // TODO self.track_by_net_address(peer_address.clone(), net_address);

            info.state = PeerAddressState::Established;
            info.last_connected = Some(SystemTime::now());
            info.failed_attempts = 0;
            info.banned_until = None;
            info.ban_backoff = INITIAL_FAILED_BACKOFF;

            if !info.peer_address.is_seed() {
                info.peer_address = Arc::clone(&peer_address);
            }

            // TODO Add route.
        }
    }

    /// Called when a connection to this peerAddress is closed.
    pub fn close(&mut self, channel: Option<&PeerChannel>, peer_address: Arc<PeerAddress>, ty: CloseType) {
        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            // Register the type of disconnection.
            info.close(ty);

            // TODO Delete all addresses that were signalable over the disconnected peer.

            if ty.is_failing_type() {
                info.failed_attempts += 1;

                if info.failed_attempts >= info.max_failed_attempts() {
                    // Remove address only if we have tried the maximum number of backoffs.
                    if info.ban_backoff >= MAX_FAILED_BACKOFF {
                        self.remove_from_store(Arc::clone(&peer_address));
                    } else {
                        info.banned_until = Some(Instant::now() + info.ban_backoff);
                        info.ban_backoff = cmp::min(MAX_FAILED_BACKOFF, info.ban_backoff * 2);
                    }
                }
            }

            if ty.is_banning_type() {
                self.ban(Arc::clone(&peer_address), DEFAULT_BAN_TIME);
            }

            // Immediately delete dumb addresses, since we cannot connect to those anyway.
            if peer_address.protocol() == Protocol::Dumb {
                self.remove_from_store(peer_address);
            }
        }
    }

    /// Called when a message has been returned as unroutable.
    pub fn unroutable(&mut self, channel: &PeerChannel, peer_address: Arc<PeerAddress>) {
        if let Some(info) = self.info_by_address.get(&peer_address) {

            // TODO
            if let Some(best_route) = &info.signal_router.best_route {
                unimplemented!()
            }

            info.signal_router.delete_best_route();
            if !info.signal_router.has_route() {
                self.remove_from_store(Arc::clone(&info.peer_address));
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
        return false;
    }

    fn remove_from_store(&mut self, peer_address: Arc<PeerAddress>) {
        // Never delete seed addresses, ban them instead for a couple of minutes.
        if let Some(info) = self.get_info(&peer_address) {
            if info.peer_address.is_seed() {
                self.ban(peer_address.clone(), DEFAULT_BAN_TIME);
                return;
            }
        }

        // Delete from peerId index.
        self.address_by_peer_id.remove(&peer_address.peer_id);

        // TODO Delete from net_address index.

        // Remove from protocol index.
        match peer_address.protocol() {
            Protocol::Ws => {
                self.ws_addresses.remove(&peer_address);
            },
            Protocol::Wss => {
                self.wss_addresses.remove(&peer_address);
            },
            Protocol::Rtc => {
                self.rtc_addresses.remove(&peer_address);
            },
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

    fn housekeeping(&mut self) {
        let now = Instant::now();
        let mut unbanned_addresses: Vec<Arc<PeerAddress>> = Vec::new();

        let mut to_remove_from_store= Vec::new();

        for (peer_address, info) in self.info_by_address.iter_mut() {
            match info.state {
                PeerAddressState::New | PeerAddressState::Tried | PeerAddressState::Failed => {
                    // Delete all new peer addresses that are older than MAX_AGE.
                    if peer_address.exceeds_age() {
                        to_remove_from_store.push(peer_address.clone());
                        continue;
                    }

                    // Reset failed attempts after banned_until has expired.
                    if info.state == PeerAddressState::Failed
                        && info.failed_attempts >= info.max_failed_attempts() {
                        if let Some(ref banned_until) = info.banned_until {
                            if banned_until <= &now {
                                info.banned_until = None;
                                info.failed_attempts = 0;
                                unbanned_addresses.push(peer_address.clone());
                            }
                        }
                    }
                },
                PeerAddressState::Banned => {
                    if let Some(ref banned_until) = info.banned_until {
                        if banned_until <= &now {
                            // Don't remove seed addresses, unban them.
                            if peer_address.is_seed() {
                                // Restore banned seed addresses to the NEW state.
                                info.state = PeerAddressState::New;
                                info.failed_attempts = 0;
                                info.banned_until = None;
                                unbanned_addresses.push(peer_address.clone());
                            } else {
                                // Delete expired bans.
                                to_remove_from_store.push(peer_address.clone());
                            }
                        }
                    }
                },
                PeerAddressState::Established => {
                    // TODO Also update timestamp for RTC connections
                },
                _ => {
                    // TODO What about peers who are stuck connecting? Can this happen?
                    // Do nothing for CONNECTING peers.
                },
            }
        }

        // Remove addresses.
        for peer_address in to_remove_from_store.drain(..) {
            self.remove_from_store(peer_address);
        }

        if unbanned_addresses.len() > 0 {
            // TODO Fire Added event.
        }
    }

    pub fn seeded(&self) -> bool {
        self.seeded
    }

    pub fn known_addresses_count(&self) -> usize { self.info_by_address.len() }
    pub fn known_ws_addresses_count(&self) -> usize { self.ws_addresses.len() }
    pub fn known_wss_addresses_count(&self) -> usize { self.wss_addresses.len() }
    pub fn known_rtc_addresses_count(&self) -> usize { self.rtc_addresses.len() }
}

#[derive(Clone)]
enum QueryIterator<'a> {
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

pub const MAX_AGE_WEBSOCKET: Duration = Duration::from_secs(60 * 30); // 30 minutes
pub const MAX_AGE_WEBRTC: Duration = Duration::from_secs(60 * 15); // 15 minutes
pub const MAX_AGE_DUMB: Duration = Duration::from_secs(60); // 1 minute

pub const MAX_DISTANCE: u8 = 4;
pub const MAX_FAILED_ATTEMPTS_WS: u32 = 3;
pub const MAX_FAILED_ATTEMPTS_RTC: u32 = 2;

const MAX_TIMESTAMP_DRIFT: Duration = Duration::from_secs(60 * 10); // 10 minutes
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(60); // 1 minute
const DEFAULT_BAN_TIME: Duration = Duration::from_secs(60 * 10); // 10 minutes
pub const INITIAL_FAILED_BACKOFF: Duration = Duration::from_secs(30); // 30 seconds
pub const MAX_FAILED_BACKOFF: Duration = Duration::from_secs(60 * 10); // 10 minutes

const MAX_SIZE_WS: usize = 10000; // TODO different for browser
const MAX_SIZE_WSS: usize = 10000;
const MAX_SIZE_RTC: usize = 10000;
const MAX_SIZE: usize = 20500; // Includes dumb peers
const MAX_SIZE_PER_IP: usize = 250;

const SEEDING_TIMEOUT: Duration = Duration::from_secs(3); // 3 seconds
