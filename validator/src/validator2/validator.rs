// Validator states
// - Potential
// - Active
// - Parked
// - Inactive

// Operation states
// - Produce micro block
//   - Propose block
//   - Wait for block / view change
// - Produce macro block
//   - Checkpoint block
//   - Election block

use std::borrow::BorrowMut;
use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc;
use futures::task::{Context, Poll};
use futures::{ready, Future, Stream, StreamExt};
use tokio::sync::broadcast;

use block_albatross::{Block, ViewChangeProof};
use blockchain_base::BlockchainEvent;
use consensus_albatross::{Consensus, ConsensusEvent};
use network_interface::network::Network;

use crate::validator2::micro::ProduceMicroBlock;
use crate::validator2::r#macro::ProduceMacroBlock;

enum ValidatorStakingState {
    Active,
    Parked,
    Inactive,
    NoStake,
}

struct ProduceMicroBlockState {
    view_number: u32,
    last_view_change_proof: Option<ViewChangeProof>,
}

struct ActiveEpochState {
    validator_id: u16,
}

struct Validator<TNetwork: Network> {
    consensus: Arc<Consensus<TNetwork>>,
    signing_key: bls::KeyPair,
    wallet_key: Option<keys::KeyPair>,

    consensus_event_rx: broadcast::Receiver<ConsensusEvent<TNetwork>>,
    blockchain_event_rx: mpsc::Receiver<BlockchainEvent<Block>>,

    epoch_state: Option<ActiveEpochState>,

    micro_producer: Option<ProduceMicroBlock>,
    macro_producer: Option<ProduceMacroBlock>,
}

impl<TNetwork: Network> Validator<TNetwork> {
    pub fn new(
        consensus: Arc<Consensus<TNetwork>>,
        signing_key: bls::KeyPair,
        wallet_key: Option<keys::KeyPair>,
    ) -> Self {
        let consensus_event_rx = consensus.subscribe_events();
        let blockchain_event_rx = crate::validator2::mock::notifier_to_stream(
            consensus.blockchain.notifier.write().borrow_mut(),
        );
        let mut this = Self {
            consensus,
            signing_key,
            wallet_key,

            consensus_event_rx,
            blockchain_event_rx,

            epoch_state: None,

            micro_producer: None,
            macro_producer: None,
        };
        this.init_epoch();
        this
    }

    fn init_epoch(&mut self) {
        self.epoch_state = self
            .consensus
            .blockchain
            .current_validators()
            .find_idx_and_num_slots_by_public_key(&self.signing_key.public_key.compress())
            .map(|(validator_id, _)| ActiveEpochState { validator_id });
    }

    fn on_blockchain_event(&mut self, event: BlockchainEvent<Block>) {
        match event {
            BlockchainEvent::Extended(_) | BlockchainEvent::Rebranched(_, _) => {
                self.micro_producer = None
            }
            BlockchainEvent::Finalized(_) => self.macro_producer = None,
            BlockchainEvent::EpochFinalized(_) => {
                self.macro_producer = None;
                self.init_epoch()
            }
        }
    }

    fn is_active(&self) -> bool {
        self.epoch_state.is_some()
    }
}

impl<TNetwork: Network> Future for Validator<TNetwork> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Wait for consensus to be established before doing anything else.
        if !self.consensus.established() {
            loop {
                match ready!(self.consensus_event_rx.poll_next_unpin(cx)) {
                    Some(Ok(ConsensusEvent::Established)) => break, // Continue with this function
                    Some(Err(_)) | None => return Poll::Ready(()),  // Consensus terminated
                    Some(_) => {}                                   // Ignore any other events
                }
            }
        }

        // Process blockchain updates.
        while let Poll::Ready(Some(event)) = self.blockchain_event_rx.poll_next_unpin(cx) {
            // Update validator state, i.e. check if we are still active, unpark, etc.
            self.on_blockchain_event(event);
        }

        // If we are an active validator, participate in block production.
        if self.is_active() {
            // Run micro or macro block production depending on next block type.
            //if self.consensus.blockchain.get_next_block_type()
        }

        Poll::Pending
    }
}
