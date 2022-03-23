use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use async_trait::async_trait;
use parking_lot::Mutex;

use nimiq_network_interface::peer::{CloseReason, Peer};

use crate::{hub::MockHubInner, network::MockNetworkError, MockAddress, MockPeerId};

#[derive(Clone, Debug)]
pub struct MockPeer {
    /// The address of the network that sees this peer
    pub(crate) network_address: MockAddress,

    /// The peer's peer ID
    pub(crate) peer_id: MockPeerId,

    pub(crate) hub: Arc<Mutex<MockHubInner>>,
}

#[async_trait]
impl Peer for MockPeer {
    type Id = MockPeerId;
    type Error = MockNetworkError;

    fn id(&self) -> MockPeerId {
        self.peer_id
    }

    fn close(&self, _ty: CloseReason) {
        let mut hub = self.hub.lock();

        // Drops senders and thus the receiver stream will end
        hub.network_senders
            .retain(|k, _sender| k.network_recipient != self.network_address);
    }
}

impl PartialEq for MockPeer {
    fn eq(&self, other: &Self) -> bool {
        self.peer_id == other.peer_id
    }
}

impl Eq for MockPeer {}

impl Hash for MockPeer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.peer_id.hash(state);
    }
}
