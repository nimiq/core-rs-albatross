use std::{
    error::Error,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::stream::{BoxStream, Stream, StreamExt};
use linked_hash_map::LinkedHashMap;
use nimiq_block::{Block, BlockHeaderTopic, BlockTopic, BlockType, EquivocationProof};
use nimiq_blockchain::{BlockProducer, Blockchain};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, ForkEvent, PushResult};
use nimiq_bls::{lazy::LazyPublicKey, KeyPair as BlsKeyPair};
use nimiq_consensus::{Consensus, ConsensusEvent, ConsensusProxy};
use nimiq_database::{
    traits::{Database, ReadTransaction, WriteTransaction},
    DatabaseProxy, TableProxy,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, Signature as SchnorrSignature};
use nimiq_macros::store_waker;
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool};
use nimiq_network_interface::{
    network::{MsgAcceptance, Network, NetworkEvent, PubsubId, SubscribeEvents, Topic},
    request::request_handler,
};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_tendermint::SignedProposalMessage;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_validator_network::ValidatorNetwork;
use parking_lot::RwLock;
#[cfg(feature = "metrics")]
use tokio_metrics::TaskMonitor;
use tokio_stream::wrappers::BroadcastStream;

use crate::{
    aggregation::tendermint::{
        proposal::{Header, RequestProposal},
        state::MacroState,
    },
    jail::EquivocationProofPool,
    micro::{ProduceMicroBlock, ProduceMicroBlockEvent},
    r#macro::{MappedReturn, ProduceMacroBlock, ProposalTopic},
};

#[derive(PartialEq)]
enum ValidatorStakingState {
    Active,
    Inactive(Option<u32>),
    NoStake,
    Unknown,
}

struct ActiveEpochState {
    validator_slot_band: u16,
}

struct BlockchainState {
    equivocation_proofs: EquivocationProofPool,
    can_enforce_validity_window: bool,
}

/// Validator inactivity
struct InactivityState {
    inactive_tx_hash: Blake2bHash,
    inactive_tx_validity_window_start: u32,
}

enum MempoolState {
    Active,
    Inactive,
}

pub struct ValidatorProxy {
    pub validator_address: Arc<RwLock<Address>>,
    pub signing_key: Arc<RwLock<SchnorrKeyPair>>,
    pub voting_key: Arc<RwLock<BlsKeyPair>>,
    pub fee_key: Arc<RwLock<SchnorrKeyPair>>,
    pub automatic_reactivate: Arc<AtomicBool>,
}

impl Clone for ValidatorProxy {
    fn clone(&self) -> Self {
        Self {
            validator_address: Arc::clone(&self.validator_address),
            signing_key: Arc::clone(&self.signing_key),
            voting_key: Arc::clone(&self.voting_key),
            fee_key: Arc::clone(&self.fee_key),
            automatic_reactivate: Arc::clone(&self.automatic_reactivate),
        }
    }
}

pub struct Validator<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    pub consensus: ConsensusProxy<TValidatorNetwork::NetworkType>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    network: Arc<TValidatorNetwork>,
    network_event_rx: SubscribeEvents<<TValidatorNetwork::NetworkType as Network>::PeerId>,

    database: TableProxy,
    env: DatabaseProxy,

    validator_address: Arc<RwLock<Address>>,
    signing_key: Arc<RwLock<SchnorrKeyPair>>,
    voting_key: Arc<RwLock<BlsKeyPair>>,
    fee_key: Arc<RwLock<SchnorrKeyPair>>,

    proposal_receiver: ProposalReceiver<TValidatorNetwork>,

    consensus_event_rx: BroadcastStream<ConsensusEvent>,
    blockchain_event_rx: BoxStream<'static, BlockchainEvent>,
    fork_event_rx: BroadcastStream<ForkEvent>,

    epoch_state: Option<ActiveEpochState>,
    blockchain_state: BlockchainState,
    validator_state: Option<InactivityState>,
    automatic_reactivate: Arc<AtomicBool>,

    macro_producer: Option<ProduceMacroBlock<TValidatorNetwork>>,
    macro_state: Arc<RwLock<Option<MacroState>>>,

    micro_producer: Option<ProduceMicroBlock<TValidatorNetwork>>,

    pub mempool: Arc<Mempool>,
    mempool_state: MempoolState,
    #[cfg(feature = "metrics")]
    mempool_monitor: TaskMonitor,
    #[cfg(feature = "metrics")]
    control_mempool_monitor: TaskMonitor,
}

