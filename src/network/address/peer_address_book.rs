use crate::network::Protocol;
use crate::network::peer_channel::Session;
use crate::network::address::PeerId;
use crate::network::address::net_address::NetAddress;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::peer_address_state::PeerAddressInfo;
use crate::network::address::peer_address_state::PeerAddressState;
use crate::network::connection::close_type::CloseType;
use std::cmp;
use std::collections::hash_map::Entry;
use std::collections::hash_set::Iter;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use crate::utils;
use crate::utils::get_current_time_millis;

pub struct PeerAddressBook {
    info_by_address: HashMap<Arc<PeerAddress>, PeerAddressInfo>,
    ws_addresses: HashSet<Arc<PeerAddress>>,
    wss_addresses: HashSet<Arc<PeerAddress>>,
    rtc_addresses: HashSet<Arc<PeerAddress>>,
    address_by_peer_id: HashMap<PeerId, Arc<PeerAddress>>,
    address_by_net_addr: HashMap<NetAddress, Vec<PeerAddress>>
}

impl PeerAddressBook {
    pub fn get_ws(&self) -> Iter<Arc<PeerAddress>> {
        return self.ws_addresses.iter();
    }

    pub fn get_info(&self, peer_address: Arc<PeerAddress>) -> Option<&PeerAddressInfo> {
        return self.info_by_address.get(&peer_address);
    }

    pub fn get_by_peer_id(&self, peer_id: &PeerId) -> Option<Arc<PeerAddress>> {
        if let Some(peer_address) = self.address_by_peer_id.get(peer_id) {
            return Some(Arc::clone(&peer_address));
        }
        return None;
    }

    pub fn get_channel_by_peer_id(&self, peer_id: &PeerId) -> Option<&Session> {
        if let Some(peer_address) = self.address_by_peer_id.get(peer_id) {
            if let Some(info) = self.info_by_address.get(peer_address) {
                if let Some(ref best_route_opt) = info.signal_router.best_route {
                    return Some(&best_route_opt.signal_channel);
                }
            }
        }
        return None;
    }

    pub fn query(max_addresses: u32) -> Vec<PeerAddress> {
        unimplemented!()
    }

    pub fn add(&mut self, channel: Option<&Session>, peer_addresses: Vec<PeerAddress>) {
        for peer_address in peer_addresses {
            self.add_single(channel, peer_address);
        }
    }

    fn add_single(&mut self, channel: Option<&Session>, peer_address: PeerAddress) -> bool {
        let addr_arc = Arc::new(peer_address);
        // Ignore our own address.
        // TODO

        // Ignore address if it is too old.
        // Special case: allow seed addresses (timestamp == 0) via null channel.
        if addr_arc.exceeds_age() {
            if let Some(_) = channel {
                return false;
            }
        }

        // Ignore address if its timestamp is too far in the future.
        if addr_arc.timestamp > get_current_time_millis() + MAX_TIMESTAMP_DRIFT {
            return false;
        }

        // Increment distance values of RTC addresses.
        // TODO

        // Get the (reliable) netAddress of the peer that sent us this address.
        // TODO

        // Check if we already know this address.
        let mut changed = false;
        if let Some(info) = self.info_by_address.get(&addr_arc) {

            // Ignore address if it is banned.
            if let PeerAddressState::Banned = info.state {
                return false;
            }

            // Never update seed peers.
            if info.peer_address.is_seed() {
                return false;
            }

            // Never erase NetAddresses and never overwrite reliable addresses.
            // TODO
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
                    }
                Protocol::Wss =>
                    if self.wss_addresses.len() >= MAX_SIZE_WSS {
                        return false;
                    }
                Protocol::Rtc =>
                    if self.rtc_addresses.len() >= MAX_SIZE_RTC {
                        return false;
                    }
                Protocol::Dumb => { }// Dumb addresses are only part of global limit.
            }

            // If we know the IP address of the sender, check that we don't exceed the maximum number of addresses per IP.
            // TODO

