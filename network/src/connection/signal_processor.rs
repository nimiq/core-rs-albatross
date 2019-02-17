use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use collections::UniqueLinkedList;
use network_messages::{Message, SignalMessage, SignalMessageFlags};
use network_primitives::address::PeerId;

use crate::address::peer_address_book::PeerAddressBook;
use crate::network_config::NetworkConfig;
use crate::peer_channel::PeerChannel;

use super::super::Network;
use super::close_type::CloseType;

pub struct SignalProcessor {
    addresses: Arc<PeerAddressBook>,
    network_config: Arc<NetworkConfig>,
    forwards: Mutex<SignalStore>,
}

impl SignalProcessor {
    const SIGNAL_STORE_MAX_SIZE: usize = 1000;

    pub fn new(addresses: Arc<PeerAddressBook>, network_config: Arc<NetworkConfig>) -> Self {
        Self {
            addresses,
            network_config,
            forwards: Mutex::new(SignalStore::new(Self::SIGNAL_STORE_MAX_SIZE)),
        }
    }

    pub fn on_signal(&self, channel: Arc<PeerChannel>, msg: SignalMessage) {
        // Discard signals with invalid TTL.
        if msg.ttl > Network::SIGNAL_TTL_INITIAL {
            channel.close(CloseType::InvalidSignalTtl);
            return;
        }

        // Discard signals from myself.
        // Note: this should never happen because we don't have WebRTC support in the Rust implementation yet
        let my_peer_id = self.network_config.peer_id();
        if msg.sender_id == *my_peer_id {
            warn!("Received signal from myself to {:?} from {:?} (myId: {:?})", &msg.recipient_id, &channel.address_info.peer_address(), my_peer_id);
            return;
        }

        // If the signal has the unroutable flag set and we previously forwarded a matching signal,
        // mark the route as unusable.
        if msg.flags.contains(SignalMessageFlags::UNROUTABLE) && self.forwards.lock().signal_forwarded(
            msg.recipient_id.clone(), /* sender_id */
            msg.sender_id.clone(), /* recipient_id */
            msg.nonce /* nonce */
        ) {
            if let Some(sender_address) = self.addresses.state().get_by_peer_id(&msg.sender_id) {
                self.addresses.unroutable(channel.clone(), sender_address);
            }
        }

        // If the signal is intended for us, pass it on to our WebRTC connector.
        // Note: this also should never happen because we don't have WebRTC support in the Rust implementation yet
        if msg.recipient_id == *my_peer_id {
            warn!("Received signal from the id {:?} for myself through the channel {:?}", &msg.recipient_id, &channel.address_info.peer_address());
            return;
        }

        // Discard signals that have reached their TTL.
        if msg.ttl == 0 {
            debug!("Discarding signal from {:?} to {:?} - TTL reached", &msg.sender_id, &msg.recipient_id);
            // Send signal containing TTL_EXCEEDED flag back in reverse direction.
            if msg.flags.is_empty() {
                channel.send_or_close(Message::Signal(SignalMessage {
                    sender_id: msg.recipient_id,
                    recipient_id: msg.sender_id,
                    nonce: msg.nonce,
                    ttl: Network::SIGNAL_TTL_INITIAL,
                    flags: SignalMessageFlags::TTL_EXCEEDED,
                    payload: Vec::new(),
                    sender_public_key: None,
                    signature: None,
                }));
            }
            return;
        }

        // Otherwise, try to forward the signal to the intended recipient.
        let peer_address_book_state = self.addresses.state();
        let signal_channel = peer_address_book_state.get_channel_by_peer_id(&msg.recipient_id);
        if signal_channel.is_none() {
            debug!("Failed to forward signal from {:?} to {:?} - no route found", msg.sender_id, msg.recipient_id);
            // If we don't know a route to the intended recipient, return signal to sender with unroutable flag set and payload removed.
            // Only do this if the signal is not already a unroutable response.
            if msg.flags.is_empty() {
                channel.send_or_close(Message::Signal(SignalMessage {
                    sender_id: msg.recipient_id,
                    recipient_id: msg.sender_id,
                    nonce: msg.nonce,
                    ttl: Network::SIGNAL_TTL_INITIAL,
                    flags: SignalMessageFlags::UNROUTABLE,
                    payload: Vec::new(),
                    sender_public_key: None,
                    signature: None,
                }));
            }
            return;
        }

        // Discard signal if our shortest route to the target is via the sending peer.
        // XXX Why does this happen?
        let signal_channel = signal_channel.expect("This should always work because we dealed with the None case above");
        if signal_channel.address_info.peer_address() == channel.address_info.peer_address() {
            debug!("Discarding signal from {:?} to {:?} - shortest route via sending peer", msg.sender_id, msg.recipient_id);
            // If our best route is via the sending peer, return signal to sender with unroutable flag set and payload removed.
            // Only do this if the signal is not already a unroutable response.
            if msg.flags.is_empty() {
                channel.send_or_close(Message::Signal(SignalMessage {
                    sender_id: msg.recipient_id,
                    recipient_id: msg.sender_id,
                    nonce: msg.nonce,
                    ttl: Network::SIGNAL_TTL_INITIAL,
                    flags: SignalMessageFlags::UNROUTABLE,
                    payload: Vec::new(),
                    sender_public_key: None,
                    signature: None,
                }));
            }
            return;
        }

        // Decrement ttl and forward signal.
        signal_channel.send_or_close(Message::Signal(SignalMessage {
            sender_id: msg.sender_id.clone(),
            recipient_id: msg.recipient_id.clone(),
            nonce: msg.nonce,
            ttl: msg.ttl - 1,
            flags: msg.flags,
            payload: msg.payload,
            sender_public_key: msg.sender_public_key,
            signature: msg.signature,
        }));

        debug!("Forwarded signal to {:?} from {:?}", &msg.recipient_id, &msg.sender_id);

        // We store forwarded messages if there are no special flags set.
        if msg.flags.is_empty() {
            self.forwards.lock().add(msg.sender_id, msg.recipient_id, msg.nonce);
        }
    }
}