impl<TValidatorNetwork: ValidatorNetwork> Validator<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    const MACRO_STATE_DB_NAME: &'static str = "ValidatorState";
    const MACRO_STATE_KEY: &'static str = "validatorState";
    const PRODUCER_TIMEOUT: Duration = Duration::from_millis(Policy::BLOCK_PRODUCER_TIMEOUT);
    const BLOCK_SEPARATION_TIME: Duration = Duration::from_millis(Policy::BLOCK_SEPARATION_TIME);
    const EQUIVOCATION_PROOFS_MAX_SIZE: usize = 1_000; // bytes

    pub fn new(
        env: DatabaseProxy,
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
        let blockchain_event_rx = blockchain_rg.notifier_as_stream();
        let fork_event_rx = BroadcastStream::new(blockchain_rg.fork_notifier.subscribe());
        drop(blockchain_rg);

        let blockchain_state = BlockchainState {
            equivocation_proofs: EquivocationProofPool::new(),
            can_enforce_validity_window: false,
        };

        let database = env.open_table(Self::MACRO_STATE_DB_NAME.to_string());

        let macro_state: Option<MacroState> = {
            let read_transaction = env.read_transaction();
            read_transaction.get(&database, Self::MACRO_STATE_KEY)
        };
        let macro_state = Arc::new(RwLock::new(macro_state));

        let network1 = Arc::clone(&network);
        let (proposal_sender, proposal_receiver) = ProposalBuffer::new();

        let mempool = Arc::new(Mempool::new(Arc::clone(&blockchain), mempool_config));
        let mempool_state = MempoolState::Inactive;

        let automatic_reactivate = Arc::new(AtomicBool::new(automatic_reactivate));

        let mut this = Self {
            consensus: consensus.proxy(),
            blockchain,
            network,
            network_event_rx: network1.subscribe_events(),

            database,
            env,

            validator_address: Arc::new(RwLock::new(validator_address)),
            signing_key: Arc::new(RwLock::new(signing_key)),
            voting_key: Arc::new(RwLock::new(voting_key)),
            fee_key: Arc::new(RwLock::new(fee_key)),

            proposal_receiver,

            consensus_event_rx,
            blockchain_event_rx,
            fork_event_rx,

            epoch_state: None,
            blockchain_state,
            validator_state: None,
            automatic_reactivate,

            macro_producer: None,
            macro_state: Arc::clone(&macro_state),

            micro_producer: None,

            mempool: Arc::clone(&mempool),
            mempool_state,
            #[cfg(feature = "metrics")]
            mempool_monitor: TaskMonitor::new(),
            #[cfg(feature = "metrics")]
            control_mempool_monitor: TaskMonitor::new(),
        };

        this.init();

        tokio::spawn(async move {
            network1
                .subscribe::<ProposalTopic<TValidatorNetwork>>()
                .await
                .expect("Failed to subscribe to proposal topic")
                .for_each(|proposal| async { proposal_sender.send(proposal) })
                .await
        });

        Self::init_network_request_receivers(&this.consensus.network, &macro_state);

        this
    }

    #[cfg(feature = "metrics")]
    pub fn get_mempool_monitor(&self) -> TaskMonitor {
        self.mempool_monitor.clone()
    }

    #[cfg(feature = "metrics")]
    pub fn get_control_mempool_monitor(&self) -> TaskMonitor {
        self.control_mempool_monitor.clone()
    }

    fn init_network_request_receivers(
        network: &Arc<TValidatorNetwork::NetworkType>,
        macro_state: &Arc<RwLock<Option<MacroState>>>,
    ) {
        let stream = network.receive_requests::<RequestProposal>();
        tokio::spawn(Box::pin(request_handler(network, stream, macro_state)));
    }

    fn init(&mut self) {
        self.init_epoch();
        self.init_block_producer(None);
    }

    fn init_epoch(&mut self) {
        // Clear producers here, as this validator might not be active anymore.
        self.macro_producer = None;
        self.micro_producer = None;

        let blockchain = self.blockchain.read();

        // Check if the unpark/activate transaction was sent
        if let Some(validator_state) = &self.validator_state {
            let tx_validity_window_start = validator_state.inactive_tx_validity_window_start;
            // Check that the transaction was sent in the validity window
            let staking_state = self.get_staking_state(&blockchain);
            if (matches!(staking_state, ValidatorStakingState::Inactive(..)))
                && blockchain.block_number()
                    >= tx_validity_window_start + Policy::blocks_per_epoch()
                && !blockchain.tx_in_validity_window(
                    &validator_state.inactive_tx_hash,
                    tx_validity_window_start,
                    None,
                )
            {
                // If we are inactive and no transaction has been seen in the expected validity window
                // after an epoch, reset our inactive state
                log::debug!("Resetting state to re-send reactivate transactions since we are inactive and validity window doesn't contain the transaction sent");
                self.validator_state = None;
            }
        }

        let validators = blockchain.current_validators().unwrap();

        self.epoch_state = None;

        for (i, validator) in validators.iter().enumerate() {
            if validator.address == self.validator_address() {
                log::info!(
                    validator_address = %validator.address,
                    validator_slot_band = i,
                    epoch_number = blockchain.epoch_number(),
                    "We are ACTIVE in this epoch"
                );

                self.epoch_state = Some(ActiveEpochState {
                    validator_slot_band: i as u16,
                });
                break;
            }
        }

        if self.epoch_state.is_none() {
            log::debug!(
                validator_address = %self.validator_address(),
                epoch_number = blockchain.epoch_number(),
                "We are INACTIVE in this epoch"
            );
        }

        let voting_keys: Vec<LazyPublicKey> = validators
            .iter()
            .map(|validator| validator.voting_key.clone())
            .collect();
        let network = Arc::clone(&self.network);

        // TODO might better be done without the task.
        tokio::spawn(async move {
            network.set_validators(voting_keys).await;
        });
    }

    fn init_block_producer(&mut self, event: Option<Blake2bHash>) {
        if !self.is_active() {
            return;
        }

        let blockchain = self.blockchain.read();

        if let Some(event) = event {
            if blockchain.head_hash() != event {
                log::debug!("Bypassed initializing block producer for obsolete block.");
                self.micro_producer = None;
                self.macro_producer = None;
                return;
            }
        }

        let head = blockchain.head();
        let next_block_number = head.block_number() + 1;
        let block_producer = BlockProducer::new(self.signing_key(), self.voting_key());

        debug!(
            next_block_number = next_block_number,
            "Initializing block producer"
        );

        self.macro_producer = None;
        self.micro_producer = None;

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
                    next_block_number,
                    self.macro_state.read().clone(),
                    proposal_stream,
                ));
            }
            BlockType::Micro => {
                let equivocation_proofs = self
                    .blockchain_state
                    .equivocation_proofs
                    .get_equivocation_proofs_for_block(Self::EQUIVOCATION_PROOFS_MAX_SIZE);
                let prev_seed = head.seed().clone();

                drop(blockchain);

                self.micro_producer = Some(ProduceMicroBlock::new(
                    Arc::clone(&self.blockchain),
                    Arc::clone(&self.mempool),
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

    fn init_mempool(&mut self) {
        if let MempoolState::Inactive = self.mempool_state {
            let mempool = Arc::clone(&self.mempool);
            let network = Arc::clone(&self.consensus.network);
            #[cfg(not(feature = "metrics"))]
            tokio::spawn({
                async move {
                    // The mempool is not updated while consensus is lost.
                    // Thus, we need to check all transactions if they are still valid.
                    mempool.mempool_update_full();
                    mempool.start_executors(network, None, None).await;
                }
            });
            #[cfg(feature = "metrics")]
            tokio::spawn({
                let mempool_monitor = self.mempool_monitor.clone();
                let ctrl_mempool_monitor = self.control_mempool_monitor.clone();
                async move {
                    // The mempool is not updated while consensus is lost.
                    // Thus, we need to check all transactions if they are still valid.
                    mempool.mempool_update_full();

                    mempool
                        .start_executors(network, Some(mempool_monitor), Some(ctrl_mempool_monitor))
                        .await;
                }
            });
            self.mempool_state = MempoolState::Active;
        }
    }

    /// Publish our own entry to the DHT
    fn publish_dht(&self) {
        let key = self.voting_key();
        let network = Arc::clone(&self.network);

        tokio::spawn(async move {
            if let Err(err) = network
                .set_public_key(&key.public_key.compress(), &key.secret_key)
                .await
            {
                error!("could not set up DHT record: {:?}", err);
            }
        });
    }

    /// This function resets the validator state when consensus is lost.
    fn pause_validator(&mut self) {
        // When we lose consensus we may no longer be able to enforce the validity window.
        self.blockchain_state.can_enforce_validity_window = false;
    }

    /// Check and update if we can enforce the tx validity window.
    /// This is important if we use the state sync and do not have the relevant parts of the history yet.
    fn update_can_enforce_validity_window(&mut self) {
        let old_can_enforce_validity_window = self.blockchain_state.can_enforce_validity_window;
        self.blockchain_state.can_enforce_validity_window =
            self.blockchain.read().can_enforce_validity_window();

        // Re-initialize validator when flag returns to true.
        if !old_can_enforce_validity_window && self.blockchain_state.can_enforce_validity_window {
            self.init();
            self.init_mempool();
        }
    }

    fn on_blockchain_event(&mut self, event: BlockchainEvent) {
        match event {
            BlockchainEvent::Extended(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::HistoryAdopted(ref hash) => self.on_blockchain_history_adopted(hash),
            BlockchainEvent::Finalized(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::EpochFinalized(ref hash) => {
                self.on_blockchain_extended(hash);
                if self.can_be_active() {
                    self.init_epoch()
                }
            }
            BlockchainEvent::Rebranched(ref old_chain, ref new_chain) => {
                self.on_blockchain_rebranched(old_chain, new_chain)
            }
        }
    }

    fn on_blockchain_history_adopted(&mut self, _: &Blake2bHash) {
        // Mempool updates are only done once we can be active.
        if self.can_be_active() {
            self.mempool.mempool_clean_up();
            debug!("Performed a mempool clean up because new history was adopted");
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
        self.blockchain_state
            .equivocation_proofs
            .apply_block(&block);
        // Mempool updates are only done once we can be active.
        if self.can_be_active() {
            self.mempool
                .mempool_update(&vec![(hash.clone(), block)], [].as_ref());
        }
    }

    fn on_blockchain_rebranched(
        &mut self,
        old_chain: &[(Blake2bHash, Block)],
        new_chain: &[(Blake2bHash, Block)],
    ) {
        // Update mempool and blockchain state
        for (_hash, block) in old_chain.iter() {
            self.blockchain_state
                .equivocation_proofs
                .revert_block(block);
        }
        for (_hash, block) in new_chain.iter() {
            self.blockchain_state.equivocation_proofs.apply_block(block);
        }
        // Mempool updates are only done once we can be active.
        if self.can_be_active() {
            self.mempool.mempool_update(new_chain, old_chain);
        }
    }

    fn on_equivocation_proof(&mut self, proof: EquivocationProof) {
        // Keep the lock until the proof is added to the the proof pool.
        let blockchain = self.blockchain.read();
        if blockchain
            .history_store
            .has_equivocation_proof(proof.locator(), None)
        {
            return;
        }
        self.blockchain_state.equivocation_proofs.insert(proof);
        drop(blockchain);
    }

    fn on_fork_event(&mut self, event: ForkEvent) {
        match event {
            ForkEvent::Detected(fork_proof) => self.on_equivocation_proof(fork_proof.into()),
        }
    }

    fn poll_macro(&mut self, cx: &mut Context<'_>) {
        let macro_producer = self.macro_producer.as_mut().unwrap();
        while let Poll::Ready(Some(event)) = macro_producer.poll_next_unpin(cx) {
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
                        if block_copy.is_election_block() {
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

                        // todo get rid of spawn
                        let network = Arc::clone(&self.network);
                        tokio::spawn(async move {
                            Self::publish_block(network, Block::Macro(block_copy)).await;
                        });
                    }
                }

                // In case of a new state update we need to store the new version of it disregarding
                // any old state which potentially still lingers.
                MappedReturn::Update(update) => {
                    trace!(?update, "Tendermint state update",);
                    let mut write_transaction = self.env.write_transaction();
                    let expected_height = self.blockchain.read().block_number() + 1;
                    if expected_height != update.block_number {
                        warn!(
                            unexpected = true,
                            expected_height,
                            height = update.block_number,
                            "Got severely outdated Tendermint state, Tendermint instance should be
                             gone already because new blocks were pushed since it was created."
                        );
                    } else {
                        write_transaction.put::<str, Vec<u8>>(
                            &self.database,
                            Self::MACRO_STATE_KEY,
                            &nimiq_serde::Serialize::serialize_to_vec(&update),
                        );

                        write_transaction.commit();
                        *self.macro_state.write() = Some(update);
                    }
                }
            }
        }
    }

    fn poll_micro(&mut self, cx: &mut Context<'_>) {
        let micro_producer = self.micro_producer.as_mut().unwrap();
        while let Poll::Ready(Some(event)) = micro_producer.poll_next_unpin(cx) {
            match event {
                ProduceMicroBlockEvent::MicroBlock(block, result) => {
                    if result == PushResult::Extended || result == PushResult::Rebranched {
                        // Todo get rid of spawn
                        let network = self.network.clone();
                        tokio::spawn(async move {
                            Self::publish_block(network, Block::Micro(block)).await;
                        });
                    }
                }
            }
        }
    }

    async fn publish_block(network: Arc<TValidatorNetwork>, mut block: Block) {
        trace!(%block, "Publishing block");
        if let Err(e) = network.publish::<BlockTopic>(block.clone()).await {
            warn!(
                %block,
                error = &e as &dyn Error,
                "Failed to publish block"
            );
        }
        // Empty body for Micro blocks before publishing to the block header topic
        // Macro blocks must be always sent with body
        match block {
            Block::Micro(ref mut micro_block) => micro_block.body = None,
            Block::Macro(_) => {}
        }
        if let Err(e) = network.publish::<BlockHeaderTopic>(block.clone()).await {
            warn!(
                %block,
                error = &e as &dyn Error,
                "Failed to publish block header"
            );
        }
    }

    /// Checks whether we are an active validator in the current epoch.
    fn is_active(&self) -> bool {
        self.epoch_state.is_some()
    }

    /// Checks whether the validator fulfills the conditions for producing valid blocks.
    /// This includes having consensus, being able to extend the history tree and to enforce transaction validity.
    fn can_be_active(&self) -> bool {
        self.consensus.is_established() && self.blockchain_state.can_enforce_validity_window
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
        )
        .unwrap(); // TODO: Handle transaction creation error
        let tx_hash = reactivate_transaction.hash();

        let cn = self.consensus.clone();
        tokio::spawn(async move {
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
        self.epoch_state
            .as_ref()
            .expect("Validator not active")
            .validator_slot_band
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
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork> Future for Validator<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Process consensus updates.
        while let Poll::Ready(Some(event)) = self.consensus_event_rx.poll_next_unpin(cx) {
            match event {
                Ok(ConsensusEvent::Established) => {
                    self.update_can_enforce_validity_window();
                }
                Ok(ConsensusEvent::Lost) => {
                    self.pause_validator();
                    if let MempoolState::Active = self.mempool_state {
                        let mempool = Arc::clone(&self.mempool);
                        let network = Arc::clone(&self.consensus.network);
                        tokio::spawn(async move {
                            mempool.stop_executors(network).await;
                        });
                        self.mempool_state = MempoolState::Inactive;
                    }
                }
                Err(_) => return Poll::Ready(()),
            }
        }

        // Process blockchain updates.
        let mut received_event: Option<Blake2bHash> = None;
        while let Poll::Ready(Some(event)) = self.blockchain_event_rx.poll_next_unpin(cx) {
            // The `can_enforce_validity_window` flag can only change on macro blocks:
            // It can change to false during macro sync when pushing macro blocks.
            // It can change to true when we reach an offset of the transaction validity window
            // into a new epoch we have the history for. The validity window is a multiple
            // of the batch size – thus it is again a macro block.
            if matches!(
                event,
                BlockchainEvent::Finalized(..) | BlockchainEvent::EpochFinalized(..)
            ) {
                self.update_can_enforce_validity_window();
            }
            let can_be_active = self.can_be_active();
            trace!(?event, can_be_active, "blockchain event");
            let latest_hash = event.get_newest_hash();
            self.on_blockchain_event(event);
            if can_be_active {
                received_event = Some(latest_hash);
            }
        }

        if let Some(event) = received_event {
            self.init_block_producer(Some(event));
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
        if self.can_be_active() && self.is_active() {
            if self.macro_producer.is_some() {
                self.poll_macro(cx);
            }
            if self.micro_producer.is_some() {
                self.poll_micro(cx);
            }
        }

        // Once the validator can be active is established, check the validator staking state.
        if self.can_be_active() {
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
                Ok(NetworkEvent::DhtBootstrapped) => {
                    self.publish_dht();
                }
                Ok(_) => {}
                Err(e) => error!("{}", e),
            }
        }

        Poll::Pending
    }
}

type ProposalAndPubsubId<TValidatorNetwork> = (
    <ProposalTopic<TValidatorNetwork> as Topic>::Item,
    <TValidatorNetwork as ValidatorNetwork>::PubsubId,
);

struct ProposalBuffer<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    buffer: LinkedHashMap<
        <TValidatorNetwork::NetworkType as Network>::PeerId,
        ProposalAndPubsubId<TValidatorNetwork>,
    >,
    waker: Option<Waker>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalBuffer<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    // Ignoring clippy warning: this return type is on purpose
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> (
        ProposalSender<TValidatorNetwork>,
        ProposalReceiver<TValidatorNetwork>,
    ) {
        let buffer = Self {
            buffer: LinkedHashMap::new(),
            waker: None,
        };
        let shared = Arc::new(RwLock::new(buffer));
        let sender = ProposalSender {
            shared: Arc::clone(&shared),
        };
        let receiver = ProposalReceiver { shared };
        (sender, receiver)
    }
}

struct ProposalSender<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    shared: Arc<RwLock<ProposalBuffer<TValidatorNetwork>>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalSender<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    pub fn send(&self, proposal: ProposalAndPubsubId<TValidatorNetwork>) {
        let source = proposal.1.propagation_source();
        let mut shared = self.shared.write();
        shared.buffer.insert(source, proposal);
        if let Some(waker) = shared.waker.take() {
            waker.wake()
        }
    }
}

struct ProposalReceiver<TValidatorNetwork: ValidatorNetwork + 'static>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    shared: Arc<RwLock<ProposalBuffer<TValidatorNetwork>>>,
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream for ProposalReceiver<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    type Item = SignedProposalMessage<
        Header<<TValidatorNetwork as ValidatorNetwork>::PubsubId>,
        (SchnorrSignature, u16),
    >;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut shared = self.shared.write();
        if shared.buffer.is_empty() {
            store_waker!(shared, waker, cx);
            Poll::Pending
        } else {
            let value = shared.buffer.pop_front().map(|entry| {
                let (msg, id) = entry.1;
                msg.into_tendermint_signed_message(Some(id))
            });
            Poll::Ready(value)
        }
    }
}

impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone for ProposalReceiver<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}
