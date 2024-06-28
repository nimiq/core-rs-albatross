use std::{
    error::Error,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};

use futures::stream::StreamExt;
use nimiq_block::{Block, BlockHeaderTopic, BlockTopic, BlockType, EquivocationProof};
use nimiq_blockchain::{interface::HistoryInterface, BlockProducer, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, ForkEvent, PushResult};
use nimiq_bls::{lazy::LazyPublicKey, KeyPair as BlsKeyPair};
use nimiq_consensus::{Consensus, ConsensusEvent, ConsensusProxy};
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
use nimiq_utils::spawn::spawn;
use nimiq_validator_network::{PubsubId, ValidatorNetwork};
use parking_lot::RwLock;
#[cfg(feature = "metrics")]
use tokio_metrics::TaskMonitor;
use tokio_stream::wrappers::BroadcastStream;

use crate::{
    aggregation::tendermint::{proposal::RequestProposal, state::MacroState},
    jail::EquivocationProofPool,
    micro::{ProduceMicroBlock, ProduceMicroBlockEvent},
    proposal_buffer::{ProposalBuffer, ProposalReceiver},
    r#macro::{MappedReturn, ProduceMacroBlock, ProposalTopic},
};

#[derive(PartialEq)]
enum ValidatorStakingState {
    Active,
    Inactive(Option<u32>),
    NoStake,
    Unknown,
}

pub struct ConsensusState {
    equivocation_proofs: EquivocationProofPool,
}

/// Validator inactivity
struct InactivityState {
    inactive_tx_hash: Blake2bHash,
    inactive_tx_validity_window_start: u32,
}

pub struct ValidatorProxy {
    pub validator_address: Arc<RwLock<Address>>,
    pub signing_key: Arc<RwLock<SchnorrKeyPair>>,
    pub voting_key: Arc<RwLock<BlsKeyPair>>,
    pub fee_key: Arc<RwLock<SchnorrKeyPair>>,
    pub automatic_reactivate: Arc<AtomicBool>,
    pub slot_band: Arc<RwLock<Option<u16>>>,
    pub consensus_state: Arc<RwLock<ConsensusState>>,
}

impl Clone for ValidatorProxy {
    fn clone(&self) -> Self {
        Self {
            validator_address: Arc::clone(&self.validator_address),
            signing_key: Arc::clone(&self.signing_key),
            voting_key: Arc::clone(&self.voting_key),
            fee_key: Arc::clone(&self.fee_key),
            automatic_reactivate: Arc::clone(&self.automatic_reactivate),
            slot_band: Arc::clone(&self.slot_band),
            consensus_state: Arc::clone(&self.consensus_state),
        }
    }
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

    validator_address: Arc<RwLock<Address>>,
    signing_key: Arc<RwLock<SchnorrKeyPair>>,
    voting_key: Arc<RwLock<BlsKeyPair>>,
    fee_key: Arc<RwLock<SchnorrKeyPair>>,

    proposal_receiver: ProposalReceiver<TValidatorNetwork>,

    consensus_event_rx: BroadcastStream<ConsensusEvent>,
    network_event_rx: SubscribeEvents<<TValidatorNetwork::NetworkType as Network>::PeerId>,
    fork_event_rx: BroadcastStream<ForkEvent>,

    slot_band: Arc<RwLock<Option<u16>>>,
    consensus_state: Arc<RwLock<ConsensusState>>,
    validator_state: Option<InactivityState>,
    automatic_reactivate: Arc<AtomicBool>,

    macro_producer: Option<ProduceMacroBlock<TValidatorNetwork>>,
    macro_state: Arc<RwLock<Option<MacroState>>>,

    micro_producer: Option<ProduceMicroBlock<TValidatorNetwork>>,

    pub mempool_task: MempoolTask<TValidatorNetwork::NetworkType>,
}

