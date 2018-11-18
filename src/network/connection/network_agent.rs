use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::time::SystemTime;

use parking_lot::RwLock;
use rand::OsRng;

use crate::consensus::base::blockchain::Blockchain;
use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::message::AddrMessage;
use crate::network::message::ChallengeNonce;
use crate::network::message::GetAddrMessage;
use crate::network::message::Message;
use crate::network::message::VerAckMessage;
use crate::network::message::VersionMessage;
use crate::network::network_config::NetworkConfig;
use crate::network::Peer;
use crate::network::peer_channel::{PeerChannel, PeerChannelEvent};
use crate::network::peer_channel::Agent;
use crate::utils::observer::ListenerHandle;
use crate::utils::observer::weak_listener;

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

    ping_times: HashMap<usize, SystemTime>,

    peer_challenge_nonce: Option<ChallengeNonce>,
    address_request: Option<AddressRequest>,

    challenge_nonce: ChallengeNonce,

    listener: Weak<RwLock<NetworkAgent>>,
    listener_handle: Option<ListenerHandle>,
}

impl NetworkAgent {
    const VERSION_ATTEMPTS_MAX: usize = 10;

    pub fn new(blockchain: Arc<Blockchain<'static>>, addresses: Arc<RwLock<PeerAddressBook>>, network_config: Arc<NetworkConfig>, channel: PeerChannel) -> Arc<RwLock<Self>> {
        let mut agent = Arc::new(RwLock::new(Self {
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
        let msg = VersionMessage::new(self.network_config.peer_address(), self.blockchain.head_hash(), self.challenge_nonce.clone());
        if self.channel.send(msg).is_err() {
            self.version_attempts += 1;
            if self.version_attempts >= NetworkAgent::VERSION_ATTEMPTS_MAX || self.channel.closed() {
                // TODO Close channel
                return;
            }

            // TODO Version retry timer
            return;
        }

        self.version_sent = true;

        // Drop the peer if it doesn't send us a version message.
        // Only do this if we haven't received the peer's version message already.
        if !self.version_received {
            // TODO Should we ban instead?
            // TODO Close timer
        } else if self.peer_address_verified {
            self.send_ver_ack();
        }
        // TODO Verack timer
    }

    fn send_ver_ack(&mut self) {
        assert!(self.peer_address_verified);
        assert!(self.peer_challenge_nonce.is_some());

        // TODO Handle Err case?
        self.channel.send(VerAckMessage::new(self.network_config.peer_id(), self.peer_challenge_nonce.as_ref().unwrap(), self.network_config.key_pair()));

        self.verack_sent = true;
    }

    fn on_version(&mut self, msg: &VersionMessage) {
        warn!("[VERSION] {:?} {:?}", &msg.peer_address, &msg.head_hash);
    }

    fn on_ver_ack(&mut self, msg: &VerAckMessage) {

    }

    fn on_addr(&mut self, msg: &AddrMessage) {

    }

    fn on_get_addr(&mut self, msg: &GetAddrMessage) {

    }

    fn on_ping(&mut self, nonce: u32) {

    }

    fn on_pong(&mut self, nonce: u32) {

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
                PeerChannelEvent::Error => {},
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
        unimplemented!()
    }
}

struct AddressRequest {
    max_results: usize,
}
