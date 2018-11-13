use network::address::peer_address::PeerAddress;
use network;
use std::sync::Arc;
use std::collections::HashMap;
use network::peer_channel::Session;

pub struct PeerAddressInfo {
    pub peer_address: Arc<PeerAddress>,
    pub state: PeerAddressState,
    pub signal_router: SignalRouter,
    pub last_connected: u64,
    pub failed_attempts: u32,
    pub banned_until: i32
}

impl PeerAddressInfo {
    pub fn new(peer_address: Arc<PeerAddress>) -> PeerAddressInfo {
        return PeerAddressInfo {
            peer_address: Arc::clone(&peer_address),
            state: PeerAddressState::New,
            signal_router: SignalRouter::new(peer_address),
            last_connected: 0,
            failed_attempts: 0,
            banned_until: 0
        };
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
    pub fn new(peer_address: Arc<PeerAddress>) -> SignalRouter {
        return SignalRouter {
            best_route: None,
            peer_address: peer_address
        };
    }

    pub fn add_route(&mut self, signal_channel: Session, distance: u8, timestamp: u64) -> bool {
        let mut new_route = SignalRouteInfo::new(signal_channel, distance, timestamp);

        // TODO old route

        let mut is_new_best = false;
        match &self.best_route {
            Some(route) =>  {
                is_new_best = new_route.score() > route.score() || (new_route.score() == route.score() && timestamp > route.timestamp);
            },
            None => {
                is_new_best = true;
            }
        }
        if is_new_best {
            if let Some(ref mut peer_addr_mut) = Arc::get_mut(&mut self.peer_address) {
                peer_addr_mut.distance = new_route.distance;
            }
            self.best_route = Some(new_route);
        }

        return false;
    }
}

pub struct SignalRouteInfo {
    failed_attempts: u32,
    pub timestamp: u64,
    pub signal_channel: Session,
    distance: u8
}

impl SignalRouteInfo {
    pub fn new(signal_channel: Session, distance: u8, timestamp: u64) -> SignalRouteInfo {
        return SignalRouteInfo {
            failed_attempts: 0,
            timestamp: timestamp,
            signal_channel: signal_channel,
            distance: distance
        }
    }

    pub fn score(&self) -> u32 {
        ((network::address::peer_address_book::MAX_DISTANCE - self.distance) / 2) as u32 * (1 - self.failed_attempts / network::address::peer_address_book::MAX_FAILED_ATTEMPTS_RTC)
    }
}