impl<TValidatorNetwork: ValidatorNetwork> Validator<TValidatorNetwork>
where
    PubsubId<TValidatorNetwork>: std::fmt::Debug + Unpin,
{
    const PRODUCER_TIMEOUT: Duration = Duration::from_millis(Policy::BLOCK_PRODUCER_TIMEOUT);
    const BLOCK_SEPARATION_TIME: Duration = Duration::from_millis(Policy::BLOCK_SEPARATION_TIME);
    const EQUIVOCATION_PROOFS_MAX_SIZE: usize = 1_000; // bytes

    pub fn new(
        env: MdbxDatabase,
        consensus: &Consensus<TValidatorNetwork::NetworkType>,
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TValidatorNetwork>,
        validator_address: Address,
        automatic_reactivate: bool,
        signing_key: SchnorrKeyPair,
        voting_key: BlsKeyPair,
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

        let automatic_reactivate = Arc::new(AtomicBool::new(automatic_reactivate));

        Self::init_network_request_receivers(&consensus.network, &macro_state);

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

            validator_address: Arc::new(RwLock::new(validator_address)),
            signing_key: Arc::new(RwLock::new(signing_key)),
            voting_key: Arc::new(RwLock::new(voting_key)),
            fee_key: Arc::new(RwLock::new(fee_key)),

            proposal_receiver,

            consensus_event_rx,
            network_event_rx,
            fork_event_rx,

            slot_band: Arc::new(RwLock::new(None)),
            consensus_state: Arc::new(RwLock::new(blockchain_state)),
            validator_state: None,
            automatic_reactivate,

            macro_producer: None,
            macro_state: Arc::clone(&macro_state),

            micro_producer: None,

            mempool_task: mempool,
        }
    }

    fn init_network_request_receivers(
        network: &Arc<TValidatorNetwork::NetworkType>,
        macro_state: &Arc<RwLock<Option<MacroState>>>,
    ) {
        let stream = network.receive_requests::<RequestProposal>();
        spawn(Box::pin(request_handler(network, stream, macro_state)));
    }

    fn init(&mut self, head_hash: Option<&Blake2bHash>) {
        self.init_epoch();
        self.init_block_producer(head_hash);
    }

    fn check_reactivate(&mut self, block_number: u32) {
        // Check if the reactivate/activate transaction was sent
        if let Some(validator_state) = &self.validator_state {
            // We check this in the last possible block of the validity window
            let tx_validity_window_start = validator_state.inactive_tx_validity_window_start;
            if block_number
                == tx_validity_window_start + Policy::transaction_validity_window_blocks() - 1
            {
                let blockchain = self.blockchain.read();
                let staking_state = self.get_staking_state(&blockchain);
                // Check that the transaction was sent in the validity window
                if (matches!(staking_state, ValidatorStakingState::Inactive(..)))
                    && !blockchain.history_store.tx_in_validity_window(
                        &validator_state.inactive_tx_hash.clone().into(),
                        None,
                    )
                {
                    // If we are inactive and no transaction has been seen in the expected validity window
                    // after an epoch, reset our inactive state
                    log::debug!("Resetting state to re-send reactivate transactions since we are inactive and validity window doesn't contain the transaction sent");
                    self.validator_state = None;
                }
            }
        }
    }

    fn init_epoch(&mut self) {
        *self.slot_band.write() = None;

        if !self.is_synced() {
            return;
        }

        let blockchain = self.blockchain.read();
        let validators = blockchain.current_validators().unwrap();

        *self.slot_band.write() = validators.get_slot_band_by_address(&self.validator_address());

        if let Some(slot_band) = *self.slot_band.read() {
            log::info!(
                validator_address = %self.validator_address(),
                validator_slot_band = slot_band,
                epoch_number = blockchain.epoch_number(),
                "We are ACTIVE in this epoch"
            );
        } else {
            log::debug!(
                validator_address = %self.validator_address(),
                epoch_number = blockchain.epoch_number(),
                "We are INACTIVE in this epoch"
            );
        }

        // Inform the network about the current validator ID.
        self.network.set_validator_id(*self.slot_band.read());

        let voting_keys: Vec<LazyPublicKey> = validators
            .iter()
            .map(|validator| validator.voting_key.clone())
            .collect();
        let network = Arc::clone(&self.network);

        // TODO might better be done without the task.
        spawn(async move {
            network.set_validators(voting_keys).await;
        });
    }

    fn init_block_producer(&mut self, head_hash: Option<&Blake2bHash>) {
        self.macro_producer = None;
        self.micro_producer = None;

        if !self.is_elected() || !self.is_synced() {
            return;
        }

        let blockchain = self.blockchain.read();

        if let Some(hash) = head_hash {
            if blockchain.head_hash() != *hash {
                log::debug!("Bypassed initializing block producer for obsolete block");
                return;
            }
        }

        let head = blockchain.head();
        let next_block_number = head.block_number() + 1;
        let network_id = head.network();
        let block_producer = BlockProducer::new(self.signing_key(), self.voting_key());

        debug!(
            next_block_number = next_block_number,
            "Initializing block producer"
        );

        match BlockType::of(next_block_number) {
            BlockType::Macro => {
                let active_validators = blockchain.current_validators().unwrap();
                let proposal_stream = self.proposal_receiver.clone().boxed();

                drop(blockchain);

                self.macro_producer = Some(ProduceMacroBlock::new(
                    Arc::clone(&self.blockchain),
                    Arc::clone(&self.network),
                    block_producer,
                    self.validator_slot_band(),
                    active_validators,
                    network_id,
                    next_block_number,
                    self.macro_state.read().clone(),
                    proposal_stream,
                ));
            }
            BlockType::Micro => {
                let equivocation_proofs = self
                    .consensus_state
                    .read()
                    .equivocation_proofs
                    .get_equivocation_proofs_for_block(Self::EQUIVOCATION_PROOFS_MAX_SIZE);
                let prev_seed = head.seed().clone();

                drop(blockchain);

                self.micro_producer = Some(ProduceMicroBlock::new(
                    Arc::clone(&self.blockchain),
                    Arc::clone(&self.mempool_task.mempool),
                    Arc::clone(&self.network),
                    block_producer,
                    self.validator_slot_band(),
                    equivocation_proofs,
                    prev_seed,
                    next_block_number,
                    Self::PRODUCER_TIMEOUT,
                    Self::BLOCK_SEPARATION_TIME,
                ));
            }
        }
    }

    fn pause(&mut self) {
        *self.slot_band.write() = None;
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
            .consensus
            .blockchain
            .read()
            .get_block(hash, true)
            .expect("Head block not found");

        // Update mempool and blockchain state
        self.consensus_state
            .write()
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
        let mut consensus_state = self.consensus_state.write();
        for (_hash, block) in old_chain.iter() {
            consensus_state.equivocation_proofs.revert_block(block);
        }
        for (_hash, block) in new_chain.iter() {
            consensus_state.equivocation_proofs.apply_block(block);
        }
        drop(consensus_state);

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
        self.consensus_state
            .write()
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

                    // If the event is a result meaning the next macro block was produced we push it onto our local chain
                    let block_copy = block.clone();

                    // Use a trusted push since these blocks were generated by this validator
                    let result = if cfg!(feature = "trusted_push") {
                        Blockchain::trusted_push(
                            self.blockchain.upgradable_read(),
                            Block::Macro(block),
                        )
                        .map_err(|e| error!("Failed to push macro block onto the chain: {:?}", e))
                        .ok()
                    } else {
                        Blockchain::push(self.blockchain.upgradable_read(), Block::Macro(block))
                            .map_err(|e| {
                                error!("Failed to push macro block onto the chain: {:?}", e)
                            })
                            .ok()
                    };

                    if result == Some(PushResult::Extended)
                        || result == Some(PushResult::Rebranched)
                    {
                        if block_copy.is_election() {
                            info!(
                                block_number = &block_copy.header.block_number,
                                "Publishing Election MacroBlock"
                            );
                        } else {
                            info!(
                                block_number = &block_copy.header.block_number,
                                "Publishing Checkpoint MacroBlock"
                            );
                        }

                        self.publish_block(Block::Macro(block_copy));
                    }
                }

                // In case of a new state update we need to store the new version of it disregarding
                // any old state which potentially still lingers.
                MappedReturn::Update(update) => {
                    trace!(?update, "Tendermint state update");

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
        while let Poll::Ready(Some(event)) =
            self.micro_producer.as_mut().unwrap().poll_next_unpin(cx)
        {
            match event {
                ProduceMicroBlockEvent::MicroBlock(block, result) => {
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        self.publish_block(Block::Micro(block));
                    }
                }
            }
        }
    }

    /// Publish our own validator record to the DHT.
    fn publish_dht(&self) {
        let key = self.voting_key();
        let network = Arc::clone(&self.network);

        spawn(async move {
            if let Err(err) = network
                .set_public_key(&key.public_key.compress(), &key.secret_key)
                .await
            {
                error!("could not set up DHT record: {:?}", err);
            }
        });
    }

    /// Publish a block via gossipsub.
    fn publish_block(&self, mut block: Block) {
        trace!(%block, "Publishing block");

        let network = Arc::clone(&self.network);
        spawn(async move {
            let block_id = format!("{}", block);

            if let Err(e) = network.publish::<BlockTopic>(block.clone()).await {
                debug!(
                    block = block_id,
                    error = &e as &dyn Error,
                    "Failed to publish block"
                );
            }

            // Remove body from micro blocks before publishing to the block header topic.
            // Macro blocks must be always sent with body.
            match block {
                Block::Micro(ref mut micro_block) => micro_block.body = None,
                Block::Macro(_) => {}
            }

            if let Err(e) = network.publish::<BlockHeaderTopic>(block).await {
                debug!(
                    block = block_id,
                    error = &e as &dyn Error,
                    "Failed to publish block header"
                );
            }
        });
    }

    /// Checks whether we are an elected validator in the current epoch.
    fn is_elected(&self) -> bool {
        self.slot_band.read().is_some()
    }

    /// Checks whether the validator fulfills the conditions for producing valid blocks.
    /// This includes having consensus, being able to extend the history tree and to enforce transaction validity.
    fn is_synced(&self) -> bool {
        self.consensus.is_ready_for_validation()
    }

    fn get_staking_state(&self, blockchain: &Blockchain) -> ValidatorStakingState {
        let validator_address = self.validator_address();
        let staking_contract = match blockchain.get_staking_contract_if_complete(None) {
            Some(contract) => contract,
            None => return ValidatorStakingState::Unknown,
        };

        // Then fetch the validator to see if it is active.
        let data_store = blockchain.get_staking_contract_store();
        let txn = blockchain.read_transaction();
        staking_contract
            .get_validator(&data_store.read(&txn), &validator_address)
            .map_or(
                ValidatorStakingState::NoStake,
                |validator| match validator.inactive_from {
                    Some(_) => ValidatorStakingState::Inactive(validator.jailed_from),
                    None => ValidatorStakingState::Active,
                },
            )
    }

    fn reactivate(&self, blockchain: &Blockchain) -> InactivityState {
        let validity_start_height = blockchain.block_number();

        let reactivate_transaction = TransactionBuilder::new_reactivate_validator(
            &self.fee_key(),
            self.validator_address(),
            &self.signing_key(),
            Coin::ZERO,
            validity_start_height,
            blockchain.network_id(),
        );
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

    pub fn validator_slot_band(&self) -> u16 {
        self.slot_band.read().expect("Validator not elected")
    }

    pub fn validator_address(&self) -> Address {
        self.validator_address.read().clone()
    }

    pub fn voting_key(&self) -> BlsKeyPair {
        self.voting_key.read().clone()
    }

    pub fn signing_key(&self) -> SchnorrKeyPair {
        self.signing_key.read().clone()
    }

    pub fn fee_key(&self) -> SchnorrKeyPair {
        self.fee_key.read().clone()
    }

    pub fn proxy(&self) -> ValidatorProxy {
        ValidatorProxy {
            validator_address: Arc::clone(&self.validator_address),
            signing_key: Arc::clone(&self.signing_key),
            voting_key: Arc::clone(&self.voting_key),
            fee_key: Arc::clone(&self.fee_key),
            automatic_reactivate: Arc::clone(&self.automatic_reactivate),
            slot_band: Arc::clone(&self.slot_band),
            consensus_state: Arc::clone(&self.consensus_state),
        }
    }

    #[cfg(feature = "metrics")]
    pub fn get_mempool_monitor(&self) -> TaskMonitor {
        self.mempool_task.get_mempool_monitor()
    }

    #[cfg(feature = "metrics")]
    pub fn get_control_mempool_monitor(&self) -> TaskMonitor {
        self.mempool_task.get_control_mempool_monitor()
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
            trace!(
                ?event,
                is_synced = self.is_synced(),
                "mempool/blockchain event"
            );
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
        if self.is_synced() && self.is_elected() {
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
            match self.get_staking_state(&blockchain) {
                ValidatorStakingState::Active => {
                    drop(blockchain);
                    if self.validator_state.is_some() {
                        self.validator_state = None;
                        info!("Automatically reactivated.");
                    }
                }
                ValidatorStakingState::Inactive(jailed_from) => {
                    if self.validator_state.is_none()
                        && jailed_from
                            .map(|jailed_from| {
                                blockchain.block_number() >= Policy::block_after_jail(jailed_from)
                            })
                            .unwrap_or(true)
                        && self.automatic_reactivate.load(Ordering::Acquire)
                    {
                        let inactivity_state = self.reactivate(&blockchain);
                        drop(blockchain);
                        self.validator_state = Some(inactivity_state);
                    }
                }
                ValidatorStakingState::NoStake | ValidatorStakingState::Unknown => {}
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
