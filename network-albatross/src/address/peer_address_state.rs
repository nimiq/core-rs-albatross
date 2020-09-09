use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::connection::close_type::CloseType;
use crate::peer_channel::PeerChannel;
use peer_address::address::{NetAddress, PeerAddress};
use peer_address::protocol::Protocol;

pub struct PeerAddressInfo {
    pub peer_address: Arc<PeerAddress>,
    pub state: PeerAddressState,
    pub signal_router: SignalRouter,
    pub last_connected: Option<SystemTime>,
    pub failed_attempts: u32,
    pub banned_until: Option<Instant>,
    pub ban_backoff: Duration,

    pub close_types: HashMap<CloseType, usize>,
    pub added_by: HashSet<Arc<NetAddress>>,
}

impl PeerAddressInfo {
    pub fn new(peer_address: Arc<PeerAddress>) -> Self {
        PeerAddressInfo {
            peer_address: Arc::clone(&peer_address),
            state: PeerAddressState::New,
            signal_router: SignalRouter::new(peer_address),
            last_connected: None,
            failed_attempts: 0,
            banned_until: None,
            ban_backoff: super::peer_address_book::INITIAL_FAILED_BACKOFF,
            close_types: HashMap::new(),
            added_by: HashSet::new(),
        }
    }

    pub fn max_failed_attempts(&self) -> u32 {
        match self.peer_address.protocol() {
            Protocol::Rtc => super::peer_address_book::MAX_FAILED_ATTEMPTS_RTC,
            Protocol::Ws | Protocol::Wss => super::peer_address_book::MAX_FAILED_ATTEMPTS_WS,
            _ => 0,
        }
    }

    pub fn close(&mut self, ty: CloseType) {
        *self.close_types.entry(ty).or_insert(0) += 1;

        if self.state == PeerAddressState::Banned {
            return;
        }

        if ty.is_banning_type() {
            self.state = PeerAddressState::Banned;
        } else if ty.is_failing_type() {
            self.state = PeerAddressState::Failed;
        } else {
            self.state = PeerAddressState::Tried;
        }
    }
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum PeerAddressState {
    New = 1,
    Established = 2,
    Tried = 3,
    Failed = 4,
    Banned = 5,
}

pub struct SignalRouter {
    peer_address: Arc<PeerAddress>,
    pub best_route: Option<SignalRouteInfo>,
    routes: HashSet<SignalRouteInfo>,
}

impl SignalRouter {
    pub fn new(peer_address: Arc<PeerAddress>) -> Self {
        SignalRouter {
            peer_address,
            best_route: None,
            routes: HashSet::new(),
        }
    }

    /// Adds a new route and returns whether we have a new best route
    pub fn add_route(
        &mut self,
        signal_channel: Arc<PeerChannel>,
        distance: u8,
        timestamp: u64,
    ) -> bool {
        let mut new_route = SignalRouteInfo::new(signal_channel, distance, timestamp);
        // SignalRouteInfo matches only on signal_channel, so this will get us the old route with the same channel
        let old_route = self.routes.get(&new_route);

        if let Some(old_route) = old_route {
            // Do not reset failed attempts.
            new_route.failed_attempts = old_route.failed_attempts;
        }
        self.routes.replace(new_route.clone());

        let is_new_best = match &self.best_route {
            Some(route) => {
                new_route.score() > route.score()
                    || (new_route.score() == route.score() && timestamp > route.timestamp)
            }
            None => true,
        };
        if is_new_best {
            if let Some(ref mut peer_addr_mut) = Arc::get_mut(&mut self.peer_address) {
                peer_addr_mut.distance = new_route.distance;
            }
            self.best_route = Some(new_route);
            return true;
        }

        false
    }

    pub fn delete_best_route(&mut self) {
        if let Some(best_route) = &self.best_route {
            let signal_channel = best_route.signal_channel.clone();
            self.delete_route(signal_channel);
        }
    }

    pub fn delete_route(&mut self, signal_channel: Arc<PeerChannel>) {
        let route = SignalRouteInfo::new(signal_channel, 0, 0); // Equality is determined by comparing signal_channel only
        self.routes.remove(&route);
        if let Some(best_route) = &self.best_route {
            if *best_route == route {
                self.update_best_route();
            }
        }
    }

    pub fn delete_all_routes(&mut self) {
        self.best_route = None;
        self.routes.clear();
    }

    pub fn has_route(&self) -> bool {
        !self.routes.is_empty()
    }

    pub fn update_best_route(&mut self) {
        let mut best_route: Option<SignalRouteInfo> = None;

        // Choose the route with minimal distance and maximal timestamp.
        for route in self.routes.iter() {
            match best_route {
                Some(ref mut best_route) => {
                    if route.score() > best_route.score()
                        || (route.score() == best_route.score()
                            && route.timestamp > best_route.timestamp)
                    {
                        *best_route = route.clone()
                    }
                }
                None => best_route = Some(route.clone()),
            }
        }
        self.best_route = best_route;

        let mut distance = super::peer_address_book::MAX_DISTANCE + 1;
        if let Some(ref best_route) = self.best_route {
            distance = best_route.distance;
        }

        if let Some(peer_address) = Arc::get_mut(&mut self.peer_address) {
            peer_address.distance = distance;
        }
    }
}

#[derive(Clone)]
pub struct SignalRouteInfo {
    failed_attempts: u32,
    pub timestamp: u64,
    pub signal_channel: Arc<PeerChannel>,
    distance: u8,
}

impl SignalRouteInfo {
    pub fn new(signal_channel: Arc<PeerChannel>, distance: u8, timestamp: u64) -> Self {
        let signal_channel = signal_channel;
        SignalRouteInfo {
            failed_attempts: 0,
            timestamp,
            signal_channel,
            distance,
        }
    }

    pub fn score(&self) -> u32 {
        u32::from((super::peer_address_book::MAX_DISTANCE - self.distance) / 2)
            * (1 - self.failed_attempts / super::peer_address_book::MAX_FAILED_ATTEMPTS_RTC)
    }
}

impl PartialEq for SignalRouteInfo {
    fn eq(&self, other: &SignalRouteInfo) -> bool {
        // We consider signal route infos to be equal if their signal_channel is equal
        self.signal_channel == other.signal_channel
        /* failed_attempts is ignored */
        /* timestamp is ignored */
        /* distance is ignored */
    }
}

impl Eq for SignalRouteInfo {}

impl Hash for SignalRouteInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.signal_channel.hash(state);
    }
}
