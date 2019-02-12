use std::cmp;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::time::{Duration, Instant, SystemTime};

use parking_lot::RwLock;
use rand::{Rng, rngs::OsRng};

use beserial::Serialize;
use blockchain::Blockchain;
use network_messages::*;
use network_primitives::address::peer_address::PeerAddress;
use network_primitives::address::PeerId;
use network_primitives::networks::get_network_info;
use network_primitives::version;
use utils::observer::{Notifier, weak_listener, weak_passthru_listener};
use utils::rate_limit::RateLimit;
use utils::time::systemtime_to_timestamp;
use utils::timers::Timers;
use utils::unique_ptr::UniquePtr;

use crate::address::peer_address_book::PeerAddressBook;
use crate::connection::close_type::CloseType;
use crate::network_config::NetworkConfig;
use crate::Peer;
use crate::peer_channel::PeerChannel;

pub struct NetworkAgent {
    blockchain: Arc<Blockchain<'static>>,
    addresses: Arc<PeerAddressBook>,
    network_config: Arc<NetworkConfig>,
    channel: Arc<PeerChannel>,

    peer: Option<Peer>,

    version_received: bool,
    verack_received: bool,
    version_sent: bool,
    verack_sent: bool,
    version_attempts: usize,
    peer_address_verified: bool,

    ping_times: HashMap<u32, Instant>,

    peer_challenge_nonce: Option<ChallengeNonce>,
    address_request: Option<AddressRequest>,
    get_address_limit: RateLimit,

    challenge_nonce: ChallengeNonce,

    self_weak: Weak<RwLock<NetworkAgent>>,
    pub notifier: Notifier<'static, NetworkAgentEvent>,
    
    timers: Timers<NetworkAgentTimer>,
}

#[derive(Ord, PartialOrd, PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum NetworkAgentTimer {
    Handshake,
    Version,
    VerAck,
    Connectivity,
    AnnounceAddr,
    Ping(u32),
}

pub enum NetworkAgentEvent {
    Version(UniquePtr<Peer>),
    Handshake(UniquePtr<Peer>),
    Addr,
    PingPong(Duration),
}

