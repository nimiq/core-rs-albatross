use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use futures::{future::BoxFuture, ready, FutureExt, Stream};
use parking_lot::RwLock;
use tokio::time;

use nimiq_block::{Block, ForkProof, MicroBlock, ViewChange, ViewChangeProof};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, PushResult};
use nimiq_mempool::mempool::Mempool;
use nimiq_primitives::slots::Validators;
use nimiq_utils::time::systemtime_to_timestamp;
use nimiq_validator_network::ValidatorNetwork;
use nimiq_vrf::VrfSeed;

use crate::aggregation::view_change::ViewChangeAggregation;

// Ignoring this clippy warning since size difference is not that much (320
// bytes) and we probably don't want the performance penalty of the allocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock(MicroBlock, PushResult),
    ViewChange(ViewChange, ViewChangeProof),
}

#[derive(Clone)]
struct NextProduceMicroBlockEvent<TValidatorNetwork> {
    blockchain: Arc<RwLock<Blockchain>>,
    mempool: Arc<Mempool>,
    network: Arc<TValidatorNetwork>,
    block_producer: BlockProducer,
    validator_slot_band: u16,
    fork_proofs: Vec<ForkProof>,
    prev_seed: VrfSeed,
    block_number: u32,
    view_number: u32,
    view_change_proof: Option<ViewChangeProof>,
    view_change: Option<ViewChange>,
    view_change_delay: Duration,
    empty_block_delay: Duration,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> NextProduceMicroBlockEvent<TValidatorNetwork> {
    const CHECK_MEMPOOL_DELAY: Duration = Duration::from_millis(100);

    // Ignoring clippy warning because there wouldn't be much to be gained by refactoring this,
    // except making clippy happy
    #[allow(clippy::too_many_arguments)]
    fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        mempool: Arc<Mempool>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        fork_proofs: Vec<ForkProof>,
        prev_seed: VrfSeed,
        block_number: u32,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
        view_change: Option<ViewChange>,
        view_change_delay: Duration,
        empty_block_delay: Duration,
    ) -> Self {
        Self {
            blockchain,
            mempool,
            network,
            block_producer,
            validator_slot_band,
            fork_proofs,
            prev_seed,
            block_number,
            view_number,
            view_change_proof,
            view_change,
            view_change_delay,
            empty_block_delay,
        }
    }

    async fn next(
        mut self,
    ) -> (
        Option<ProduceMicroBlockEvent>,
        NextProduceMicroBlockEvent<TValidatorNetwork>,
    ) {
        let in_current_state = |head: &Block| {
            self.prev_seed == *head.seed()
                && self.block_number == head.block_number() + 1
                && self.view_number >= head.next_view_number()
        };

        // If there are no transactions to include, we want to wait a bit before producing the
        // micro block to reduce the number of unnecessary empty blocks. Set the deadline at which
        // we're going to produce the block even if it is empty.
        let deadline = SystemTime::now() + self.empty_block_delay;
        let mut delay = Duration::default();
        let mut logged = false;

        let return_value = loop {
            // Acquire blockchain.upgradable_read() to prevent further changes to the blockchain while
            // we're constructing the block. Check if we're still in the correct state, abort otherwise.
            {
                let blockchain = self.blockchain.upgradable_read();
                if !in_current_state(&blockchain.head()) {
                    break Some(None);
                } else if self.is_our_turn(&blockchain) {
                    if !logged {
                        info!(
                            block_number = self.block_number,
                            view_number = self.view_number,
                            slot_band = self.validator_slot_band,
                            "Our turn, producing micro block #{}.{}",
                            self.block_number,
                            self.view_number,
                        );
                        logged = true;
                    }

                    let block = self.produce_micro_block(&blockchain);
                    let num_transactions = block
                        .body
                        .as_ref()
                        .map(|body| body.transactions.len())
                        .unwrap_or(0);

                    // Publish block immediately if it contains transactions or the block production
                    // deadline has passed.
                    if num_transactions > 0 || SystemTime::now() >= deadline {
                        debug!(
                            block_number = block.header.block_number,
                            view_number = block.header.view_number,
                            num_transactions,
                            ?delay,
                            "Produced micro block {} with {} transactions",
                            block,
                            num_transactions
                        );

                        let block1 = block.clone();

                        // Use a trusted push since these blocks were generated by this validator
                        let result = if cfg!(feature = "trusted_push") {
                            Blockchain::trusted_push(blockchain, Block::Micro(block))
                        } else {
                            Blockchain::push(blockchain, Block::Micro(block))
                        };

                        if let Err(e) = &result {
                            error!("Failed to push our own block onto the chain: {:?}", e);
                        }

                        let event = result
                            .map(move |result| ProduceMicroBlockEvent::MicroBlock(block1, result))
                            .ok();
                        break Some(event);
                    }
                } else {
                    break None;
                }
            }

            // We have dropped the blockchain lock.
            // Wait a bit before trying to produce a block again.
            delay += Self::CHECK_MEMPOOL_DELAY;
            time::sleep(Self::CHECK_MEMPOOL_DELAY).await;
        };

        if let Some(event) = return_value {
            return (event, self);
        }

        debug!(
            block_number = self.block_number,
            view_number = self.view_number,
            slot_band = self.validator_slot_band,
            "Not our turn, waiting for micro block #{}.{}",
            self.block_number,
            self.view_number,
        );

        time::sleep(self.view_change_delay).await;

        info!(
            block_number = self.block_number,
            view_number = self.view_number,
            "No micro block received within timeout, starting view change"
        );

        // Acquire a blockchain read lock and check if the state still matches to fetch active validators.
        let active_validators = {
            let blockchain = self.blockchain.read();
            if in_current_state(&blockchain.head()) {
                Some(blockchain.current_validators().unwrap())
            } else {
                None
            }
        };
        if active_validators.is_none() {
            return (None, self);
        }

        let (view_change, view_change_proof) = self.change_view(active_validators.unwrap()).await;
        info!(
            block_number = self.block_number,
            view_number = self.view_number,
            new_view_number = view_change.new_view_number,
            "View change completed"
        );

        let event = ProduceMicroBlockEvent::ViewChange(view_change, view_change_proof);
        (Some(event), self)
    }

