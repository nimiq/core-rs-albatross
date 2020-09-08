use std::sync::Arc;

use blockchain_albatross::Blockchain;
use consensus::{Consensus, ConsensusEvent};
use json::JsonValue;
use network::Network;
use parking_lot::RwLock;

use crate::handler::Method;
use crate::handlers::Module;

pub struct ConsensusHandler {
    pub consensus: Arc<Consensus<Network>>,
    state: Arc<RwLock<ConsensusHandlerState>>,
}

pub struct ConsensusHandlerState {
    consensus: &'static str,
}

impl ConsensusHandler {
    pub fn new(consensus: Arc<Consensus<Network>>) -> Self {
        let state = ConsensusHandlerState {
            consensus: "syncing",
        };
        let state = Arc::new(RwLock::new(state));
        let this = Self {
            consensus: Arc::clone(&consensus),
            state: Arc::clone(&state),
        };

        // Register for consensus events.
        // TODO: Right now this uses the Consensus struct from the consensus crate. That crate was
        //       deleted. Change this part to use the Consensus struct from the consensus-albatross
        //       crate.
        // {
        //     trace!("Register listener for consensus");
        //     let state = Arc::downgrade(&state);
        //     consensus
        //         .notifier
        //         .write()
        //         .register(move |e: &ConsensusEvent<Network<Blockchain>>| {
        //             if let Some(state) = state.upgrade() {
        //                 match e {
        //                     ConsensusEvent::Established => state.write().consensus = "established",
        //                     ConsensusEvent::Lost => state.write().consensus = "lost",
        //                     ConsensusEvent::Syncing => state.write().consensus = "syncing",
        //                     _ => (),
        //                 }
        //             } else {
        //                 // TODO Remove listener
        //             }
        //         });
        // }

        this
    }

    fn consensus(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        Ok(self.state.read().consensus.into())
    }
}

impl Module for ConsensusHandler {
    rpc_module_methods! {
        "consensus" => consensus,
    }
}
