use crate::network::address::peer_address::PeerAddress;
use crate::network;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use crate::network::peer_channel::PeerChannel;
use crate::network::Protocol;

pub struct PeerAddressInfo {
    pub peer_address: Arc<PeerAddress>,
    pub state: PeerAddressState,
    pub signal_router: SignalRouter,
    pub last_connected: Option<SystemTime>,
    pub failed_attempts: u32,
    pub banned_until: Option<SystemTime>,
    pub ban_backoff: Duration
}

impl PeerAddressInfo {
    pub fn new(peer_address: Arc<PeerAddress>) -> Self {
        return PeerAddressInfo {
            peer_address: Arc::clone(&peer_address),
            state: PeerAddressState::New,
            signal_router: SignalRouter::new(peer_address),
            last_connected: None,
            failed_attempts: 0,
            banned_until: None,
            ban_backoff: network::address::peer_address_book::INITIAL_FAILED_BACKOFF
        };
    }

    pub fn max_failed_attempts(&self) -> u32 {
        match self.peer_address.protocol() {
            Protocol::Rtc => network::address::peer_address_book::MAX_FAILED_ATTEMPTS_RTC,
            Protocol::Ws | Protocol::Wss => network::address::peer_address_book::MAX_FAILED_ATTEMPTS_WS,
            default => 0
        }
    }
}

#[derive(PartialEq)]
pub enum PeerAddressState {
    New = 1,
    Established,
    Tried,
    Failed,
    Banned
}

pub struct SignalRouter {
    pub best_route: Option<SignalRouteInfo>,
    peer_address: Arc<PeerAddress>
}

impl SignalRouter {
    pub fn new(peer_address: Arc<PeerAddress>) -> Self {
        return SignalRouter {
            best_route: None,
            peer_address
        };
    }

    pub fn add_route(&mut self, signal_channel: Arc<PeerChannel>, distance: u8, timestamp: u64) -> bool {
        let new_route = SignalRouteInfo::new(signal_channel, distance, timestamp);

        // TODO old route

        let is_new_best = match &self.best_route {
            Some(route) => new_route.score() > route.score()
                || (new_route.score() == route.score() && timestamp > route.timestamp),
            None => true
        };
        if is_new_best {
            if let Some(ref mut peer_addr_mut) = Arc::get_mut(&mut self.peer_address) {
                peer_addr_mut.distance = new_route.distance;
            }
            self.best_route = Some(new_route);
        }

        return false;
    }

    pub fn delete_best_route(&self) {
        unimplemented!()
    }

    pub fn delete_route(&self, signal_channel: Arc<PeerChannel>) {
        unimplemented!()
    }

    pub fn delete_all_routes(&mut self) {
        self.best_route = None;
        unimplemented!()
    }

    pub fn has_route(&self) -> bool {
        unimplemented!()
    }
}

pub struct SignalRouteInfo {
    failed_attempts: u32,
    pub timestamp: u64,
    pub signal_channel: Arc<PeerChannel>,
    distance: u8
}

impl SignalRouteInfo {
    pub fn new(signal_channel: Arc<PeerChannel>, distance: u8, timestamp: u64) -> Self {
        return SignalRouteInfo {
            failed_attempts: 0,
            timestamp,
            signal_channel,
            distance
        }
    }

    pub fn score(&self) -> u32 {
        ((network::address::peer_address_book::MAX_DISTANCE - self.distance) / 2) as u32 * (1 - self.failed_attempts / network::address::peer_address_book::MAX_FAILED_ATTEMPTS_RTC)
    }
}