    fn is_our_turn(&self, blockchain: &Blockchain) -> bool {
        let proposer_slot = blockchain.get_proposer_at(
            self.block_number,
            self.view_number,
            self.prev_seed.entropy(),
            None,
        );

        match proposer_slot {
            Some(slot) => slot.band == self.validator_slot_band,
            None => {
                // The only scenario where this could potentially fail is if the macro block that
                // precedes self.block_number is pruned while we're initializing ProduceMicroBlock.
                warn!("Failed to find next proposer");
                false
            }
        }
    }

    fn produce_micro_block(&self, blockchain: &Blockchain) -> MicroBlock {
        let timestamp = u64::max(
            blockchain.timestamp(),
            systemtime_to_timestamp(SystemTime::now()),
        );

        // First we try to fill the block with control transactions
        let mut block_available_bytes = MicroBlock::get_available_bytes(self.fork_proofs.len());

        let (mut transactions, txn_size) = self
            .mempool
            .get_control_transactions_for_block(block_available_bytes);

        block_available_bytes -= txn_size;

        let (mut regular_transactions, _) = self
            .mempool
            .get_transactions_for_block(block_available_bytes);

        transactions.append(&mut regular_transactions);

        self.block_producer.next_micro_block(
            blockchain,
            timestamp,
            self.view_number,
            self.view_change_proof.clone(),
            self.fork_proofs.clone(),
            transactions,
            vec![], // TODO: Allow validators to set extra data field.
        )
    }

    async fn change_view(
        &mut self,
        active_validators: Validators,
    ) -> (ViewChange, ViewChangeProof) {
        let new_view_number = self.view_number + 1;
        let view_change = ViewChange {
            block_number: self.block_number,
            new_view_number,
            vrf_entropy: self.prev_seed.entropy(),
        };

        // Include the previous_view_change_proof only if it has not yet been persisted on chain.
        let view_change_proof = self.view_change.as_ref().and_then(|vc| {
            if vc.block_number == self.block_number {
                Some(self.view_change_proof.as_ref().unwrap().sig.clone())
            } else {
                None
            }
        });

        let (view_change, view_change_proof) = ViewChangeAggregation::start(
            view_change.clone(),
            view_change_proof,
            self.block_producer.voting_key.clone(),
            self.validator_slot_band,
            active_validators,
            Arc::clone(&self.network),
        )
        .await;

        // Set the view change and view_change_proof properties so in case another view change happens they are available.
        self.view_number = view_change.new_view_number;
        self.view_change = Some(view_change.clone());
        self.view_change_proof = Some(view_change_proof.clone());

        (view_change, view_change_proof)
    }
}

pub(crate) struct ProduceMicroBlock<TValidatorNetwork> {
    next_event: Option<
        BoxFuture<
            'static,
            (
                Option<ProduceMicroBlockEvent>,
                NextProduceMicroBlockEvent<TValidatorNetwork>,
            ),
        >,
    >,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProduceMicroBlock<TValidatorNetwork> {
    // Ignoring clippy warning because there wouldn't be much to be gained by refactoring this,
    // except making clippy happy
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        mempool: Arc<Mempool>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        fork_proofs: Vec<ForkProof>,
        prev_seed: VrfSeed,
        block_number: u32,
        view_number: u32,
        view_change_proof: Option<ViewChangeProof>,
        view_change: Option<ViewChange>,
        view_change_delay: Duration,
        empty_block_delay: Duration,
    ) -> Self {
        let next_event = NextProduceMicroBlockEvent::new(
            blockchain,
            mempool,
            network,
            block_producer,
            validator_slot_band,
            fork_proofs,
            prev_seed,
            block_number,
            view_number,
            view_change_proof,
            view_change,
            view_change_delay,
            empty_block_delay,
        )
        .next()
        .boxed();
        Self {
            next_event: Some(next_event),
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream
    for ProduceMicroBlock<TValidatorNetwork>
{
    type Item = ProduceMicroBlockEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next_event = match self.next_event.as_mut() {
            Some(next_event) => next_event,
            None => return Poll::Ready(None),
        };

        let (event, next_event) = ready!(next_event.poll_unpin(cx));
        let event = match event {
            Some(event) => event,
            None => {
                self.next_event.take();
                return Poll::Ready(None);
            }
        };

        self.next_event = match &event {
            ProduceMicroBlockEvent::MicroBlock(..) => None,
            ProduceMicroBlockEvent::ViewChange(..) => Some(next_event.next().boxed()),
        };
        Poll::Ready(Some(event))
    }
}
