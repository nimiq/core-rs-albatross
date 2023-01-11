use futures::stream::BoxStream;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures::{Stream, StreamExt};
use linked_hash_map::LinkedHashMap;
use nimiq_bls::lazy::LazyPublicKey;
use parking_lot::RwLock;
#[cfg(feature = "metrics")]
use tokio_metrics::TaskMonitor;
use tokio_stream::wrappers::BroadcastStream;

use crate::micro::{ProduceMicroBlock, ProduceMicroBlockEvent};
use crate::r#macro::{PersistedMacroState, ProduceMacroBlock};
use crate::slash::ForkProofPool;
use nimiq_account::StakingContract;
use nimiq_block::{Block, BlockType, SignedTendermintProposal};
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent, ForkEvent, PushResult};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_consensus::sync::live::block_queue::{BlockHeaderTopic, BlockTopic};
use nimiq_consensus::{Consensus, ConsensusEvent, ConsensusProxy};
use nimiq_database::{Database, Environment, ReadTransaction, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair};
use nimiq_macros::store_waker;
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool, mempool_transactions::TxPriority};
use nimiq_network_interface::network::{Network, PubsubId, Topic};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_tendermint::TendermintReturn;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_validator_network::ValidatorNetwork;

pub struct ProposalTopic;

impl Topic for ProposalTopic {
    type Item = SignedTendermintProposal;

    const BUFFER_SIZE: usize = 8;
    const NAME: &'static str = "tendermint-proposal";
    const VALIDATE: bool = true;
}

#[derive(PartialEq)]
enum ValidatorStakingState {
    Active,
    Parked,
    Inactive,
    NoStake,
}

struct ActiveEpochState {
    validator_slot_band: u16,
}

struct BlockchainState {
    fork_proofs: ForkProofPool,
}

/// Validator inactivity and parking state
enum ValidatorState {
    /// Validator parking state
    ParkingState {
        park_tx_hash: Blake2bHash,
        park_tx_validity_window_start: u32,
    },
    /// Validator inactive state
    InactivityState {
        inactive_tx_hash: Blake2bHash,
        inactive_tx_validity_window_start: u32,
    },
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

pub struct Validator<TNetwork: Network, TValidatorNetwork: ValidatorNetwork + 'static> {
    pub consensus: ConsensusProxy<TNetwork>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    network: Arc<TValidatorNetwork>,

    database: Database,
    env: Environment,

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
    validator_state: Option<ValidatorState>,
    automatic_reactivate: Arc<AtomicBool>,

    macro_producer: Option<ProduceMacroBlock<TValidatorNetwork>>,
    macro_state: Option<PersistedMacroState<TValidatorNetwork>>,

    micro_producer: Option<ProduceMicroBlock<TValidatorNetwork>>,

    pub mempool: Arc<Mempool>,
    mempool_state: MempoolState,
    #[cfg(feature = "metrics")]
    mempool_monitor: TaskMonitor,
    #[cfg(feature = "metrics")]
    control_mempool_monitor: TaskMonitor,
}