pub struct SignalStore {
    /// maximum number of entries
    max_size: usize,
    queue: UniqueLinkedList<ForwardedSignal>,
    store: HashMap<ForwardedSignal, Instant>,
}

impl SignalStore {
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            queue: UniqueLinkedList::new(),
            store: HashMap::new(),
        }
    }

    pub fn add(&mut self, sender_id: PeerId, recipient_id: PeerId, nonce: u32) {
        let signal = ForwardedSignal::new(sender_id, recipient_id, nonce);

        // If we already forwarded such a message, just update timestamp.
        if self.store.contains_key(&signal) {
            self.store.insert(signal.clone(), Instant::now());
            // requeue
            self.queue.remove(&signal);
            self.queue.push_front(signal);
            return
        }

        // Delete oldest if needed.
        if self.queue.len() >= self.max_size {
            if let Some(oldest) = self.queue.pop_back() {
                self.store.remove(&oldest);
            }
        }
        self.queue.push_front(signal.clone());
        self.store.insert(signal, Instant::now());
    }

    pub fn signal_forwarded(&mut self, sender_id: PeerId, recipient_id: PeerId, nonce: u32) -> bool {
        let signal = ForwardedSignal::new(sender_id, recipient_id, nonce);
        if let Some(last_seen) = self.store.get(&signal) {
            let valid = last_seen.elapsed() < ForwardedSignal::SIGNAL_MAX_AGE;
            if !valid {
                // Because of the ordering, we know that everything after that is invalid too.
                for _ in 0..self.queue.len() {
                    let signal_to_remove = self.queue.pop_back().expect("The check above guarantees this is Some()");
                    self.store.remove(&signal_to_remove);
                    if signal == signal_to_remove {
                        break;
                    }
                }
            }
            valid
        } else {
            false
        }
    }
}
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct ForwardedSignal {
    sender_id: PeerId,
    recipient_id: PeerId,
    nonce: u32,
}

impl ForwardedSignal {
    const SIGNAL_MAX_AGE: Duration = Duration::from_secs(10);

    pub fn new(sender_id: PeerId, recipient_id: PeerId, nonce: u32) -> Self  {
        Self {
            sender_id,
            recipient_id,
            nonce
        }
    }
}
