use std::borrow::BorrowMut;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::task::{Context, Poll};
use futures::{ready, Future, Stream, StreamExt};
use tokio::sync::{broadcast, mpsc};

use block_albatross::{Block, BlockType, ViewChangeProof};
use blockchain_albatross::{BlockchainEvent, ForkEvent};
use consensus_albatross::{Consensus, ConsensusEvent};
use hash::Blake2bHash;
use network_interface::network::Network;

use crate::slash::ForkProofPool;
use crate::validator2::micro::{ProduceMicroBlock, ProduceMicroBlockEvent};
use crate::validator2::mock::notifier_to_stream;
use crate::validator2::r#macro::ProduceMacroBlock;

enum ValidatorStakingState {
    Active,
    Parked,
    Inactive,
    NoStake,
}

struct ActiveEpochState {
    validator_id: u16,
}

struct BlockchainState {
    fork_proofs: ForkProofPool,
}

struct ProduceMicroBlockState {
    view_number: u32,
    view_change_proof: Option<ViewChangeProof>,
}

struct Validator<TNetwork: Network> {
    consensus: Arc<Consensus<TNetwork>>,
    signing_key: bls::KeyPair,
    wallet_key: Option<keys::KeyPair>,

    consensus_event_rx: broadcast::Receiver<ConsensusEvent<TNetwork>>,
    blockchain_event_rx: mpsc::UnboundedReceiver<BlockchainEvent>,
    fork_event_rx: mpsc::UnboundedReceiver<ForkEvent>,

    epoch_state: Option<ActiveEpochState>,
    blockchain_state: BlockchainState,

    macro_producer: Option<ProduceMacroBlock>,

    micro_producer: Option<ProduceMicroBlock>,
    micro_state: ProduceMicroBlockState,
}

impl<TNetwork: Network> Validator<TNetwork> {
    const VIEW_CHANGE_DELAY: Duration = Duration::from_secs(10);
    const FORK_PROOFS_MAX_SIZE: usize = 1_000; // bytes

    pub fn new(
        consensus: Arc<Consensus<TNetwork>>,
        signing_key: bls::KeyPair,
        wallet_key: Option<keys::KeyPair>,
    ) -> Self {
        let consensus_event_rx = consensus.subscribe_events();
        let blockchain_event_rx =
            notifier_to_stream(consensus.blockchain.notifier.write().borrow_mut());
        let fork_event_rx =
            notifier_to_stream(consensus.blockchain.fork_notifier.write().borrow_mut());

        let blockchain_state = BlockchainState {
            fork_proofs: ForkProofPool::new(),
        };

        let micro_state = ProduceMicroBlockState {
            view_number: consensus.blockchain.view_number(),
            view_change_proof: None,
        };

        let mut this = Self {
            consensus,
            signing_key,
            wallet_key,

            consensus_event_rx,
            blockchain_event_rx,
            fork_event_rx,

            epoch_state: None,
            blockchain_state,

            macro_producer: None,

            micro_producer: None,
            micro_state,
        };
        this.init();
        this
    }

    fn init(&mut self) {
        self.init_epoch();
        self.init_block_producer();
    }

    fn init_epoch(&mut self) {
        self.epoch_state = self
            .consensus
            .blockchain
            .current_validators()
            .find_idx_and_num_slots_by_public_key(&self.signing_key.public_key.compress())
            .map(|(validator_id, _)| ActiveEpochState { validator_id });
    }

