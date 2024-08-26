use std::{
    cmp,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use futures::{future::BoxFuture, ready, FutureExt, Stream};
use nimiq_block::{Block, EquivocationProof, MicroBlock, SkipBlockInfo};
use nimiq_blockchain::{BlockProducer, BlockProducerError, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_mempool::mempool::Mempool;
use nimiq_primitives::policy::Policy;
use nimiq_time::sleep;
use nimiq_utils::time::systemtime_to_timestamp;
use nimiq_validator_network::ValidatorNetwork;
use nimiq_vrf::VrfSeed;
use parking_lot::RwLock;

use crate::{aggregation::skip_block::SkipBlockAggregation, validator::BlockPublisher};

// Ignoring this clippy warning since size difference is not that much (320
// bytes) and we probably don't want the performance penalty of the allocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock(MicroBlock, PushResult),
}

#[derive(Clone)]
struct NextProduceMicroBlockEvent<TValidatorNetwork> {
    blockchain: Arc<RwLock<Blockchain>>,
    mempool: Arc<Mempool>,
    network: Arc<TValidatorNetwork>,
    block_producer: BlockProducer,
    validator_slot_band: u16,
    equivocation_proofs: Vec<EquivocationProof>,
    prev_seed: VrfSeed,
    block_number: u32,
    producer_timeout: Duration,
    block_separation_time: Duration,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> NextProduceMicroBlockEvent<TValidatorNetwork> {
    // Ignoring clippy warning because there wouldn't be much to be gained by refactoring this,
    // except making clippy happy
    #[allow(clippy::too_many_arguments)]
    fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        mempool: Arc<Mempool>,
        network: Arc<TValidatorNetwork>,
        block_producer: BlockProducer,
        validator_slot_band: u16,
        equivocation_proofs: Vec<EquivocationProof>,
        prev_seed: VrfSeed,
        block_number: u32,
        producer_timeout: Duration,
        block_separation_time: Duration,
    ) -> Self {
        Self {
            blockchain,
            mempool,
            network,
            block_producer,
            validator_slot_band,
            equivocation_proofs,
            prev_seed,
            block_number,
            producer_timeout,
            block_separation_time,
        }
    }

    // TODO Remove this line
    #[allow(unreachable_code)]
    async fn next(
        self,
    ) -> (
        Option<ProduceMicroBlockEvent>,
        NextProduceMicroBlockEvent<TValidatorNetwork>,
    ) {
        let in_current_state = |head: &Block| {
            self.prev_seed == *head.seed() && self.block_number == head.block_number() + 1
        };

        let block_publisher = BlockPublisher {
            network: Arc::clone(&self.network),
        };

        let mut delay = Duration::default();
        let mut expected_next_ts;

        let return_value = loop {
            // Wait for the expected timestamp to arrive before producing the block.
            // We sleep at the beginning of the loop such that we can `continue` into the sleep.
            // We can't sleep while holding the blockchain lock.
            if !delay.is_zero() {
                sleep(delay).await;
            }

            // Acquire blockchain.upgradable_read() to prevent further changes to the blockchain while
            // we're constructing the block. Check if we're still in the correct state, abort otherwise.
            let blockchain = self.blockchain.upgradable_read();

            // Calculate the expected block time as expected by the reward function.
            expected_next_ts = self.expected_next_timestamp(&blockchain);

            if !in_current_state(blockchain.head()) {
                break Some(None);
            } else if !self.is_our_turn(&blockchain) {
                break None;
            }

            // We want to produce a block at the expected timestamp for this block in this batch
            // as it is calculated by the reward function and set the producer timeout accordingly
            let now = systemtime_to_timestamp(SystemTime::now());

            // If the timestamp hasn't passed, wait until the expected block timestamp
            // to produce the block.
            if expected_next_ts > now {
                delay = Duration::from_millis(expected_next_ts - now);
                continue;
            }

            // If the expected timestamp is already in the past, produce a block immediately.
            info!(
                block_number = self.block_number,
                slot_band = self.validator_slot_band,
                "Our turn, producing micro block #{}",
                self.block_number,
            );

            let block = match self.produce_micro_block(&blockchain) {
                Ok(block) => block,
                Err(error) => {
                    error!(
                        block_number = self.block_number,
                        %error,
                        "Failed to construct micro block"
                    );

                    // Add an artificial panic here so that it's easier to catch any remaining
                    // mempool bugs.
                    // TODO Remove this line and the #[allow(unreachable_code)] annotation on this function.
                    panic!("Failed to construct micro block");

                    // Sleep a bit to allow the task to be cancelled externally if needed.
                    delay = Duration::from_millis(50);
                    continue;
                }
            };

            let num_transactions = block
                .body
                .as_ref()
                .map(|body| body.transactions.len())
                .unwrap_or(0);

            debug!(
                block_number = block.header.block_number,
                num_transactions,
                ?delay,
                "Produced micro block {} with {} transactions",
                block,
                num_transactions
            );

            // Use a trusted push since these blocks were generated by this validator
            let result = if cfg!(feature = "trusted_push") {
                Blockchain::trusted_push(blockchain, Block::Micro(block.clone()), &block_publisher)
            } else {
                Blockchain::push(blockchain, Block::Micro(block.clone()), &block_publisher)
            };

            if let Err(error) = &result {
                error!(
                    block_number = self.block_number,
                    %error,
                    "Failed to push our own micro block onto the chain"
                );

                // Sleep a bit to allow the task to be cancelled externally if needed.
                delay = Duration::from_millis(50);
                continue;
            }

            let event = result
                .map(move |result| ProduceMicroBlockEvent::MicroBlock(block, result))
                .ok();
            break Some(event);
        };

        if let Some(event) = return_value {
            return (event, self);
        }

        debug!(
            block_number = self.block_number,
            slot_band = self.validator_slot_band,
            "Not our turn, waiting for micro block #{}",
            self.block_number,
        );

        // Wait for the block to be produced. We wait for at least `producer_timeout` here, but can
        // wait longer if the expected timestamp of the block is further in the future.
        let now = systemtime_to_timestamp(SystemTime::now());
        let wait_until_min = now + self.producer_timeout.as_millis() as u64;
        let wait_until_expected = expected_next_ts
            + (self
                .producer_timeout
                .saturating_sub(self.block_separation_time))
            .as_millis() as u64;

        let next_block_timeout = cmp::max(wait_until_min, wait_until_expected) - now;
        let timeout = Duration::from_millis(next_block_timeout);
        sleep(timeout).await;

        info!(
            block_number = self.block_number,
            ?timeout,
            "No micro block received within timeout, producing skip block"
        );

        // Acquire a blockchain read lock and check if the state still matches to fetch active validators.
        let active_validators = {
            let blockchain = self.blockchain.read();
            if in_current_state(blockchain.head()) {
                Some(blockchain.current_validators().unwrap().clone())
            } else {
                None
            }
        };
        if active_validators.is_none() {
            return (None, self);
        }

        let skip_block_info = SkipBlockInfo {
            block_number: self.block_number,
            vrf_entropy: self.prev_seed.entropy(),
        };

        let (_, skip_block_proof) = SkipBlockAggregation::start(
            skip_block_info.clone(),
            self.block_producer.voting_key.clone(),
            self.validator_slot_band,
            active_validators.unwrap(),
            Arc::clone(&self.network),
        )
        .await;

        let result = {
            // Acquire blockchain.upgradable_read() to prevent further changes to the blockchain while
            // we're constructing the block. Check if we're still in the correct state, abort otherwise.
            let blockchain = self.blockchain.upgradable_read();
            let head = blockchain.head();

            if !in_current_state(head) {
                None
            } else {
                let timestamp = head.timestamp() + Policy::MIN_PRODUCER_TIMEOUT;

                let skip_block = self.block_producer.next_micro_block(
                    &blockchain,
                    timestamp,
                    vec![],
                    vec![],
                    vec![], // TODO: Allow validators to set extra data field.
                    Some(skip_block_proof),
                );

                let skip_block = match skip_block {
                    Ok(block) => block,
                    Err(error) => {
                        error!(
                            block_number = self.block_number,
                            %error,
                            "Failed to construct skip block"
                        );
                        drop(blockchain);
                        return (None, self);
                    }
                };

                // Use a trusted push since these blocks were generated by this validator
                let result = if cfg!(feature = "trusted_push") {
                    Blockchain::trusted_push(
                        blockchain,
                        Block::Micro(skip_block.clone()),
                        &block_publisher,
                    )
                } else {
                    Blockchain::push(
                        blockchain,
                        Block::Micro(skip_block.clone()),
                        &block_publisher,
                    )
                };

                if let Err(error) = &result {
                    error!(
                        block_number = self.block_number,
                        %error,
                        "Failed to push our own skip block onto the chain"
                    );
                }

                Some((result, skip_block))
            }
        };

        if let Some((result, block)) = result {
            let event = result
                .map(move |result| ProduceMicroBlockEvent::MicroBlock(block, result))
                .ok();
            info!(block_number = self.block_number, "Skip block pushed");

            (event, self)
        } else {
            (None, self)
        }
    }

    fn is_our_turn(&self, blockchain: &Blockchain) -> bool {
        let proposer_slot = blockchain.get_proposer(
            self.block_number,
            self.block_number,
            self.prev_seed.entropy(),
            None,
        );

        match proposer_slot {
            Ok(slot) => slot.band == self.validator_slot_band,
            Err(error) => {
                // The only scenario where this could potentially fail is if the macro block that
                // precedes self.block_number is pruned while we're initializing ProduceMicroBlock.
                warn!(%error, "Failed to find next proposer");
                false
            }
        }
    }

    fn produce_micro_block(
        &self,
        blockchain: &Blockchain,
    ) -> Result<MicroBlock, BlockProducerError> {
        let timestamp = u64::max(
            blockchain.timestamp(),
            systemtime_to_timestamp(SystemTime::now()),
        );

        // First we try to fill the block with control transactions
        let mut block_available_bytes = MicroBlock::get_available_bytes(&self.equivocation_proofs);

        let (mut transactions, txn_size) = self
            .mempool
            .get_control_transactions_for_block_locked(blockchain, block_available_bytes);

        block_available_bytes = block_available_bytes.saturating_sub(txn_size);

        let (mut regular_transactions, _) = self
            .mempool
            .get_transactions_for_block_locked(blockchain, block_available_bytes);

        transactions.append(&mut regular_transactions);

        self.block_producer.next_micro_block(
            blockchain,
            timestamp,
            self.equivocation_proofs.clone(),
            transactions,
            vec![], // TODO: Allow validators to set extra data field.
            None,
        )
    }

    fn expected_next_timestamp(&self, blockchain: &Blockchain) -> u64 {
        let last_macro_block = blockchain.macro_head();
        let block_separation_time = self.block_separation_time.as_millis() as u64;
        let next_block_index_in_batch =
            (blockchain.block_number() + 1).saturating_sub(last_macro_block.block_number()) as u64;
        last_macro_block.header.timestamp + (next_block_index_in_batch * block_separation_time)
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
        equivocation_proofs: Vec<EquivocationProof>,
        prev_seed: VrfSeed,
        block_number: u32,
        producer_timeout: Duration,
        block_separation_time: Duration,
    ) -> Self {
        let next_event = NextProduceMicroBlockEvent::new(
            blockchain,
            mempool,
            network,
            block_producer,
            validator_slot_band,
            equivocation_proofs,
            prev_seed,
            block_number,
            producer_timeout,
            block_separation_time,
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

        let (event, _) = ready!(next_event.poll_unpin(cx));
        let event = match event {
            Some(event) => event,
            None => {
                self.next_event.take();
                return Poll::Ready(None);
            }
        };

        self.next_event = match &event {
            ProduceMicroBlockEvent::MicroBlock(..) => None,
        };
        Poll::Ready(Some(event))
    }
}
