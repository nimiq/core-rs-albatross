use std::cmp;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::time::{Duration, Instant, SystemTime};

use parking_lot::RwLock;
use rand::{OsRng, Rng};

use beserial::Serialize;

use crate::consensus::base::blockchain::Blockchain;
use crate::consensus::networks::get_network_info;
use crate::network::Peer;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::address::PeerId;
use crate::network::connection::close_type::CloseType;
use crate::network::message::*;
use crate::network::message::MessageType;
use crate::network::message::RejectMessage;
use crate::network::network_config::NetworkConfig;
use crate::network::peer_channel::{Agent, PeerChannel, PeerChannelEvent};
use crate::utils::observer::ListenerHandle;
use crate::utils::observer::Notifier;
use crate::utils::observer::weak_listener;
use crate::utils::systemtime_to_timestamp;
use crate::utils::timers::Timers;
use crate::utils::unique_ptr::UniquePtr;
use crate::utils::version;

pub struct NetworkAgent {
    blockchain: Arc<Blockchain<'static>>,
    addresses: Arc<RwLock<PeerAddressBook>>,
    network_config: Arc<NetworkConfig>,
    channel: PeerChannel,

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

    challenge_nonce: ChallengeNonce,

    listener: Weak<RwLock<NetworkAgent>>,
    listener_handle: Option<ListenerHandle>,

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
    const PING_TIMEOUT: Duration = Duration::from_secs(10); // 10 seconds
    const CONNECTIVITY_CHECK_INTERVAL: Duration = Duration::from_secs(60); // 1 minute
    const ANNOUNCE_ADDR_INTERVAL: Duration = Duration::from_secs(60 * 10); // 10 minutes
    const VERSION_RETRY_DELAY: Duration = Duration::from_millis(500); // 500 ms
    const GETADDR_RATE_LIMIT: usize = 3; // per minute
    const MAX_ADDR_PER_MESSAGE: u16 = 1000;
    const MAX_ADDR_PER_REQUEST: u16 = 500;
    const NUM_ADDR_PER_REQUEST: u16 = 200;

