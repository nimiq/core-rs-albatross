use std::{
    cmp,
    collections::HashSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use futures::{future::BoxFuture, ready, FutureExt, Stream};
use nimiq_block::{Block, EquivocationProof, MicroBlock, SkipBlockInfo};
use nimiq_blockchain::{BlockProducer, BlockProducerError, Blockchain};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mempool::mempool::Mempool;
use nimiq_primitives::policy::{upgrades, Policy};
use nimiq_time::sleep;
use nimiq_utils::time::systemtime_to_timestamp;
use nimiq_validator_network::ValidatorNetwork;
use nimiq_vrf::VrfSeed;
use parking_lot::RwLock;

use crate::{aggregation::skip_block::SkipBlockAggregation, validator::Validator};

pub(crate) enum ProduceMicroBlockEvent {
    MicroBlock,
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

    async fn next(
        self,
    ) -> (
        Option<ProduceMicroBlockEvent>,
        NextProduceMicroBlockEvent<TValidatorNetwork>,
    ) {
        let in_current_state = |head: &Block| {
            self.prev_seed == *head.seed() && self.block_number == head.block_number() + 1
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

            let block = Block::Micro(block);

            debug!(
                block_number = block.block_number(),
                num_transactions,
                ?delay,
                "Produced micro block {} with {} transactions",
                block,
                num_transactions
            );

            // Publish the block. It is valid as we have just created it.
            Validator::publish_block(Arc::clone(&self.network), block.clone());

            // Use a trusted push since these blocks were generated by this validator
            let result = if cfg!(feature = "trusted_push") {
                // no hook required as we are the producer
                Blockchain::trusted_push(blockchain, block, &())
            } else {
                Blockchain::push_with_hook(
                    blockchain,
                    block,
                    // no hook required as we are the producer
                    &(),
                )
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
                .map(move |_result| ProduceMicroBlockEvent::MicroBlock)
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

        // Acquire a blockchain read lock and check if the state still matches to fetch the active
        // validators as well as the deterministic state root and protocol version of the skip
        // block we are about to produce. From `upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING` on, the state
        // root is part of the signed skip block message, so it must be known before aggregating.
        let skip_block_inputs = {
            let blockchain = self.blockchain.read();
            if in_current_state(blockchain.head()) {
                let protocol_version = blockchain.state().current_version();
                let state_root = match blockchain.next_skip_block_state_root() {
                    Some(state_root) => Some(state_root),
                    // Before the state root binding it is not part of the signed message
                    // (see `SkipBlockInfo::signing_hash`), so any value works and a
                    // validator with an incomplete accounts state can still participate
                    None if protocol_version < upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING => {
                        Some(Blake2bHash::default())
                    }
                    None => {
                        warn!(
                            block_number = self.block_number,
                            "Cannot participate in the skip block aggregation, accounts state \
                             is incomplete"
                        );
                        None
                    }
                };
                state_root.map(|state_root| {
                    (
                        blockchain.current_validators().unwrap().clone(),
                        state_root,
                        protocol_version,
                    )
                })
            } else {
                None
            }
        };
        let Some((active_validators, state_root, protocol_version)) = skip_block_inputs else {
            return (None, self);
        };

        let skip_block_info = SkipBlockInfo {
            network_id: self.blockchain.read().network_id,
            block_number: self.block_number,
            vrf_entropy: self.prev_seed.entropy(),
        };

        let (_, skip_block_proof) = SkipBlockAggregation::start(
            skip_block_info.clone(),
            state_root,
            protocol_version,
            self.block_producer.voting_key.clone(),
            self.validator_slot_band,
            active_validators,
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

                let block = Block::Micro(skip_block);

                // Publish the block. It is valid as we have just created it.
                Validator::publish_block(Arc::clone(&self.network), block.clone());

                // Use a trusted push since these blocks were generated by this validator
                let result = if cfg!(feature = "trusted_push") {
                    Blockchain::trusted_push(blockchain, block, &())
                } else {
                    Blockchain::push_with_hook(blockchain, block, &())
                };

                if let Err(error) = &result {
                    error!(
                        block_number = self.block_number,
                        %error,
                        "Failed to push our own skip block onto the chain"
                    );
                }

                Some(result)
            }
        };

        if let Some(result) = result {
            let event = result
                .map(move |_result| ProduceMicroBlockEvent::MicroBlock)
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

        let block_available_bytes = MicroBlock::get_available_bytes(&self.equivocation_proofs);

        // Control transactions are prioritized over regular ones, but they must not be able to
        // claim the whole body: signaling staking transactions are valueless and may carry a zero
        // fee, so an unbounded priority would let free spam starve regular transactions out of
        // every block. Hence the first pass gives control transactions at most half of the body.
        let control_bytes = block_available_bytes / 2;

        let (mut transactions, control_size) = self
            .mempool
            .get_control_transactions_for_block_locked(blockchain, control_bytes);

        // Regular transactions get everything the control transactions did not use.
        let (mut regular_transactions, regular_size) =
            self.mempool.get_transactions_for_block_locked(
                blockchain,
                block_available_bytes.saturating_sub(control_size),
            );

        // Whatever the regular transactions did not use is handed back to the control
        // transactions, so that block space is never wasted.
        let (mut extra_control_transactions, _) =
            self.mempool.get_control_transactions_for_block_locked(
                blockchain,
                block_available_bytes.saturating_sub(control_size + regular_size),
            );

        transactions.append(&mut extra_control_transactions);
        transactions.append(&mut regular_transactions);

        // The mempool has handed these transactions over to us, meaning they are gone from it.
        // Whatever does not end up in the block would be lost even though nothing is wrong with
        // it, so keep a copy to hand back. Cloning a block body once per production is cheap next
        // to executing it.
        let selected_transactions = transactions.clone();

        let block = self.block_producer.next_micro_block(
            blockchain,
            timestamp,
            self.equivocation_proofs.clone(),
            transactions,
            vec![], // TODO: Allow validators to set extra data field.
            None,
        );

        match &block {
            Ok(micro_block) => {
                // Hand back the transactions we selected but did not include -- ones cut from the
                // body because they turned out unappliable in this block, or that sit behind such
                // a transaction. Unappliable ones are admitted again (the mempool checks against
                // the head block, which is how they got in the first time), but the block we just
                // produced advances the head past what made them unappliable, so the mempool
                // eviction that runs when it is adopted drops them for good.
                let included: HashSet<Blake2bHash> = micro_block
                    .body
                    .as_ref()
                    .map(|body| {
                        body.transactions
                            .iter()
                            .map(|tx| tx.get_raw_transaction().hash())
                            .collect()
                    })
                    .unwrap_or_default();

                let returned: Vec<_> = selected_transactions
                    .into_iter()
                    .filter(|tx| !included.contains(&tx.hash()))
                    .collect();

                if !returned.is_empty() {
                    self.mempool
                        .return_transactions_locked(blockchain, returned);
                }
            }
            Err(_) => {
                // The block was never built (not a transaction problem -- an unappliable
                // transaction only fails itself now), so the whole selection goes back.
                self.mempool
                    .return_transactions_locked(blockchain, selected_transactions);
            }
        }

        block
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
            ProduceMicroBlockEvent::MicroBlock => None,
        };
        Poll::Ready(Some(event))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use futures::StreamExt;
    use nimiq_block::{ForkProof, MicroJustification};
    use nimiq_consensus::messages::{BlockHeaderMessage, BlockHeaderTopic};
    use nimiq_database::traits::WriteTransaction;
    use nimiq_keys::{KeyPair, PrivateKey};
    use nimiq_mempool::config::MempoolConfig;
    use nimiq_network_interface::network::Network;
    use nimiq_network_mock::MockHub;
    use nimiq_primitives::networks::NetworkId;
    use nimiq_test_log::test;
    use nimiq_test_utils::{
        block_production::TemporaryBlockProducer,
        blockchain::{generate_transactions, validator_address, REWARD_KEY},
    };
    use nimiq_time::timeout;
    use nimiq_validator_network::network_impl::ValidatorNetworkImpl;

    use super::*;

    #[test(tokio::test)]
    async fn it_abstains_from_skip_block_aggregation_on_incomplete_accounts() {
        let temp_producer = TemporaryBlockProducer::new_with_protocol_version(
            upgrades::v2::SKIP_BLOCK_STATE_ROOT_BINDING,
        );

        // Make the accounts trie incomplete.
        {
            let blockchain = temp_producer.blockchain.read();
            let mut txn = blockchain.write_transaction();
            blockchain
                .state
                .accounts
                .reinitialize_as_incomplete(&mut (&mut txn).into());
            txn.commit();
        }

        let blockchain = Arc::clone(&temp_producer.blockchain);
        let mempool = Arc::new(Mempool::new(
            Arc::clone(&blockchain),
            MempoolConfig::default(),
        ));
        let mut hub = MockHub::default();
        let network = Arc::new(ValidatorNetworkImpl::new(Arc::new(hub.new_network())));

        let (prev_seed, block_number) = {
            let blockchain = blockchain.read();
            (
                blockchain.head().seed().clone(),
                blockchain.block_number() + 1,
            )
        };

        // Slot band 1 never matches the single genesis validator's band, so this node is
        // not the proposer and takes the skip block path as soon as the timeout elapses.
        let event = NextProduceMicroBlockEvent::new(
            blockchain,
            mempool,
            network,
            temp_producer.producer.clone(),
            1,
            vec![],
            prev_seed,
            block_number,
            Duration::ZERO,
            Duration::ZERO,
        );

        // With an incomplete accounts state, from the state root binding on the validator
        // must abstain from the aggregation instead of panicking.
        let (produced, _) = timeout(Duration::from_secs(10), event.next())
            .await
            .expect("Producer must abstain instead of aggregating");
        assert!(produced.is_none());
    }

    /// Two signed micro headers for the same height from one validator always form a valid fork
    /// proof, because their VRF entropy is the same. So even when our own push of a produced block
    /// fails and we retry, the network must only ever see the block we accepted.
    #[test(tokio::test)]
    async fn it_only_publishes_the_micro_block_it_accepted() {
        let temp_producer = TemporaryBlockProducer::new();
        let blockchain = Arc::clone(&temp_producer.blockchain);
        let mempool = Arc::new(Mempool::new(
            Arc::clone(&blockchain),
            MempoolConfig::default(),
        ));

        // Add a transaction to the mempool, then include it in a block behind the mempool's back.
        // Our next block picks it up again and our own push rejects it as a duplicate.
        let key_pair = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());
        let height = blockchain.read().block_number();
        let tx = generate_transactions(&key_pair, height, NetworkId::UnitAlbatross, 1, 7)
            .pop()
            .unwrap();
        mempool.add_transaction(tx.clone(), None).unwrap();
        temp_producer.next_block_with_txs(vec![], false, vec![tx]);

        let mut hub = MockHub::default();
        let network = hub.new_network();
        let observer = hub.new_network();
        network.dial_mock(&observer);
        let mut headers = observer.subscribe::<BlockHeaderTopic>().await.unwrap();

        let (prev_seed, block_number) = {
            let blockchain = blockchain.read();
            (
                blockchain.head().seed().clone(),
                blockchain.block_number() + 1,
            )
        };

        // Slot band 0 is the single genesis validator, so it is our turn.
        let mut producer = ProduceMicroBlock::new(
            Arc::clone(&blockchain),
            mempool,
            Arc::new(ValidatorNetworkImpl::new(Arc::new(network))),
            temp_producer.producer.clone(),
            0,
            vec![],
            prev_seed,
            block_number,
            Duration::from_secs(60),
            Duration::ZERO,
        );
        let event = timeout(Duration::from_secs(10), producer.next())
            .await
            .expect("Producer must produce the block");
        assert!(
            event.is_some(),
            "The slot must still be filled after the failed push"
        );
        assert_eq!(blockchain.read().block_number(), block_number);

        // Everything the network saw for this height, signed with our key.
        let mut published = Vec::new();
        while let Ok(Some((message, _))) = timeout(Duration::from_secs(1), headers.next()).await {
            if let BlockHeaderMessage::Micro {
                header,
                justification: MicroJustification::Micro(signature),
            } = message
                && header.block_number == block_number
            {
                published.push((header, signature));
            }
        }

        let signing_key = &temp_producer.producer.signing_key.public;
        for (i, (header1, signature1)) in published.iter().enumerate() {
            for (header2, signature2) in &published[i + 1..] {
                let proof = ForkProof::new(
                    validator_address(),
                    header1.clone(),
                    signature1.clone(),
                    header2.clone(),
                    signature2.clone(),
                );
                assert!(
                    proof
                        .verify_excluding_address_and_network(signing_key)
                        .is_err(),
                    "Published headers for #{block_number} are a valid fork proof against us"
                );
            }
        }
        let published: Vec<Blake2bHash> =
            published.iter().map(|(header, _)| header.hash()).collect();
        assert_eq!(
            published,
            vec![blockchain.read().head_hash()],
            "Only the accepted block may be published"
        );
    }
}