impl NetworkAgent {
    const VERSION_ATTEMPTS_MAX: usize = 10;
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4); // 4 seconds
    pub const PING_TIMEOUT: Duration = Duration::from_secs(10); // 10 seconds
    const CONNECTIVITY_CHECK_INTERVAL: Duration = Duration::from_secs(60); // 1 minute
    const ANNOUNCE_ADDR_INTERVAL: Duration = Duration::from_secs(60 * 10); // 10 minutes
    const VERSION_RETRY_DELAY: Duration = Duration::from_millis(500); // 500 ms
    const GETADDR_RATE_LIMIT: usize = 3; // per minute
    const MAX_ADDR_PER_MESSAGE: u16 = 1000;
    const MAX_ADDR_PER_REQUEST: u16 = 500;
    const NUM_ADDR_PER_REQUEST: u16 = 200;

    pub fn new(blockchain: Arc<Blockchain<'static>>, addresses: Arc<PeerAddressBook>, network_config: Arc<NetworkConfig>, channel: Arc<PeerChannel>) -> Arc<RwLock<Self>> {
        let agent = Arc::new(RwLock::new(Self {
            blockchain,
            addresses,
            network_config,
            channel,

            peer: None,

            version_received: false,
            verack_received: false,
            version_sent: false,
            verack_sent: false,
            version_attempts: 0,
            peer_address_verified: false,

            ping_times: HashMap::new(),

            peer_challenge_nonce: None,
            address_request: None,
            get_address_limit: RateLimit::new_per_minute(Self::GETADDR_RATE_LIMIT),

            challenge_nonce: ChallengeNonce::generate(),

            self_weak: Weak::new(),
            notifier: Notifier::new(),
            
            timers: Timers::new(),
        }));
        Self::init_listeners(&agent);
        agent
    }

    fn init_listeners(agent: &Arc<RwLock<Self>>) {
        agent.write().self_weak = Arc::downgrade(agent);

        let channel = &agent.read().channel;
        let msg_notifier = &channel.msg_notifier;
        msg_notifier.version.write().register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, msg: VersionMessage| agent.write().on_version(msg)));

        msg_notifier.ver_ack.write().register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, msg: VerAckMessage| agent.write().on_ver_ack(msg)));

        msg_notifier.addr.write().register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, msg: AddrMessage| agent.write().on_addr(msg)));

        msg_notifier.get_addr.write().register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, msg: GetAddrMessage| agent.write().on_get_addr(msg)));

        msg_notifier.ping.write().register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, nonce: u32| agent.write().on_ping(nonce)));

        msg_notifier.pong.write().register(weak_passthru_listener(
            Arc::downgrade(agent),
            |agent, nonce: u32| agent.write().on_pong(nonce)));

        let mut close_notifier = channel.close_notifier.write();
        close_notifier.register(weak_listener(
            Arc::downgrade(agent),
            |agent, _| agent.write().on_close()));
    }

    pub fn handshake(&mut self) {
        if self.version_sent {
            // Version already sent, no need to handshake again.
            return;
        }

        // Kick off the handshake by telling the peer our version, network address & blockchain head hash.
        // Firefox sends the data-channel-open event too early, so sending the version message might fail.
        // Try again in this case.
        let network_info = get_network_info(self.blockchain.network_id).unwrap();
        let msg = VersionMessage::new(
            self.network_config.peer_address(),
            self.blockchain.head_hash(),
            network_info.genesis_hash.clone(),
            self.challenge_nonce.clone(),
            self.network_config.user_agent().clone());
        if self.channel.send(msg).is_err() {
            self.version_attempts += 1;
            if self.version_attempts >= Self::VERSION_ATTEMPTS_MAX || self.channel.closed() {
                self.channel.close(CloseType::SendingVersionMessageFailed);
                return;
            }

            let weak = self.self_weak.clone();
            self.timers.reset_delay(NetworkAgentTimer::Handshake, move || {
                let arc = upgrade_weak!(weak);
                arc.write().handshake();
            }, Self::VERSION_RETRY_DELAY);
            return;
        }

        self.version_sent = true;

        // Drop the peer if it doesn't send us a version message.
        // Only do this if we haven't received the peer's version message already.
        if !self.version_received {
            // TODO Should we ban instead?
            let weak = self.self_weak.clone();
            self.timers.set_delay(NetworkAgentTimer::Version, move || {
                let arc = upgrade_weak!(weak);
                let agent = arc.read();
                agent.timers.clear_delay(&NetworkAgentTimer::Version);
                agent.channel.close(CloseType::VersionTimeout);
            }, Self::HANDSHAKE_TIMEOUT);
        } else if self.peer_address_verified {
            self.send_ver_ack();
        }

        let weak = self.self_weak.clone();
        self.timers.set_delay(NetworkAgentTimer::VerAck, move || {
            let arc = upgrade_weak!(weak);
            let agent = arc.read();
            agent.timers.clear_delay(&NetworkAgentTimer::VerAck);
            agent.channel.close(CloseType::VerackTimeout);
        }, Self::HANDSHAKE_TIMEOUT);
    }

    fn send_ver_ack(&mut self) {
        assert!(self.peer_address_verified);
        assert!(self.peer_challenge_nonce.is_some());

        let msg = VerAckMessage::new(
            &self.channel.address_info.peer_address().unwrap().peer_id,
            self.peer_challenge_nonce.as_ref().unwrap(),
            self.network_config.key_pair());
        self.channel.send_or_close(msg);

        self.verack_sent = true;
    }

    fn on_version(&mut self, msg: VersionMessage) {
        debug!("[VERSION] {} {} {}", &msg.peer_address, &msg.head_hash, &msg.user_agent.unwrap_or_else(|| String::from("None")));

        let now = SystemTime::now();

        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::Version) {
            return;
        }

        // Ignore duplicate version messages.
        if self.version_received {
            debug!("Ignoring duplicate version message from {}", self.channel.address_info.peer_address().unwrap());
            return;
        }

        // Clear the version timeout.
        self.timers.clear_delay(&NetworkAgentTimer::Version);

        // Check if the peer is running a compatible version.
        if !version::is_compatible(msg.version) {
            self.channel.send_or_close(RejectMessage::new(
                MessageType::Version,
                RejectMessageCode::Obsolete,
                format!("incompatible version (ours={}, theirs={})", version::CODE, msg.version),
                None)
            );
            self.channel.close(CloseType::IncompatibleVersion);
            return;
        }

        // Check if the peer is working on the same genesis block.
        let network_info = get_network_info(self.blockchain.network_id).unwrap();
        if network_info.genesis_hash != msg.genesis_hash {
            self.channel.close(CloseType::DifferentGenesisBlock);
            return;
        }

        // Check that the given peerAddress is correctly signed.
        if !msg.peer_address.verify_signature() {
            self.channel.close(CloseType::InvalidPeerAddressInVersionMessage);
            return;
        }

        // TODO Check services?

        // Check that the given peerAddress matches the one we expect.
        // In case of inbound WebSocket connections, this is the first time we
        // see the remote peer's peerAddress.
        let mut peer_address = msg.peer_address.clone();
        if let Some(channel_peer_address) = self.channel.address_info.peer_address() {
            if &peer_address != channel_peer_address.as_ref() {
                self.channel.close(CloseType::UnexpectedPeerAddressInVersionMessage);
                return;
            }
            self.peer_address_verified = true;
        }

        // The client might not send its netAddress. Set it from our address database if we have it.
        if peer_address.net_address.is_pseudo() {
            let addresses = self.addresses.state();
            let stored_address = addresses.get_info(&peer_address);
            if let Some(peer_address_info) = stored_address {
                if !peer_address_info.peer_address.net_address.is_pseudo() {
                    peer_address.net_address = peer_address_info.peer_address.net_address.clone();
                }
            }
        }

        let peer_address = Arc::new(peer_address);
        // Set/update the channel's peer address.
        self.channel.address_info.set_peer_address(peer_address.clone());

        // Create peer object. Since the initial version message received from the
        // peer contains their local timestamp, we can use it to calculate their
        // offset to our local timestamp and store it for later (last argument).
        self.peer = Some(Peer::new(
            self.channel.clone(),
            msg.version,
            msg.head_hash.clone(),
            peer_address.timestamp as i64 - systemtime_to_timestamp(now) as i64)
        );

        self.peer_challenge_nonce = Some(msg.challenge_nonce.clone());
        self.version_received = true;

        // Tell listeners that we received this peer's version information.
        // Listeners registered to this event might close the connection to this peer.
        self.notifier.notify(NetworkAgentEvent::Version(UniquePtr::new(self.peer.as_ref().unwrap())));

        // Abort handshake if the connection was closed.
        if self.channel.closed() {
            return;
        }

        if !self.version_sent {
            self.handshake();
            return;
        }

        if self.peer_address_verified {
            self.send_ver_ack();
        }

        if self.verack_received {
            self.finish_handshake();
        }
    }

    fn on_ver_ack(&mut self, msg: VerAckMessage) {
        debug!("[VERACK] from {}", self.channel.address_info.peer_address()
            .map_or("<unknown>".to_string(), |p| p.to_string()));

        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::VerAck) {
            return;
        }

        // Ignore duplicate VerAck messages.
        if self.verack_received {
            debug!("Ignoring duplicate VerAck message from {}", self.channel.address_info.peer_address()
                .map_or("<unknown>".to_string(), |p| p.to_string()));
            return;
        }

        // Clear the VerAck delay.
        self.timers.clear_delay(&NetworkAgentTimer::VerAck);

        // Verify public key.
        if &PeerId::from(&msg.public_key) != self.channel.address_info.peer_address().unwrap().peer_id() {
            self.channel.close(CloseType::InvalidPublicKeyInVerackMessage);
            return;
        }

        // Verify signature.
        let mut data = self.network_config.peer_id().serialize_to_vec();
        self.challenge_nonce.serialize(&mut data).unwrap();
        if !msg.public_key.verify(&msg.signature, &data[..]) {
            self.channel.close(CloseType::InvalidSignatureInVerackMessage);
            return;
        }

        if !self.peer_address_verified {
            self.peer_address_verified = true;
            self.send_ver_ack();
        }

        self.verack_received = true;

        if self.verack_sent {
            self.finish_handshake();
        }
    }

    fn finish_handshake(&mut self) {
        // Setup regular connectivity check.
        // TODO randomize interval?
        let weak = self.self_weak.clone();
        self.timers.set_interval(NetworkAgentTimer::Connectivity, move || {
            let arc = upgrade_weak!(weak);
            let mut agent = arc.write();
            agent.check_connectivity();
        }, Self::CONNECTIVITY_CHECK_INTERVAL);

        // Regularly announce our address.
        let weak = self.self_weak.clone();
        self.timers.set_interval(NetworkAgentTimer::AnnounceAddr, move || {
            let arc = upgrade_weak!(weak);
            let agent = arc.read();
            agent.channel.send_or_close(AddrMessage::new(vec![agent.network_config.peer_address()]));
        }, Self::ANNOUNCE_ADDR_INTERVAL);

        // Tell listeners that the handshake with this peer succeeded.
        self.notifier.notify(NetworkAgentEvent::Handshake(UniquePtr::new(self.peer.as_ref().unwrap())));

        // Request new network addresses from the peer.
        self.request_addresses(None);
    }

    pub fn request_addresses(&mut self, max_results: Option<u16>) {
        assert!(self.peer.is_some());
        debug!("Requesting addresses from {}", self.peer.as_ref().unwrap());

        let max_results = max_results.unwrap_or(Self::NUM_ADDR_PER_REQUEST);

        self.address_request = Some(AddressRequest {
           max_results
        });

        // Request addresses from peer.
        self.channel.send_or_close(GetAddrMessage::new(
            self.network_config.protocol_mask(),
            self.network_config.services().accepted,
            max_results));

        // We don't use a timeout here. The peer will not respond with an addr message if
        // it doesn't have any new addresses.
    }

    fn on_addr(&mut self, msg: AddrMessage) {
        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::Addr) {
            return;
        }

        // Reject unsolicited address messages unless it is the peer's own address.
        let peer_address = self.peer.as_ref().unwrap().peer_address();
        let is_own_address = msg.addresses.len() == 1 && peer_address.as_ref() == msg.addresses.get(0).unwrap();
        if self.address_request.is_none() && !is_own_address {
            return;
        }

        let address_request = self.address_request.take().unwrap_or_else(|| AddressRequest {
            max_results: Self::MAX_ADDR_PER_REQUEST,
        });

        // Reject messages that contain more than 1000 addresses, ban peer (bitcoin).
        if msg.addresses.len() > Self::MAX_ADDR_PER_MESSAGE as usize {
            warn!("Rejecting addr message - too many addresses");
            self.channel.close(CloseType::AddrMessageTooLarge);
            return;
        }

        debug!("[ADDR] {} addresses from {}", msg.addresses.len(), peer_address);

        // XXX Discard any addresses beyond the ones we requested
        // and check the addresses the peer sent to us.
        // TODO reject addr messages not matching our request.
        let addresses: Vec<PeerAddress> = msg.addresses.iter().take(address_request.max_results as usize).map(|p| p.clone()).collect();

        for address in addresses.iter() {
            if !address.verify_signature() {
                self.channel.close(CloseType::InvalidAddr);
                return;
            }

            // TODO Check globally reachable.
//            if (address.protocol() == Protocol::Ws || address.protocol() == Protocol::Wss) && address.globally_reachable() {
//                self.channel.close(CloseType::AddrNotGloballyReachable);
//                return;
//            }
        }

        // Update peer with new address.
        if is_own_address {
            self.channel.address_info.set_peer_address(Arc::new(addresses[0].clone()));
        }

        // Put the new addresses in the address pool.
        self.addresses.add(Some(self.channel.clone()), addresses);

        // Tell listeners that we have received new addresses.
        self.notifier.notify(NetworkAgentEvent::Addr);
    }

    fn on_get_addr(&mut self, msg: GetAddrMessage) {
        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::GetAddr) {
            return;
        }

        if !self.get_address_limit.note_single() {
            warn!("Rejecting GetAddr message - rate limit exceeded");
            return;
        }

        // Find addresses that match the given protocolMask & serviceMask.
        let num_results = cmp::min(msg.max_results, Self::MAX_ADDR_PER_REQUEST);
        let addresses = self.addresses.query(
            msg.protocol_mask,
            msg.service_mask,
            num_results
        );
        let addresses = addresses
            .iter().map(|peer_address| peer_address.as_ref().clone()).collect();
        self.channel.send_or_close(AddrMessage::new(addresses));
    }

    fn check_connectivity(&mut self) {
        // Generate random nonce.
        let mut cspring: OsRng = OsRng::new().unwrap();
        let nonce: u32 = cspring.gen();

        // Send ping message to peer.
        // If sending the ping message fails, assume the connection has died.
        if self.channel.send(Message::Ping(nonce)).is_err() {
            self.channel.close(CloseType::SendingPingMessageFailed);
            return;
        }

        // Save ping timestamp to detect the speed of the connection.
        let start_time = Instant::now();
        self.ping_times.insert(nonce, start_time.clone());

        // Expect the peer to answer with a pong message if we haven't heard anything from it
        // within the last CONNECTIVITY_CHECK_INTERVAL. Drop the peer otherwise.
        // TODO last_message_received missing
        if false {
            let weak = self.self_weak.clone();
            self.timers.set_delay(NetworkAgentTimer::Ping(nonce), move || {
                let arc = upgrade_weak!(weak);
                let mut agent = arc.write();
                agent.timers.clear_delay(&NetworkAgentTimer::Ping(nonce));
                agent.ping_times.remove(&nonce);
                agent.channel.close(CloseType::PingTimeout);
            }, Self::PING_TIMEOUT);
        }
    }

    fn on_ping(&self, nonce: u32) {
        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::Ping) {
            return;
        }

        // Respond with a pong message.
        self.channel.send_or_close(Message::Pong(nonce));
    }

    fn on_pong(&mut self, nonce: u32) {
        // Clear the ping delay for this nonce.
        self.timers.clear_delay(&NetworkAgentTimer::Ping(nonce));

        let start_time = self.ping_times.remove(&nonce);
        if let Some(start_time) = start_time {
            let delta = start_time.elapsed();
            self.notifier.notify(NetworkAgentEvent::PingPong(delta));
        }
    }

    fn on_close(&mut self) {
        // Clear all timers and intervals when the peer disconnects.
        self.timers.clear_all();
    }

    fn can_accept_message(&self, ty: MessageType) -> bool {
        // The first message must be the version message.
        if !self.version_received && ty != MessageType::Version {
            warn!("Discarding {:?} message from {:?} / {:?} - no version message received previously", ty, self.channel.address_info.peer_address(), self.channel.address_info.net_address());
            return false;
        }
        if self.version_received && !self.verack_received && ty != MessageType::VerAck {
            warn!("Discarding {:?} message from {:?} / {:?} - no verack message received previously", ty, self.channel.address_info.peer_address(), self.channel.address_info.net_address());
            return false;
        }

        return true;
    }
}

struct AddressRequest {
    max_results: u16,
}
