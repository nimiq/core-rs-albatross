use std::{
    error::Error,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::stream::StreamExt;
use nimiq_account::Validator as ValidatorAccount;
use nimiq_block::{Block, BlockType, EquivocationProof};
use nimiq_blockchain::{interface::HistoryInterface, BlockProducer, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, ForkEvent};
use nimiq_consensus::{
    messages::{BlockBodyTopic, BlockHeaderMessage, BlockHeaderTopic},
    Consensus, ConsensusEvent, ConsensusProxy,
};
use nimiq_database::{
    declare_table,
    mdbx::MdbxDatabase,
    traits::{Database, ReadTransaction, WriteTransaction},
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair};
use nimiq_mempool::config::MempoolConfig;
use nimiq_mempool_task::MempoolTask;
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, NetworkEvent, SubscribeEvents},
    request::request_handler,
};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::spawn;
use nimiq_validator_network::{PubsubId, ValidatorNetwork};
use parking_lot::RwLock;
#[cfg(feature = "metrics")]
use tokio_metrics::TaskMonitor;
use tokio_stream::wrappers::BroadcastStream;

use crate::{
    aggregation::tendermint::{proposal::RequestProposal, state::MacroState},
    jail::EquivocationProofPool,
    key_utils::VotingKeys,
    micro::ProduceMicroBlock,
    proposal_buffer::{ProposalBuffer, ProposalReceiver},
    r#macro::{MappedReturn, ProduceMacroBlock, ProposalTopic},
};

#[derive(PartialEq)]
enum ValidatorStakingState {
    Active,
    Inactive(Option<u32>),
    UnknownOrNoStake,
}

pub struct ConsensusState {
    equivocation_proofs: EquivocationProofPool,
}

/// Validator inactivity
pub struct InactivityState {
    pub inactive_tx_hash: Blake2bHash,
    pub inactive_tx_validity_window_start: u32,
}

pub struct ValidatorState {
    pub validator_address: Address,
    pub signing_key: SchnorrKeyPair,
    pub voting_keys: VotingKeys,
    pub fee_key: SchnorrKeyPair,
    pub automatic_reactivate: bool,
    pub slot_band: Option<u16>,
    pub consensus: ConsensusState,
}

declare_table!(ValidatorTable, "ValidatorState", () => MacroState);

pub struct Validator<TValidatorNetwork: ValidatorNetwork + 'static>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    pub consensus: ConsensusProxy<TValidatorNetwork::NetworkType>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub network: Arc<TValidatorNetwork>,

    table: ValidatorTable,
    env: MdbxDatabase,

    state: Arc<RwLock<ValidatorState>>,
    inactivity_state: Option<InactivityState>,

    proposal_receiver: ProposalReceiver<TValidatorNetwork>,

    consensus_event_rx: BroadcastStream<ConsensusEvent>,
    network_event_rx: SubscribeEvents<<TValidatorNetwork::NetworkType as Network>::PeerId>,
    fork_event_rx: BroadcastStream<ForkEvent>,

    macro_producer: Option<ProduceMacroBlock<TValidatorNetwork>>,
    macro_state: Arc<RwLock<Option<MacroState>>>,

    micro_producer: Option<ProduceMicroBlock<TValidatorNetwork>>,

    pub mempool_task: MempoolTask<TValidatorNetwork::NetworkType>,
}

impl ValidatorState {
    fn get_validator(&self, blockchain: &Blockchain) -> Option<ValidatorAccount> {
        blockchain
            .get_staking_contract_if_complete(None)
            .and_then(|staking_contract| {
                // Then fetch the validator to see if it is active.
                let data_store = blockchain.get_staking_contract_store();
                let txn = blockchain.read_transaction();
                staking_contract.get_validator(&data_store.read(&txn), &self.validator_address)
            })
    }

    fn get_staking_state(&self, blockchain: &Blockchain) -> ValidatorStakingState {
        self.get_validator(blockchain).map_or(
            ValidatorStakingState::UnknownOrNoStake,
            |validator| match validator.inactive_from {
                Some(_) => ValidatorStakingState::Inactive(validator.jailed_from),
                None => ValidatorStakingState::Active,
            },
        )
    }

