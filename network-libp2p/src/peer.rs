use std::hash::{Hash, Hasher};

use async_trait::async_trait;
use libp2p::PeerId;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use nimiq_network_interface::peer::{CloseReason, Peer as PeerInterface};

use crate::NetworkError;

pub struct Peer {
    pub id: PeerId,

    /// Channel used to pass the close reason the the network handler.
    close_tx: Mutex<Option<oneshot::Sender<CloseReason>>>,
}

impl Peer {
    pub fn new(id: PeerId, close_tx: oneshot::Sender<CloseReason>) -> Self {
        Self {
            id,
            close_tx: Mutex::new(Some(close_tx)),
        }
    }
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut debug = f.debug_struct("Peer");

        debug.field("peer_id", &self.id());

        if self.close_tx.lock().is_none() {
            debug.field("closed", &true);
        }

        debug.finish()
    }
}

impl PartialEq for Peer {
    fn eq(&self, other: &Peer) -> bool {
        self.id == other.id
    }
}

impl Eq for Peer {}

impl Hash for Peer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

#[async_trait]
impl PeerInterface for Peer {
    type Id = PeerId;
    type Error = NetworkError;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn close(&self, reason: CloseReason) {
        // TODO: I think we must poll_close on the underlying socket

        log::debug!("Peer::close: reason={:?}", reason);

        let close_tx_opt = self.close_tx.lock().take();

        if let Some(close_tx) = close_tx_opt {
            if close_tx.send(reason).is_err() {
                log::error!("The receiver for Peer::close was already dropped.");
            }
        } else {
            log::error!("Peer is already closed");
        }
    }
}