    fn init_block_producer(&mut self) {
        self.macro_producer = None;
        self.micro_producer = None;

        if !self.is_active() {
            return;
        }

        let _lock = self.consensus.blockchain.lock();
        match self.consensus.blockchain.get_next_block_type(None) {
            BlockType::Macro => {
                // TODO
            }
            BlockType::Micro => {
                self.micro_state = ProduceMicroBlockState {
                    view_number: self.consensus.blockchain.view_number(),
                    view_change_proof: None,
                };

                let fork_proofs = self
                    .blockchain_state
                    .fork_proofs
                    .get_fork_proofs_for_block(Self::FORK_PROOFS_MAX_SIZE);
                self.micro_producer = Some(ProduceMicroBlock::new(
                    Arc::clone(&self.consensus.blockchain),
                    Arc::clone(&self.consensus.mempool),
                    self.signing_key.clone(),
                    self.validator_id(),
                    fork_proofs,
                    self.micro_state.view_number,
                    self.micro_state.view_change_proof.clone(),
                    Self::VIEW_CHANGE_DELAY,
                ));
            }
        }
    }

    fn on_blockchain_event(&mut self, event: BlockchainEvent) {
        match event {
            BlockchainEvent::Extended(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::Finalized(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::EpochFinalized(ref hash) => {
                self.on_blockchain_extended(hash);
                self.init_epoch()
            }
            BlockchainEvent::Rebranched(ref old_chain, ref new_chain) => {
                self.on_blockchain_rebranched(old_chain, new_chain)
            }
        }

        self.init_block_producer();
    }

    fn on_blockchain_extended(&mut self, hash: &Blake2bHash) {
        let block = self
            .consensus
            .blockchain
            .get_block(hash, true)
            .expect("Head block not found");
        self.blockchain_state.fork_proofs.apply_block(&block);
    }

    fn on_blockchain_rebranched(
        &mut self,
        old_chain: &[(Blake2bHash, Block)],
        new_chain: &[(Blake2bHash, Block)],
    ) {
        for (_hash, block) in old_chain.iter() {
            self.blockchain_state.fork_proofs.revert_block(block);
        }
        for (_hash, block) in new_chain.iter() {
            self.blockchain_state.fork_proofs.apply_block(&block);
        }
    }

    fn on_fork_event(&mut self, event: ForkEvent) {
        match event {
            ForkEvent::Detected(fork_proof) => self.blockchain_state.fork_proofs.insert(fork_proof),
        };
    }

    fn poll_macro(&mut self, cx: &mut Context<'_>) {
        // TODO
    }

    fn poll_micro(&mut self, cx: &mut Context<'_>) {
        let micro_producer = self.micro_producer.as_mut().unwrap();
        while let Poll::Ready(Some(event)) = micro_producer.poll_next_unpin(cx) {
            match event {
                ProduceMicroBlockEvent::MicroBlock(block) => {
                    self.consensus.blockchain.push(Block::Micro(block));
                }
                ProduceMicroBlockEvent::ViewChange(new_view_number, view_change_proof) => {
                    self.micro_state.view_number = new_view_number;
                    self.micro_state.view_change_proof = Some(view_change_proof);
                }
            }
        }
    }

    fn is_active(&self) -> bool {
        self.epoch_state.is_some()
    }

    fn validator_id(&self) -> u16 {
        self.epoch_state
            .as_ref()
            .expect("Validator not active")
            .validator_id
    }
}

impl<TNetwork: Network> Future for Validator<TNetwork> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Process consensus updates.
        while let Poll::Ready(Some(event)) = self.consensus_event_rx.poll_next_unpin(cx) {
            match event {
                Ok(ConsensusEvent::Established) => self.init(),
                Err(_) => return Poll::Ready(()),
                _ => {}
            }
        }

        // Process blockchain updates.
        while let Poll::Ready(Some(event)) = self.blockchain_event_rx.poll_next_unpin(cx) {
            if self.consensus.established() {
                self.on_blockchain_event(event);
            }
        }

        // Process fork events.
        while let Poll::Ready(Some(event)) = self.fork_event_rx.poll_next_unpin(cx) {
            if self.consensus.established() {
                self.on_fork_event(event);
            }
        }

        // If we are an active validator, participate in block production.
        if self.consensus.established() && self.is_active() {
            if self.macro_producer.is_some() {
                self.poll_macro(cx);
            }
            if self.micro_producer.is_some() {
                self.poll_micro(cx);
            }
        }

        Poll::Pending
    }
}