    /// Checks whether we are an elected validator in the current epoch.
    fn is_elected(&self) -> bool {
        self.slot_band.is_some()
    }
}

impl<TValidatorNetwork: ValidatorNetwork> Validator<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    /// The minimum time to wait for a micro block before starting a skip block.
    const MIN_PRODUCER_TIMEOUT: Duration = Duration::from_millis(Policy::MIN_PRODUCER_TIMEOUT);
    /// The maximum time to wait for a micro block before starting a skip block.
    const MAX_PRODUCER_TIMEOUT: Duration = Duration::from_secs(16);
    /// The number of blocks to consider when calculating the micro block producer timeout.
    const PRODUCER_TIMEOUT_WINDOW_SIZE: u32 = 120;
    /// Multiplier applied to the average block time when calculating the micro block producer timeout.
    const PRODUCER_TIMEOUT_MULTIPLIER: u64 = 4;
    /// The targeted block time.
    const BLOCK_SEPARATION_TIME: Duration = Duration::from_millis(Policy::BLOCK_SEPARATION_TIME);
    /// The maximum number of bytes that equivocation proofs are allowed to take up in a block.
    const EQUIVOCATION_PROOFS_MAX_SIZE: usize = 1_000; // bytes

    pub fn new(
        env: MdbxDatabase,
        consensus: &Consensus<TValidatorNetwork::NetworkType>,
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        validator_address: Address,
        automatic_reactivate: bool,
        signing_key: SchnorrKeyPair,
        voting_keys: VotingKeys,
        fee_key: SchnorrKeyPair,
        mempool_config: MempoolConfig,
    ) -> Self {
        let consensus_event_rx = consensus.subscribe_events();

        let blockchain_rg = blockchain.read();
        let fork_event_rx = BroadcastStream::new(blockchain_rg.fork_notifier.subscribe());
        drop(blockchain_rg);

        let network_event_rx = network.subscribe_events();

        let blockchain_state = ConsensusState {
            equivocation_proofs: EquivocationProofPool::new(),
        };

        env.create_regular_table(&ValidatorTable);

        let macro_state: Option<MacroState> = {
            let read_transaction = env.read_transaction();
            read_transaction.get(&ValidatorTable, &())
        };
        let macro_state = Arc::new(RwLock::new(macro_state));

        let (proposal_sender, proposal_receiver) = ProposalBuffer::new(
            Arc::clone(&blockchain),
            Arc::clone(&network),
            consensus.proxy(),
        );

        let mempool = MempoolTask::new(consensus, Arc::clone(&blockchain), mempool_config);

        Self::init_network_request_receivers(&consensus.network, &network, &macro_state);

        let network1 = Arc::clone(&network);
        spawn(async move {
            network1
                .subscribe::<ProposalTopic<TValidatorNetwork>>()
                .await
                .expect("Failed to subscribe to proposal topic")
                .for_each(|proposal| async { proposal_sender.send(proposal) })
                .await
        });

        Self {
            consensus: consensus.proxy(),
            blockchain,
            network,

            table: ValidatorTable,
            env,

            state: Arc::new(RwLock::new(ValidatorState {
                validator_address,
                signing_key,
                voting_keys,
                fee_key,
                automatic_reactivate,
                slot_band: None,
                consensus: blockchain_state,
            })),
            inactivity_state: None,

            proposal_receiver,

            consensus_event_rx,
            network_event_rx,
            fork_event_rx,

            macro_producer: None,
            macro_state: Arc::clone(&macro_state),

            micro_producer: None,

            mempool_task: mempool,
        }
    }

    fn init_network_request_receivers(
        raw_network: &Arc<TValidatorNetwork::NetworkType>,
        validator_network: &Arc<TValidatorNetwork>,
        macro_state: &Arc<RwLock<Option<MacroState>>>,
    ) {
        // Proposal requests are sent through the validator network (see
        // `TendermintProtocol::request_proposal`), which wraps them in a
        // `ValidatorMessage` and therefore uses a distinct wire `TYPE_ID`. They
        // must be received on the validator network as well, otherwise the
        // `TYPE_ID` does not match and the request never reaches this handler.
        //
        // The validator network already unwraps the message and validates the
        // sender, yielding the validator id instead of a peer id. The
        // `RequestProposal` handler does not use the peer id, so we substitute the
        // local peer id in order to reuse the generic `request_handler`. The
        // request id is the underlying network's request id, so the response is
        // served on the raw network and routed back to the requester by id.
        let local_peer_id = raw_network.get_local_peer_id();
        let stream = validator_network
            .receive_requests::<RequestProposal>()
            .map(move |(request, request_id, _validator_id)| (request, request_id, local_peer_id))
            .boxed();
        spawn(Box::pin(request_handler(raw_network, stream, macro_state)));
    }

    fn init(&mut self, head_hash: Option<&Blake2bHash>) {
        self.init_epoch();
        self.init_block_producer(head_hash);
    }

    /// Calculates the micro block producer timeout by averaging block times over a window ending at
    /// `head_block`. The window size is dynamically adjusted if there are not enough blocks
    /// available, i.e. at the beginning of the chain or if not all blocks are present in the
    /// database.
    fn compute_micro_block_producer_timeout(
        head_block: &Block,
        blockchain: &Blockchain,
    ) -> Duration {
        let end_block_number = head_block.block_number();
        let end_timestamp = head_block.timestamp();

        // Calculate the block number at the start of the window. Make sure this points to at least
        // one block after the genesis block, as the genesis timestamp can be unreliable.
        let start_block_number =
            end_block_number.saturating_sub(Self::PRODUCER_TIMEOUT_WINDOW_SIZE);
        let mut start_block_number =
            u32::max(start_block_number, Policy::genesis_block_number() + 1);

        // We might not have the block at `start_block_number` in our database. If it's missing,
        // move up the chain to the next succeeding macro block(s), adjusting `start_block_number`
        // in the process.
        let start_timestamp = loop {
            if let Ok(block) = blockchain.get_block_at(start_block_number, false, None) {
                break block.timestamp();
            }

            // Try the next macro block.
            start_block_number = Policy::macro_block_after(start_block_number);

            // Bail if we didn't find a macro block. This shouldn't happen as we should always have
            // at least the macro block at the beginning of our current batch.
            if start_block_number >= end_block_number {
                return Self::MIN_PRODUCER_TIMEOUT;
            }
        };

        // Calculate the effective window size.
        // If there are no blocks in the window, we return the default value.
        let effective_window_size = end_block_number.saturating_sub(start_block_number) as u64;
        if effective_window_size < 1 {
            return Self::MIN_PRODUCER_TIMEOUT;
        }

        // Calculate the timeout and clamp it to the allowed range.
        let avg_block_time = end_timestamp.saturating_sub(start_timestamp) / effective_window_size;
        let timeout = Duration::from_millis(avg_block_time * Self::PRODUCER_TIMEOUT_MULTIPLIER);
        timeout.clamp(Self::MIN_PRODUCER_TIMEOUT, Self::MAX_PRODUCER_TIMEOUT)
    }

    /// Resets the reactivation state if we sent a reactivate transaction that expired.
    /// This ensures that in future polls of the validator we continue trying to reactivate
    /// ourselves if the previous tx didn't get included within the validity window.
    fn check_reactivate(&mut self, block_number: u32) {
        // Check if the reactivate/activate transaction was sent
        if let Some(inactivity_state) = &self.inactivity_state {
            // We check this in the last possible block of the validity window
            let tx_validity_window_start = inactivity_state.inactive_tx_validity_window_start;
            if block_number
                >= tx_validity_window_start + Policy::transaction_validity_window_blocks() - 1
            {
                let blockchain = self.blockchain.read();
                let staking_state = self.state.read().get_staking_state(&blockchain);
                // Check that the transaction was sent in the validity window
                if (matches!(staking_state, ValidatorStakingState::Inactive(..)))
                    && !blockchain.history_store.tx_in_validity_window(
                        &inactivity_state.inactive_tx_hash.clone().into(),
                        None,
                    )
                {
                    // If we are inactive and no transaction has been seen in the expected validity window
                    // after an epoch, reset our inactive state
                    log::debug!("Resetting state to re-send reactivate transactions since we are inactive and validity window doesn't contain the transaction sent");
                    self.inactivity_state = None;
                }
            }
        }
    }

    fn init_epoch(&mut self) {
        self.state.write().slot_band = None;

        if !self.is_synced() {
            return;
        }

        let blockchain = self.blockchain.read();
        let validators = blockchain.current_validators().unwrap();

        let mut state = self.state.write();
        state.slot_band = validators.get_slot_band_by_address(&state.validator_address);

        if let Some(slot_band) = state.slot_band {
            let epoch_validator = validators
                .get_validator_by_slot_band(slot_band)
                .expect("validator slot band from address lookup must exist");
            log::info!(
                validator_address = %state.validator_address,
                validator_slot_band = slot_band,
                epoch_number = blockchain.epoch_number(),
                slots = ?epoch_validator.slots,
                "We are ELECTED in this epoch"
            );

            // Update the validator key to be the expected one (relevant in case of a key rotation).
            if state
                .voting_keys
                .update_current_key(epoch_validator.voting_key.compressed())
                .is_err()
            {
                panic!("Invalid validator configuration: None of the voting keys match the one expected from this validator in the current epoch")
            }

            // Compare configured validator signing key to the one in the current epoch to make sure it is the same.
            if epoch_validator.signing_key != state.signing_key.public {
                panic!("Invalid validator configuration: Configured signing key does not match signing key in this epoch");
            }
        } else {
            log::info!(
                validator_address = %state.validator_address,
                epoch_number = blockchain.epoch_number(),
                "We are NOT ELECTED in this epoch"
            );
        }

        // Inform the network about the current validator ID.
        self.network.set_validator_id(state.slot_band);

        // Set the elected validators of the current epoch in the network as well.
        self.network.set_validators(validators);
    }

    fn init_block_producer(&mut self, head_hash: Option<&Blake2bHash>) {
        self.macro_producer = None;
        self.micro_producer = None;

        if !self.is_synced() {
            return;
        }

        let state = self.state.read();
        let Some(slot_band) = state.slot_band else {
            // Not elected.
            return;
        };

        let blockchain = self.blockchain.read();

        if let Some(hash) = head_hash
            && blockchain.head_hash() != *hash
        {
            log::debug!("Bypassed initializing block producer for obsolete block");
            return;
        }

        let head = blockchain.head();
        let next_block_number = head.block_number() + 1;
        let network_id = head.network();
        let block_producer = BlockProducer::new(
            state.signing_key.clone(),
            state.voting_keys.get_current_key(),
        );

        debug!(
            next_block_number = next_block_number,
            "Initializing block producer"
        );

        match BlockType::of(next_block_number) {
            BlockType::Macro => {
                let active_validators = blockchain.current_validators().unwrap().clone();
                let proposal_stream = self.proposal_receiver.clone().boxed();

                drop(blockchain);

                self.macro_producer = Some(ProduceMacroBlock::new(
                    Arc::clone(&self.blockchain),
                    Arc::clone(&self.network),
                    block_producer,
                    slot_band,
                    active_validators,
                    network_id,
                    next_block_number,
                    self.macro_state.read().clone(),
                    proposal_stream,
                ));
            }
            BlockType::Micro => {
                let equivocation_proofs = state
                    .consensus
                    .equivocation_proofs
                    .get_equivocation_proofs_for_block(Self::EQUIVOCATION_PROOFS_MAX_SIZE);
                let prev_seed = head.seed().clone();

                self.micro_producer = Some(ProduceMicroBlock::new(
                    Arc::clone(&self.blockchain),
                    Arc::clone(&self.mempool_task.mempool),
                    Arc::clone(&self.network),
                    block_producer,
                    slot_band,
                    equivocation_proofs,
                    prev_seed,
                    next_block_number,
                    Self::compute_micro_block_producer_timeout(head, &blockchain),
                    Self::BLOCK_SEPARATION_TIME,
                ));
            }
        }
    }

    fn pause(&mut self) {
        self.state.write().slot_band = None;
        self.macro_producer = None;
        self.micro_producer = None;
    }

    fn on_blockchain_event(&mut self, event: BlockchainEvent) {
        match event {
            BlockchainEvent::Extended(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::Finalized(ref hash) => {
                // The on_blockchain_extended is necessary for the order of events to not matter.
                self.on_blockchain_extended(hash);
            }
            BlockchainEvent::EpochFinalized(ref hash) => {
                self.init_epoch();
                // The on_blockchain_extended is necessary for the order of events to not matter.
                self.on_blockchain_extended(hash);
            }
            BlockchainEvent::Rebranched(ref old_chain, ref new_chain) => {
                self.on_blockchain_rebranched(old_chain, new_chain)
            }
            BlockchainEvent::HistoryAdopted(_) | BlockchainEvent::Stored(_) => {
                // Nothing to do here for now. Forks are already reported on `fork_event_rx`
                // and inferior chain blocks are irrelevant here.
            }
        }
    }

    fn on_blockchain_extended(&mut self, hash: &Blake2bHash) {
        let block = self
            .blockchain
            .read()
            .get_block(hash, true, None)
            .expect("Head block not found");

        // Update mempool and blockchain state
        self.state
            .write()
            .consensus
            .equivocation_proofs
            .apply_block(&block);

        self.check_reactivate(block.block_number());
        self.init_block_producer(Some(hash));
    }

    fn on_blockchain_rebranched(
        &mut self,
        old_chain: &[(Blake2bHash, Block)],
        new_chain: &[(Blake2bHash, Block)],
    ) {
        // Update mempool and blockchain state
        let mut state = self.state.write();
        for (_hash, block) in old_chain.iter() {
            state.consensus.equivocation_proofs.revert_block(block);
        }
        for (_hash, block) in new_chain.iter() {
            state.consensus.equivocation_proofs.apply_block(block);
        }
        drop(state);

        let head_hash = &new_chain.last().expect("new_chain must not be empty").0;
        self.init_block_producer(Some(head_hash));
    }

    fn on_fork_event(&mut self, event: ForkEvent) {
        match event {
            ForkEvent::Detected(fork_proof) => self.on_equivocation_proof(fork_proof.into()),
        }
    }

    fn on_equivocation_proof(&mut self, proof: EquivocationProof) {
        // Keep the lock until the proof is added to the proof pool.
        let blockchain = self.blockchain.read();
        if blockchain
            .history_store
            .has_equivocation_proof(proof.locator(), None)
        {
            return;
        }
        self.state
            .write()
            .consensus
            .equivocation_proofs
            .insert(proof);
    }

    fn poll_macro(&mut self, cx: &mut Context<'_>) {
        while let Poll::Ready(Some(event)) =
            self.macro_producer.as_mut().unwrap().poll_next_unpin(cx)
        {
            match event {
                MappedReturn::ProposalAccepted(proposal) => {
                    if let Some(id) = proposal.message.proposal.1 {
                        self.network
                            .validate_message::<ProposalTopic<TValidatorNetwork>>(
                                id,
                                MsgAcceptance::Accept,
                            );
                    }
                }
                MappedReturn::ProposalIgnored(proposal) => {
                    if let Some(id) = proposal.message.proposal.1 {
                        self.network
                            .validate_message::<ProposalTopic<TValidatorNetwork>>(
                                id,
                                MsgAcceptance::Ignore,
                            );
                    }
                }
                MappedReturn::ProposalRejected(proposal) => {
                    if let Some(id) = proposal.message.proposal.1 {
                        self.network
                            .validate_message::<ProposalTopic<TValidatorNetwork>>(
                                id,
                                MsgAcceptance::Reject,
                            );
                    }
                }
                MappedReturn::Decision(block) => {
                    trace!("Tendermint returned block {}", block);

                    // Cache the hash for this block.
                    let mut block = Block::Macro(block);
                    block.hash_cached();

                    // Publish the block. It is valid as we have just created it.
                    Self::publish_block(Arc::clone(&self.network), block.clone());

                    // Use a trusted push since these blocks were generated by this validator
                    if cfg!(feature = "trusted_push") {
                        Blockchain::trusted_push(
                            self.blockchain.upgradable_read(),
                            block,
                            // No hook required as we are the producer.
                            &(),
                        )
                    } else {
                        Blockchain::push_with_hook(
                            self.blockchain.upgradable_read(),
                            block,
                            // No hook required as we are the producer.
                            &(),
                        )
                    }
                    .map_err(|e| error!("Failed to push macro block onto the chain: {:?}", e))
                    .ok();
                }

                // In case of a new state update we need to store the new version of it disregarding
                // any old state which potentially still lingers.
                MappedReturn::Update(update) => {
                    debug!(?update, "Tendermint state update");

                    let expected_block_number = self.blockchain.read().block_number() + 1;
                    if expected_block_number != update.block_number {
                        debug!(
                            expected_block_number,
                            update_block_number = update.block_number,
                            "Discarding obsolete Tendermint state update"
                        );
                        continue;
                    }

                    let mut write_transaction = self.env.write_transaction();
                    write_transaction.put(&self.table, &(), &update);
                    write_transaction.commit();

                    *self.macro_state.write() = Some(update);
                }
            }
        }
    }

    fn poll_micro(&mut self, cx: &mut Context<'_>) {
        while let Poll::Ready(Some(_event)) =
            self.micro_producer.as_mut().unwrap().poll_next_unpin(cx)
        {}
    }

    /// Publish our own validator record to the DHT.
    fn publish_dht(&self) {
        let state = self.state.read();
        let key_pair = state.signing_key.clone();
        let validator_address = state.validator_address.clone();
        let network = Arc::clone(&self.network);

        spawn(async move {
            if let Err(err) = network.set_public_key(&validator_address, &key_pair).await {
                error!("could not set up DHT record: {:?}", err);
            }
        });
    }

    /// Checks whether the validator fulfills the conditions for producing valid blocks.
    /// This includes having consensus, being able to extend the history tree and to enforce transaction validity.
    fn is_synced(&self) -> bool {
        self.consensus.is_ready_for_validation()
    }

    fn reactivate(&self, blockchain: &Blockchain) -> InactivityState {
        let validity_start_height = blockchain.block_number();

        let reactivate_transaction = {
            let state = self.state.read();
            TransactionBuilder::new_reactivate_validator(
                &state.fee_key,
                state.validator_address.clone(),
                &state.signing_key,
                Coin::ZERO,
                validity_start_height,
                blockchain.network_id(),
            )
        };
        let tx_hash = reactivate_transaction.hash();

        let cn = self.consensus.clone();
        spawn(async move {
            debug!("Sending reactivate transaction to the network");
            if cn
                .send_transaction(reactivate_transaction.clone())
                .await
                .is_err()
            {
                error!("Failed to send reactivate transaction");
            }
        });

        InactivityState {
            inactive_tx_hash: tx_hash,
            inactive_tx_validity_window_start: validity_start_height,
        }
    }

    pub fn state(&self) -> &Arc<RwLock<ValidatorState>> {
        &self.state
    }

    /// Returns a handle to the cached macro block state. This is the same state
    /// that is served in response to [`RequestProposal`] requests.
    ///
    /// Only available when the `test-accessors` feature is enabled, as it exposes
    /// internal state that should not be reachable in production builds.
    #[cfg(feature = "test-accessors")]
    pub fn macro_state(&self) -> &Arc<RwLock<Option<MacroState>>> {
        &self.macro_state
    }

    #[cfg(feature = "metrics")]
    pub fn get_mempool_monitor(&self) -> TaskMonitor {
        self.mempool_task.get_mempool_monitor()
    }

    #[cfg(feature = "metrics")]
    pub fn get_control_mempool_monitor(&self) -> TaskMonitor {
        self.mempool_task.get_control_mempool_monitor()
    }

    /// Publishes the given block on both the BlockHeaderTopic and BlockBodyTopic.
    pub fn publish_block(network: Arc<TValidatorNetwork>, block: Block) {
        if block.is_election() {
            info!(%block, "Publishing Election MacroBlock");
        } else if block.is_macro() {
            info!(%block, "Publishing Checkpoint MacroBlock");
        } else {
            info!(%block, "Publishing MicroBlock");
        }

        spawn(async move {
            let block_id = format!("{block}");

            let (header, body) = BlockHeaderMessage::split_block(block);

            if let Err(e) = network.publish::<BlockHeaderTopic>(header).await {
                trace!(
                    block = block_id,
                    error = &e as &dyn Error,
                    "Failed to publish block header"
                );
            }

            if let Err(e) = network.publish::<BlockBodyTopic>(body).await {
                trace!(
                    block = block_id,
                    error = &e as &dyn Error,
                    "Failed to publish block body"
                );
            }
        });
    }
}