            // Add new peerAddressState.
            let new_info = PeerAddressInfo::new(Arc::clone(&addr_arc));
            self.add_to_store(new_info);
            changed = true;
        }

        return changed;
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
    pub fn established(&mut self, channel: &Session, peer_address: Arc<PeerAddress>) {
        if self.info_by_address.get(&Arc::clone(&peer_address)).is_none() {
            self.add_to_store(PeerAddressInfo::new(Arc::clone(&peer_address)));
        }

        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            info.state = PeerAddressState::Established;
            info.last_connected = utils::get_current_time_millis();
            info.failed_attempts = 0;
            info.banned_until = 0;

            if !info.peer_address.is_seed() {
                info.peer_address = Arc::clone(&peer_address);
            }
        }
    }

    /// Called when a connection to this peerAddress is closed.
    pub fn close(&mut self, channel: &Session, peer_address: Arc<PeerAddress>, ty: CloseType) {
        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            if ty.is_failing_type() {
                info.failed_attempts += 1;

                if info.failed_attempts > info.max_failed_attempts() {
                    // Remove address only if we have tried the maximum number of backoffs.
                    if info.ban_backoff >= MAX_FAILED_BACKOFF {
                        self.remove_from_store(Arc::clone(&peer_address));
                    } else {
                        info.banned_until = get_current_time_millis() + info.ban_backoff;
                        info.ban_backoff = cmp::min(MAX_FAILED_BACKOFF, info.ban_backoff * 2);
                    }
                }
            }

            if ty.is_banning_type() {
                self.ban(Arc::clone(&peer_address), DEFAULT_BAN_TIME);
            }

            // Immediately delete dumb addresses, since we cannot connect to those anyway.
            if peer_address.protocol() == Protocol::Dumb {
                self.remove_from_store(Arc::clone(&peer_address));
            }
        }
    }

    /// Called when a message has been returned as unroutable.
    pub fn unroutable(&mut self, channel: &Session, peer_address: Arc<PeerAddress>) {
        if let Some(info) = self.info_by_address.get(&peer_address) {

            if let Some(best_route) = &info.signal_router.best_route {
                unimplemented!()
            }

            info.signal_router.delete_best_route();
            if !info.signal_router.has_route() {
                self.remove_from_store(Arc::clone(&info.peer_address));
            }
        }
    }

    fn ban(&mut self, peer_address: Arc<PeerAddress>, duration: u64) {
        if self.info_by_address.get(&Arc::clone(&peer_address)).is_none() {
            self.add_to_store(PeerAddressInfo::new(Arc::clone(&peer_address)));
        }

        if let Some(info) = self.info_by_address.get_mut(&peer_address) {
            info.state = PeerAddressState::Banned;
            info.banned_until = get_current_time_millis() + duration;

            // Drop all routes to this peer.
            info.signal_router.delete_all_routes();
        }
    }

    pub fn is_banned(&self, peer_address: Arc<PeerAddress>) -> bool {
        if let Some(info) = self.get_info(peer_address) {
            if PeerAddressState::Banned == info.state {
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
        if let Some(info) = self.get_info(Arc::clone(&peer_address)) {
            if info.peer_address.is_seed() {
                self.ban(Arc::clone(&peer_address), DEFAULT_BAN_TIME);
            }
        }

        // Delete from peerId index.
        self.address_by_peer_id.remove(&peer_address.peer_id);

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
            default => {}
        }

        if let Some(info) = self.get_info(Arc::clone(&peer_address)) {

            // Don't delete bans.
            if info.state == PeerAddressState::Banned {
                return;
            }

            // Delete the address.
            self.info_by_address.remove(&peer_address);
        }
    }
}

const MAX_AGE_WEBSOCKET: u32 = 1000 * 60 * 30; // 30 minutes
const MAX_AGE_WEBRTC: u32 = 1000 * 60 * 15; // 10 minutes
const MAX_AGE_DUMB: u32 = 1000 * 60; // 1 minute

pub const MAX_DISTANCE: u8 = 4;
pub const MAX_FAILED_ATTEMPTS_WS: u32 = 3;
pub const MAX_FAILED_ATTEMPTS_RTC: u32 = 2;

const MAX_TIMESTAMP_DRIFT: u64 = 1000 * 60 * 10; // 10 minutes
const HOUSEKEEPING_INTERVAL: i32 = 1000 * 60; // 1 minute
const DEFAULT_BAN_TIME: u64 = 1000 * 60 * 10; // 10 minutes
pub const INITIAL_FAILED_BACKOFF: u64 = 1000 * 30; // 30 seconds
pub const MAX_FAILED_BACKOFF: u64 = 1000 * 60 * 10; // 10 minutes

const MAX_SIZE_WS: usize = 10000; // TODO different for browser
const MAX_SIZE_WSS: usize = 10000;
const MAX_SIZE_RTC: usize = 10000;
const MAX_SIZE: usize = 20500; // Includes dumb peers
const MAX_SIZE_PER_IP: i32 = 250;

const SEEDING_TIMEOUT: i32 = 1000 * 3; // 3 seconds
