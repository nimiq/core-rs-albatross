use std::sync::Arc;

use json::JsonValue;
use parking_lot::RwLock;

use consensus::{Consensus, ConsensusEvent, ConsensusProtocol};

use crate::handler::Method;
use crate::handlers::Module;

pub struct ConsensusHandler<P>
where
    P: ConsensusProtocol + 'static,
{
    pub consensus: Arc<Consensus<P>>,
    state: Arc<RwLock<ConsensusHandlerState>>,
}

pub struct ConsensusHandlerState {
    consensus: &'static str,
}

impl<P> ConsensusHandler<P>
where
    P: ConsensusProtocol + 'static,
{
    pub fn new(consensus: Arc<Consensus<P>>) -> Self {
        let state = ConsensusHandlerState {
            consensus: "syncing",
        };
        let state = Arc::new(RwLock::new(state));
        let this = Self {
            consensus: Arc::clone(&consensus),
            state: Arc::clone(&state),
        };

        // Register for consensus events.
        {
            trace!("Register listener for consensus");
            let state = Arc::downgrade(&state);
            consensus
                .notifier
                .write()
                .register(move |e: &ConsensusEvent| {
                    trace!("Consensus Event: {:?}", e);
                    if let Some(state) = state.upgrade() {
                        match e {
                            ConsensusEvent::Established => state.write().consensus = "established",
                            ConsensusEvent::Lost => state.write().consensus = "lost",
                            ConsensusEvent::Syncing => state.write().consensus = "syncing",
                            _ => (),
                        }
                    } else {
                        // TODO Remove listener
                    }
                });
        }

        this
    }

    fn consensus(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        Ok(self.state.read().consensus.into())
    }
}

impl<P: ConsensusProtocol + 'static> Module for ConsensusHandler<P> {
    rpc_module_methods! {
        "consensus" => consensus,
    }
}