impl<TValidatorNetwork: ValidatorNetwork> Future for Validator<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Process consensus updates.
        while let Poll::Ready(Some(event)) = self.consensus_event_rx.poll_next_unpin(cx) {
            match event {
                Ok(ConsensusEvent::Established {
                    synced_validity_window: true,
                }) => self.init(None),
                Ok(ConsensusEvent::Lost)
                | Ok(ConsensusEvent::Established {
                    synced_validity_window: false,
                }) => self.pause(),
                Err(_) => return Poll::Ready(()),
            }
        }

        // Process blockchain updates.
        while let Poll::Ready(Some(event)) = self.mempool_task.poll_next_unpin(cx) {
            self.on_blockchain_event(event.into());
        }

        // Process fork events.
        // We can already start with processing fork events before we can be active.
        while let Poll::Ready(Some(Ok(event))) = self.fork_event_rx.poll_next_unpin(cx) {
            let consensus_established = self.consensus.is_established();
            trace!(?event, consensus_established, "fork event");
            if consensus_established {
                self.on_fork_event(event);
            }
        }

        // If we are an active validator, participate in block production.
        if self.is_synced() && self.state.read().is_elected() {
            if self.macro_producer.is_some() {
                self.poll_macro(cx);
            }
            if self.micro_producer.is_some() {
                self.poll_micro(cx);
            }
        }

        // Once the validator can be active is established, check the validator staking state.
        if self.is_synced() {
            let blockchain = self.blockchain.read();
            let state = self.state.read();
            match state.get_staking_state(&blockchain) {
                ValidatorStakingState::Active => {
                    drop(state);
                    drop(blockchain);
                    if self.inactivity_state.is_some() {
                        self.inactivity_state = None;
                        info!("Automatically reactivated.");
                    }
                }
                ValidatorStakingState::Inactive(jailed_from) => {
                    drop(state);
                    if self.inactivity_state.is_none()
                        && jailed_from
                            .map(|jailed_from| {
                                blockchain.block_number() >= Policy::block_after_jail(jailed_from)
                            })
                            .unwrap_or(true)
                        && self.state.read().automatic_reactivate
                    {
                        let inactivity_state = self.reactivate(&blockchain);
                        drop(blockchain);
                        self.inactivity_state = Some(inactivity_state);
                    }
                }
                ValidatorStakingState::UnknownOrNoStake => {}
            }
        }

        // Check if DHT is bootstrapped and we can publish our record
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::DhtReady) => {
                    self.publish_dht();
                }
                Ok(_) => {}
                Err(e) => error!("{}", e),
            }
        }

        Poll::Pending
    }
}
