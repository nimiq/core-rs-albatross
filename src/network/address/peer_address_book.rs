use crate::network::address::PeerId;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use crate::network::address::net_address::NetAddress;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::peer_address_state::PeerAddressInfo;
use crate::network::address::peer_address_state::PeerAddressState;
use crate::utils;
use crate::utils::get_current_time_millis;
use crate::network::Protocol;
use crate::network::peer_channel::Session;

pub struct PeerAddressBook {
    state_by_address: HashMap<Arc<PeerAddress>, Arc<PeerAddressInfo>>,
    state_by_ws_address: HashMap<Arc<PeerAddress>, Arc<PeerAddressInfo>>,
    state_by_wss_address: HashMap<Arc<PeerAddress>, Arc<PeerAddressInfo>>,
    state_by_rtc_address: HashMap<Arc<PeerAddress>, Arc<PeerAddressInfo>>,
    state_by_peer_id: HashMap<PeerId, Arc<PeerAddressInfo>>,
    states_by_net_addr: HashMap<NetAddress, Vec<PeerAddressInfo>>
}

impl PeerAddressBook {
    pub fn get_info(&self, peer_address: Arc<PeerAddress>) -> Option<&Arc<PeerAddressInfo>> {
        return self.state_by_address.get(&peer_address);
    }

    pub fn get_by_peer_id(&self, peer_id: &PeerId) {

    }

    pub fn get_channel_by_peer_id(&self, peer_id: &PeerId) -> Option<&Session> {
        if let Some(info) = self.state_by_peer_id.get(peer_id) {
            if let Some(ref best_route) = info.signal_router.best_route {
                return Some(&best_route.signal_channel);
            }
        }
        return None;
    }

    pub fn query(max_addresses: u32) -> Vec<PeerAddress> {
        unimplemented!()
    }

    pub fn add(&self, channel: Option<&Session>, peer_addresses: Vec<PeerAddress>) {
        for peer_address in peer_addresses {
            self.add_single(channel, peer_address);
        }
    }

    fn add_single(&self, channel: Option<&Session>, peer_address: PeerAddress) -> bool {
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
        let info_option = self.state_by_address.get(&addr_arc);
        if let Some(info) = info_option {

            // Ignore address if it is banned.
            if let PeerAddressState::Banned = info.state {
                return false;
            }

            // Never update seed peers.
            if info.peer_address.is_seed() {
                return false;
            }
        } else {
            // New address, check max book size.
            if self.state_by_address.len() >= MAX_SIZE {
                return false;
            }

            // Check max size per protocol.
            match addr_arc.ty.protocol() {
                Protocol::Ws =>
                    if self.state_by_ws_address.len() >= MAX_SIZE_WS {
                        return false;
                    }
                Protocol::Wss =>
                    if self.state_by_wss_address.len() >= MAX_SIZE_WSS {
                        return false;
                    }
                Protocol::Rtc =>
                    if self.state_by_rtc_address.len() >= MAX_SIZE_RTC {
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

    fn add_to_store(&self, info: PeerAddressInfo) {

    }

    /// Called when a connection to this peerAddress has been established.
    /// The connection might have been initiated by the other peer, so address may not be known previously.
    /// If it is already known, it has been updated by a previous version message.
    pub fn established(&mut self, channel: &Session, peer_address: Arc<PeerAddress>) {
        if self.state_by_address.get(&Arc::clone(&peer_address)).is_none() {
            self.add_to_store(PeerAddressInfo::new(Arc::clone(&peer_address)));
        }

        if let Some(info_mut) = Arc::get_mut(self.state_by_address.get_mut(&peer_address).unwrap()) {
            info_mut.state = PeerAddressState::Established;
            info_mut.last_connected = utils::get_current_time_millis();
            info_mut.failed_attempts = 0;
            info_mut.banned_until = -1;

            if !info_mut.peer_address.is_seed() {
                info_mut.peer_address = Arc::clone(&peer_address);
            }
        }
    }

    /// Called when a connection to this peerAddress is closed.
    pub fn close(&mut self, channel: &Session, peer_address: Arc<PeerAddress>) {
        if let Entry::Occupied(e) = self.state_by_address.entry(Arc::clone(&peer_address)) {

        }
    }

    /// Called when a message has been returned as unroutable.
    pub fn unroutable(&self, channel: &Session, peer_address: Arc<PeerAddress>) {

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
}

const MAX_AGE_WEBSOCKET: u32 = 1000 * 60 * 30; // 30 minutes
const MAX_AGE_WEBRTC: u32 = 1000 * 60 * 15; // 10 minutes
const MAX_AGE_DUMB: u32 = 1000 * 60; // 1 minute
pub const MAX_DISTANCE: u8 = 4;
const MAX_FAILED_ATTEMPTS_WS: u32 = 3;
pub const MAX_FAILED_ATTEMPTS_RTC: u32 = 2;
const MAX_TIMESTAMP_DRIFT: u64 = 1000 * 60 * 10; // 10 minutes
const HOUSEKEEPING_INTERVAL: u32 = 1000 * 60; // 1 minute
const DEFAULT_BAN_TIME: u32 = 1000 * 60 * 10; // 10 minutes
const INITIAL_FAILED_BACKOFF: u32 = 1000 * 30; // 30 seconds
const MAX_FAILED_BACKOFF: u32 = 1000 * 60 * 10; // 10 minutes
const MAX_SIZE_WS: usize = 10000; // TODO different for browser
const MAX_SIZE_WSS: usize = 10000;
const MAX_SIZE_RTC: usize = 10000;
const MAX_SIZE: usize = 20500; // Includes dumb peers
const MAX_SIZE_PER_IP: u32 = 250;
const SEEDING_TIMEOUT: u32 = 1000 * 3; // 3 seconds