    pub fn new(blockchain: Arc<Blockchain<'static>>, addresses: Arc<RwLock<PeerAddressBook>>, network_config: Arc<NetworkConfig>, channel: PeerChannel) -> Arc<RwLock<Self>> {
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

            challenge_nonce: ChallengeNonce::generate(),
            
            listener: Weak::new(),
            listener_handle: None,
            notifier: Notifier::new(),
            
            timers: Timers::new(),
        }));
        agent.write().listener = Arc::downgrade(&agent);
        agent.write().initialize();
        agent
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
        let msg = VersionMessage::new(self.network_config.peer_address(), self.blockchain.head_hash(), network_info.genesis_hash.clone(), self.challenge_nonce.clone());
        if self.channel.send(msg).is_err() {
            self.version_attempts += 1;
            if self.version_attempts >= NetworkAgent::VERSION_ATTEMPTS_MAX || self.channel.closed() {
                self.channel.close(CloseType::SendingOfVersionMessageFailed);
                return;
            }

            let weak = self.listener.clone();
            self.timers.reset_delay(NetworkAgentTimer::Handshake, move || {
                let arc = upgrade_weak!(weak);
                arc.write().handshake();
            }, Instant::now() + NetworkAgent::VERSION_RETRY_DELAY);
            return;
        }

        self.version_sent = true;

        // Drop the peer if it doesn't send us a version message.
        // Only do this if we haven't received the peer's version message already.
        if !self.version_received {
            // TODO Should we ban instead?
            let weak = self.listener.clone();
            self.timers.set_delay(NetworkAgentTimer::Version, move || {
                let arc = upgrade_weak!(weak);
                let agent = arc.read();
                agent.timers.clear_delay(&NetworkAgentTimer::Version);
                agent.channel.close(CloseType::VersionTimeout);
            }, Instant::now() + NetworkAgent::HANDSHAKE_TIMEOUT);
        } else if self.peer_address_verified {
            self.send_ver_ack();
        }

        let weak = self.listener.clone();
        self.timers.set_delay(NetworkAgentTimer::VerAck, move || {
            let arc = upgrade_weak!(weak);
            let agent = arc.read();
            agent.timers.clear_delay(&NetworkAgentTimer::VerAck);
            agent.channel.close(CloseType::VerackTimeout);
        }, Instant::now() + NetworkAgent::HANDSHAKE_TIMEOUT);
    }

    fn send_ver_ack(&mut self) {
        assert!(self.peer_address_verified);
        assert!(self.peer_challenge_nonce.is_some());

        // TODO Handle Err case?
        println!("PeerId: {:?}", self.network_config.peer_id());
        self.channel.send(VerAckMessage::new(self.network_config.peer_id(), self.peer_challenge_nonce.as_ref().unwrap(), self.network_config.key_pair())).unwrap();

        self.verack_sent = true;
    }

    fn on_version(&mut self, msg: &VersionMessage) {
        debug!("[VERSION] {:?} {:?}", &msg.peer_address, &msg.head_hash);

        let now = SystemTime::now();

        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::Version) {
            return;
        }

        // Ignore duplicate version messages.
        if self.version_received {
            debug!("Ignoring duplicate version message from {:?}", self.channel.address_info.peer_address());
            return;
        }

        // Clear the version timeout.
        self.timers.clear_delay(&NetworkAgentTimer::Version);

        // Check if the peer is running a compatible version.
        if !version::is_compatible(msg.version) {
            self.channel.send(RejectMessage::new(
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
            // TODO Very complicated API call here.
            let addresses = self.addresses.read();
            let stored_address = addresses.get_info(&Arc::new(peer_address.clone()));
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

    fn finish_handshake(&mut self) {
        // Setup regular connectivity check.
        // TODO randomize interval?
        let weak = self.listener.clone();
        self.timers.set_interval(NetworkAgentTimer::Connectivity, move || {
            let arc = upgrade_weak!(weak);
            let mut agent = arc.write();
            agent.check_connectivity();
        }, NetworkAgent::CONNECTIVITY_CHECK_INTERVAL);

        // Regularly announce our address.
        let weak = self.listener.clone();
        self.timers.set_interval(NetworkAgentTimer::AnnounceAddr, move || {
            let arc = upgrade_weak!(weak);
            let agent = arc.read();
            agent.channel.send(AddrMessage::new(vec![agent.network_config.peer_address()]));
        }, NetworkAgent::ANNOUNCE_ADDR_INTERVAL);

        // Tell listeners that the handshake with this peer succeeded.
        self.notifier.notify(NetworkAgentEvent::Handshake(UniquePtr::new(self.peer.as_ref().unwrap())));

        // Request new network addresses from the peer.
        self.request_addresses(None);
    }

    fn request_addresses(&mut self, max_results: Option<u16>) {
        assert!(self.peer.is_some());
        debug!("Requesting addresses from {:?}", self.peer.as_ref().unwrap());

        let max_results = max_results.unwrap_or(NetworkAgent::NUM_ADDR_PER_REQUEST);

        self.address_request = Some(AddressRequest {
           max_results
        });

        // Request addresses from peer.
        self.channel.send(GetAddrMessage::new(
            self.network_config.protocol_mask(),
            self.network_config.services().accepted,
            max_results));

        // We don't use a timeout here. The peer will not respond with an addr message if
        // it doesn't have any new addresses.
    }

    fn check_connectivity(&mut self) {
        // Generate random nonce.
        let mut cspring: OsRng = OsRng::new().unwrap();
        let nonce = cspring.next_u32();

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
            let weak = self.listener.clone();
            self.timers.set_delay(NetworkAgentTimer::Ping(nonce), move || {
                let arc = upgrade_weak!(weak);
                let mut agent = arc.write();
                agent.timers.clear_delay(&NetworkAgentTimer::Ping(nonce));
                agent.ping_times.remove(&nonce);
                agent.channel.close(CloseType::PingTimeout);
            }, Instant::now() + NetworkAgent::PING_TIMEOUT);
        }
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

    fn on_ver_ack(&mut self, msg: &VerAckMessage) {
        debug!("[VERACK] from {:?}", self.channel.address_info.peer_address());

        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::VerAck) {
            return;
        }

        // Ignore duplicate VerAck messages.
        if self.verack_received {
            debug!("Ignoring duplicate VerAck message from {:?}", self.channel.address_info.peer_address());
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
        self.challenge_nonce.serialize(&mut data);
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

    fn on_addr(&mut self, msg: &AddrMessage) {
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
            max_results: NetworkAgent::MAX_ADDR_PER_REQUEST,
        });

        // Reject messages that contain more than 1000 addresses, ban peer (bitcoin).
        if msg.addresses.len() > NetworkAgent::MAX_ADDR_PER_MESSAGE as usize {
            warn!("Rejecting addr message - too many addresses");
            self.channel.close(CloseType::AddrMessageTooLarge);
            return;
        }

        debug!("[ADDR] {} addresses from {:?}", msg.addresses.len(), peer_address);

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
        self.addresses.write().add(Some(&self.channel), addresses);

        // Tell listeners that we have received new addresses.
        self.notifier.notify(NetworkAgentEvent::Addr);
    }

    fn on_get_addr(&mut self, msg: &GetAddrMessage) {
        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::GetAddr) {
            return;
        }

        // TODO Rate limiting

        // Find addresses that match the given protocolMask & serviceMask.
        let num_results = cmp::min(msg.max_results, NetworkAgent::MAX_ADDR_PER_REQUEST);
        let addresses = self.addresses.read().query(
            msg.protocol_mask,
            msg.service_mask,
            num_results
        );
        let addresses = addresses
            .iter().map(|peer_address| peer_address.as_ref().clone()).collect();
        self.channel.send(AddrMessage::new(addresses));
    }

    fn on_ping(&self, nonce: u32) {
        // Make sure this is a valid message in our current state.
        if !self.can_accept_message(MessageType::Ping) {
            return;
        }

        // Respond with a pong message.
        self.channel.send(Message::Pong(nonce));
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
}

impl Agent for NetworkAgent {
    fn initialize(&mut self) {
        self.listener_handle = Some(self.channel.notifier.write().register(weak_listener(self.listener.clone(), move |arc, event| {
            match *event {
                PeerChannelEvent::Message(ref msg) => {
                    arc.write().on_message(msg);
                },
                PeerChannelEvent::Close(_) => {
                    arc.write().on_close();
                },
                PeerChannelEvent::Error(_) => {},
            }
        })));
    }

    fn on_message(&mut self, msg: &Message) {
        match *msg {
            Message::Version(ref version_msg) => self.on_version(version_msg),
            Message::VerAck(ref ver_ack_msg) => self.on_ver_ack(ver_ack_msg),
            Message::Addr(ref addr_msg) => self.on_addr(addr_msg),
            Message::GetAddr(ref get_addr_msg) => self.on_get_addr(get_addr_msg),
            Message::Ping(nonce) => self.on_ping(nonce),
            Message::Pong(nonce) => self.on_pong(nonce),
            _ => {},
        }
    }

    fn on_close(&mut self) {
        // Clear all timers and intervals when the peer disconnects.
        self.timers.clear_all();
        // Deregister from events.
        if let Some(listener_handle) = self.listener_handle.take() {
            self.channel.notifier.write().deregister(listener_handle);
        }
    }
}

struct AddressRequest {
    max_results: u16,
}