impl<TNetwork: Network, TValidatorNetwork: ValidatorNetwork>
    Validator<TNetwork, TValidatorNetwork>
{
    const MACRO_STATE_DB_NAME: &'static str = "ValidatorState";
    const MACRO_STATE_KEY: &'static str = "validatorState";
    const PRODUCER_TIMEOUT: Duration = Duration::from_millis(Policy::BLOCK_PRODUCER_TIMEOUT);
    const BLOCK_SEPARATION_TIME: Duration = Duration::from_millis(Policy::BLOCK_SEPARATION_TIME);
    const FORK_PROOFS_MAX_SIZE: usize = 1_000; // bytes

    pub fn new(
        consensus: &Consensus<TNetwork>,
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
            fork_proofs: ForkProofPool::new(),
        };

        let env = consensus.env.clone();
        let database = env.open_database(Self::MACRO_STATE_DB_NAME.to_string());

        let macro_state: Option<PersistedMacroState<TValidatorNetwork>> = {
            let read_transaction = ReadTransaction::new(&env);
            read_transaction.get(&database, Self::MACRO_STATE_KEY)
        };

        let network1 = Arc::clone(&network);
        let (proposal_sender, proposal_receiver) = ProposalBuffer::new();

        let mempool = Arc::new(Mempool::new(Arc::clone(&blockchain), mempool_config));
        let mempool_state = MempoolState::Inactive;

        let automatic_reactivate = Arc::new(AtomicBool::new(automatic_reactivate));

        let mut this = Self {
            consensus: consensus.proxy(),
            blockchain,
            network,

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
            macro_state,

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
                .subscribe::<ProposalTopic>()
                .await
                .expect("Failed to subscribe to proposal topic")
                .for_each(|proposal| async { proposal_sender.send(proposal) })
                .await
        });

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

    fn init(&mut self) {
        self.init_epoch();
        self.init_block_producer(None);
    }

    fn init_epoch(&mut self) {
        log::debug!("Initializing epoch");

        // Clear producers here, as this validator might not be active anymore.
        self.macro_producer = None;
        self.micro_producer = None;

        let blockchain = self.blockchain.read();

        // Check if the transaction was sent
        if let Some(validator_state) = &self.validator_state {
            let (tx_hash, tx_validity_window_start) = match validator_state {
                ValidatorState::ParkingState {
                    park_tx_hash,
                    park_tx_validity_window_start,
                } => (park_tx_hash, park_tx_validity_window_start),
                ValidatorState::InactivityState {
                    inactive_tx_hash,
                    inactive_tx_validity_window_start,
                } => (inactive_tx_hash, inactive_tx_validity_window_start),
            };
            // Check that the transaction was sent in the validity window
            let staking_state = self.get_staking_state(&blockchain);
            if (staking_state == ValidatorStakingState::Parked
                || staking_state == ValidatorStakingState::Inactive)
                && blockchain.block_number()
                    >= tx_validity_window_start + Policy::blocks_per_epoch()
                && !blockchain.tx_in_validity_window(tx_hash, *tx_validity_window_start, None)
            {
                // If we are parked or inactive and no transaction has been seen in the expected validity window
                // after an epoch, reset our parking or inactive state
                log::debug!("Resetting state to re-send un-park/activate transactions since we are parked/inactive and validity window doesn't contain the transaction sent");
                self.validator_state = None;
            }
        }

        let validators = blockchain.current_validators().unwrap();

        self.epoch_state = None;
        log::trace!(
            "This is our validator address: {}",
            self.validator_address()
        );

        for (i, validator) in validators.iter().enumerate() {
            log::trace!(
                "Matching against this current validator: {}",
                &validator.address
            );
            if validator.address == self.validator_address() {
                log::debug!("We are active on this epoch");
                self.epoch_state = Some(ActiveEpochState {
                    validator_slot_band: i as u16,
                });
                break;
            }
        }

        let voting_keys: Vec<LazyPublicKey> = validators
            .iter()
            .map(|validator| validator.voting_key.clone())
            .collect();
        let key = self.voting_key();
        let network = Arc::clone(&self.network);

        // TODO might better be done without the task.
        // However we have an entire batch to execute the task so it should not be extremely bad.
        // Also the setting up of our own public key record should probably not be done here but in `init` instead.
        tokio::spawn(async move {
            if let Err(err) = network
                .set_public_key(&key.public_key.compress(), &key.secret_key)
                .await
            {
                error!("could not set up DHT record: {:?}", err);
            }
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
                    head.seed().clone(),
                    next_block_number,
                    0, // TODO: check this
                    // This cannot be .take() as there is a chance init_epoch is called multiple times without
                    // poll_macro being called creating a new macro_state that is Some(...).
                    self.macro_state.clone(),
                    proposal_stream,
                ));
            }
            BlockType::Micro => {
                let fork_proofs = self
                    .blockchain_state
                    .fork_proofs
                    .get_fork_proofs_for_block(Self::FORK_PROOFS_MAX_SIZE);
                let prev_seed = head.seed().clone();

                drop(blockchain);

                self.micro_producer = Some(ProduceMicroBlock::new(
                    Arc::clone(&self.blockchain),
                    Arc::clone(&self.mempool),
                    Arc::clone(&self.network),
                    block_producer,
                    self.validator_slot_band(),
                    fork_proofs,
                    prev_seed,
                    next_block_number,
                    Self::PRODUCER_TIMEOUT,
                    Self::BLOCK_SEPARATION_TIME,
                ));
            }
        }
    }

    fn on_blockchain_event(&mut self, event: BlockchainEvent) {
        match event {
            BlockchainEvent::Extended(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::HistoryAdopted(ref hash) => self.on_blockchain_history_adopted(hash),
            BlockchainEvent::Finalized(ref hash) => self.on_blockchain_extended(hash),
            BlockchainEvent::EpochFinalized(ref hash) => {
                self.on_blockchain_extended(hash);
                self.init_epoch()
            }
            BlockchainEvent::Rebranched(ref old_chain, ref new_chain) => {
                self.on_blockchain_rebranched(old_chain, new_chain)
            }
        }
    }

    fn on_blockchain_history_adopted(&mut self, _: &Blake2bHash) {
        self.mempool.mempool_clean_up();
        debug!("Performed a mempool clean up because new history was adopted");
    }

    fn on_blockchain_extended(&mut self, hash: &Blake2bHash) {
        let block = self
            .consensus
            .blockchain
            .read()
            .get_block(hash, true)
            .expect("Head block not found");

        // Update mempool and blockchain state
        self.blockchain_state.fork_proofs.apply_block(&block);
        self.mempool
            .mempool_update(&vec![(hash.clone(), block)], [].as_ref());
    }

    fn on_blockchain_rebranched(
        &mut self,
        old_chain: &[(Blake2bHash, Block)],
        new_chain: &[(Blake2bHash, Block)],
    ) {
        // Update mempool and blockchain state
        for (_hash, block) in old_chain.iter() {
            self.blockchain_state.fork_proofs.revert_block(block);
        }
        for (_hash, block) in new_chain.iter() {
            self.blockchain_state.fork_proofs.apply_block(block);
        }
        self.mempool.mempool_update(new_chain, old_chain);
    }

    fn on_fork_event(&mut self, event: ForkEvent) {
        match event {
            ForkEvent::Detected(fork_proof) => self.blockchain_state.fork_proofs.insert(fork_proof),
        };
    }

    fn poll_macro(&mut self, cx: &mut Context<'_>) {
        let macro_producer = self.macro_producer.as_mut().unwrap();
        while let Poll::Ready(Some(event)) = macro_producer.poll_next_unpin(cx) {
            match event {
                TendermintReturn::Error(err) => {
                    log::error!("Tendermint returned an error: {:?}", err);
                }
                TendermintReturn::Result(block) => {
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
                TendermintReturn::StateUpdate(update) => {
                    trace!("Tendermint state update {:?}", update);
                    let mut write_transaction = WriteTransaction::new(&self.env);
                    let expected_height = self.blockchain.read().block_number() + 1;
                    if expected_height != update.height {
                        warn!(
                            unexpected = true,
                            expected_height,
                            height = update.height,
                            "Got severely outdated Tendermint state, Tendermint instance should be
                             gone already because new blocks were pushed since it was created."
                        );
                    }

                    write_transaction.put::<str, Vec<u8>>(
                        &self.database,
                        Self::MACRO_STATE_KEY,
                        &beserial::Serialize::serialize_to_vec(&update),
                    );

                    write_transaction.commit();

                    let persistable_state = PersistedMacroState(update);
                    self.macro_state = Some(persistable_state);
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

    fn is_active(&self) -> bool {
        self.epoch_state.is_some()
    }

    fn get_staking_state(&self, blockchain: &Blockchain) -> ValidatorStakingState {
        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();

        // First, check if the validator is parked.
        let validator_address = self.validator_address();
        let staking_contract = StakingContract::get_staking_contract(accounts_tree, &db_txn);
        if staking_contract.parked_set.contains(&validator_address)
            || staking_contract
                .current_disabled_slots
                .contains_key(&validator_address)
            || staking_contract
                .previous_disabled_slots
                .contains_key(&validator_address)
        {
            return ValidatorStakingState::Parked;
        }

        if let Some(validator) =
            StakingContract::get_validator(accounts_tree, &db_txn, &validator_address)
        {
            if validator.inactivity_flag.is_some() {
                ValidatorStakingState::Inactive
            } else {
                ValidatorStakingState::Active
            }
        } else {
            ValidatorStakingState::NoStake
        }
    }

    fn unpark(&self, blockchain: &Blockchain) -> ValidatorState {
        let validity_start_height = blockchain.block_number();

        let unpark_transaction = TransactionBuilder::new_unpark_validator(
            &self.fee_key(),
            self.validator_address(),
            &self.signing_key(),
            Coin::ZERO,
            validity_start_height,
            blockchain.network_id(),
        )
        .unwrap(); // TODO: Handle transaction creation error
        let tx_hash = unpark_transaction.hash();

        let cn = self.consensus.clone();
        let mempool = self.mempool.clone();

        // We publish the unpark transaction
        let publish_transaction = unpark_transaction.clone();
        tokio::spawn(async move {
            debug!("Publishing unpark transaction");
            if cn.send_transaction(publish_transaction).await.is_err() {
                error!("Failed to send unpark transaction");
            }
        });

        // We also add the unpark transaction to our own mempool with high piority
        tokio::spawn(async move {
            debug!("Adding unpark transaction to mempool");
            if mempool
                .add_transaction(unpark_transaction, Some(TxPriority::HighPriority))
                .await
                .is_err()
            {
                error!("Failed adding unpark transaction into mempool");
            }
        });

        ValidatorState::ParkingState {
            park_tx_hash: tx_hash,
            park_tx_validity_window_start: validity_start_height,
        }
    }

    fn reactivate(&self, blockchain: &Blockchain) -> ValidatorState {
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

        ValidatorState::InactivityState {
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

impl<TNetwork: Network, TValidatorNetwork: ValidatorNetwork> Future
    for Validator<TNetwork, TValidatorNetwork>
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Process consensus updates.
        while let Poll::Ready(Some(event)) = self.consensus_event_rx.poll_next_unpin(cx) {
            match event {
                Ok(ConsensusEvent::Established) => {
                    self.init();
                    if let MempoolState::Inactive = self.mempool_state {
                        let mempool = Arc::clone(&self.mempool);
                        let network = Arc::clone(&self.consensus.network);
                        #[cfg(not(feature = "metrics"))]
                        tokio::spawn({
                            async move {
                                mempool.start_executors(network, None, None).await;
                            }
                        });
                        #[cfg(feature = "metrics")]
                        tokio::spawn({
                            let mempool_monitor = self.mempool_monitor.clone();
                            let ctrl_mempool_monitor = self.control_mempool_monitor.clone();
                            async move {
                                mempool
                                    .start_executors(
                                        network,
                                        Some(mempool_monitor),
                                        Some(ctrl_mempool_monitor),
                                    )
                                    .await;
                            }
                        });
                        self.mempool_state = MempoolState::Active;
                    }
                }
                Ok(ConsensusEvent::Lost) => {
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
            let consensus_established = self.consensus.is_established();
            trace!(consensus_established, "blockchain event {:?}", event);
            if consensus_established {
                let latest_hash = event.get_newest_hash();
                self.on_blockchain_event(event);
                received_event = Some(latest_hash);
            }
        }

        if let Some(event) = received_event {
            self.init_block_producer(Some(event));
        }

        // Process fork events.
        while let Poll::Ready(Some(Ok(event))) = self.fork_event_rx.poll_next_unpin(cx) {
            let consensus_established = self.consensus.is_established();
            trace!(consensus_established, "fork event {:?}", event);
            if consensus_established {
                self.on_fork_event(event);
            }
        }

        // If we are an active validator, participate in block production.
        if self.consensus.is_established() && self.is_active() {
            if self.macro_producer.is_some() {
                self.poll_macro(cx);
            }
            if self.micro_producer.is_some() {
                self.poll_micro(cx);
            }
        }

        // Once consensus is established, check the validator staking state.
        if self.consensus.is_established() {
            let blockchain = self.blockchain.read();
            match self.get_staking_state(&blockchain) {
                ValidatorStakingState::Parked => match self.validator_state {
                    Some(ValidatorState::ParkingState { .. }) => {}
                    _ => {
                        let parking_state = self.unpark(&blockchain);
                        drop(blockchain);
                        self.validator_state = Some(parking_state);
                    }
                },
                ValidatorStakingState::Active => {
                    drop(blockchain);
                    if self.validator_state.is_some() {
                        self.validator_state = None;
                        info!("Automatically reactivativated.");
                    }
                }
                ValidatorStakingState::Inactive => match self.validator_state {
                    Some(ValidatorState::InactivityState { .. }) => {}
                    _ => {
                        if self.automatic_reactivate.load(Ordering::Acquire) {
                            let inactivity_state = self.reactivate(&blockchain);
                            drop(blockchain);
                            self.validator_state = Some(inactivity_state);
                        }
                    }
                },
                ValidatorStakingState::NoStake => {}
            }
        }

        Poll::Pending
    }
}

type ProposalAndPubsubId<TValidatorNetwork> = (
    <ProposalTopic as Topic>::Item,
    <TValidatorNetwork as ValidatorNetwork>::PubsubId,
);

struct ProposalBuffer<TValidatorNetwork: ValidatorNetwork + 'static> {
    buffer: LinkedHashMap<
        <TValidatorNetwork::NetworkType as Network>::PeerId,
        ProposalAndPubsubId<TValidatorNetwork>,
    >,
    waker: Option<Waker>,
}
impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalBuffer<TValidatorNetwork> {
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

struct ProposalSender<TValidatorNetwork: ValidatorNetwork + 'static> {
    shared: Arc<RwLock<ProposalBuffer<TValidatorNetwork>>>,
}
impl<TValidatorNetwork: ValidatorNetwork + 'static> ProposalSender<TValidatorNetwork> {
    pub fn send(&self, proposal: ProposalAndPubsubId<TValidatorNetwork>) {
        let source = proposal.1.propagation_source();
        let mut shared = self.shared.write();
        shared.buffer.insert(source, proposal);
        if let Some(waker) = shared.waker.take() {
            waker.wake()
        }
    }
}

struct ProposalReceiver<TValidatorNetwork: ValidatorNetwork + 'static> {
    shared: Arc<RwLock<ProposalBuffer<TValidatorNetwork>>>,
}
impl<TValidatorNetwork: ValidatorNetwork + 'static> Stream for ProposalReceiver<TValidatorNetwork> {
    type Item = ProposalAndPubsubId<TValidatorNetwork>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut shared = self.shared.write();
        if shared.buffer.is_empty() {
            store_waker!(shared, waker, cx);
            Poll::Pending
        } else {
            let value = shared.buffer.pop_front().map(|entry| entry.1);
            Poll::Ready(value)
        }
    }
}
impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone for ProposalReceiver<TValidatorNetwork> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}
