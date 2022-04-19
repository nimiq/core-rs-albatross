use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::{
    task::{Context, Poll, Waker},
    Future, Stream, StreamExt,
};
use linked_hash_map::LinkedHashMap;
use parking_lot::RwLock;
use tokio_stream::wrappers::BroadcastStream;

use account::StakingContract;
use block::{Block, BlockType, SignedTendermintProposal, ViewChange, ViewChangeProof};
use block_production::BlockProducer;
use blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent, ForkEvent, PushResult};
use bls::{CompressedPublicKey, KeyPair as BlsKeyPair};
use consensus::{sync::block_queue::BlockTopic, Consensus, ConsensusEvent, ConsensusProxy};
use database::{Database, Environment, ReadTransaction, WriteTransaction};
use hash::{Blake2bHash, Hash};
use keys::{Address, KeyPair as SchnorrKeyPair};
use mempool::{config::MempoolConfig, mempool::Mempool};
use network_interface::network::{Network, PubsubId, Topic};
use primitives::coin::Coin;
use primitives::policy;
use tendermint_protocol::TendermintReturn;
use transaction_builder::TransactionBuilder;
use utils::observer::NotifierStream;
use validator_network::ValidatorNetwork;

use crate::micro::{ProduceMicroBlock, ProduceMicroBlockEvent};
use crate::r#macro::{PersistedMacroState, ProduceMacroBlock};
use crate::slash::ForkProofPool;

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

struct ProduceMicroBlockState {
    view_number: u32,
    view_change_proof: Option<ViewChangeProof>,
    view_change: Option<ViewChange>,
}

/// Validator parking state
struct ParkingState {
    park_tx_hash: Blake2bHash,
    park_tx_validity_window_start: u32,
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
}

impl Clone for ValidatorProxy {
    fn clone(&self) -> Self {
        Self {
            validator_address: Arc::clone(&self.validator_address),
            signing_key: Arc::clone(&self.signing_key),
            voting_key: Arc::clone(&self.voting_key),
            fee_key: Arc::clone(&self.fee_key),
        }
    }
}

pub struct Validator<TNetwork: Network, TValidatorNetwork: ValidatorNetwork + 'static> {
    pub consensus: ConsensusProxy<TNetwork>,
    network: Arc<TValidatorNetwork>,

    database: Database,
    env: Environment,

    validator_address: Arc<RwLock<Address>>,
    signing_key: Arc<RwLock<SchnorrKeyPair>>,
    voting_key: Arc<RwLock<BlsKeyPair>>,
    fee_key: Arc<RwLock<SchnorrKeyPair>>,

    proposal_receiver: ProposalReceiver<TValidatorNetwork>,

    consensus_event_rx: BroadcastStream<ConsensusEvent>,
    blockchain_event_rx: NotifierStream<BlockchainEvent>,
    fork_event_rx: NotifierStream<ForkEvent>,

    epoch_state: Option<ActiveEpochState>,
    blockchain_state: BlockchainState,
    parking_state: Option<ParkingState>,

    macro_producer: Option<ProduceMacroBlock<TValidatorNetwork>>,
    macro_state: Option<PersistedMacroState<TValidatorNetwork>>,

    micro_producer: Option<ProduceMicroBlock<TValidatorNetwork>>,
    micro_state: ProduceMicroBlockState,

    pub mempool: Arc<Mempool>,
    mempool_state: MempoolState,
}

impl<TNetwork: Network, TValidatorNetwork: ValidatorNetwork>
    Validator<TNetwork, TValidatorNetwork>
{
    const MACRO_STATE_DB_NAME: &'static str = "ValidatorState";
    const MACRO_STATE_KEY: &'static str = "validatorState";
    const VIEW_CHANGE_DELAY: Duration = Duration::from_secs(10);
    const FORK_PROOFS_MAX_SIZE: usize = 1_000; // bytes

    pub fn new(
        consensus: &Consensus<TNetwork>,
        network: Arc<TValidatorNetwork>,
        validator_address: Address,
        signing_key: SchnorrKeyPair,
        voting_key: BlsKeyPair,
        fee_key: SchnorrKeyPair,
        mempool_config: MempoolConfig,
    ) -> Self {
        let consensus_event_rx = consensus.subscribe_events();

        let mut blockchain = consensus.blockchain.write();
        let blockchain_event_rx = blockchain.notifier.as_stream();
        let fork_event_rx = blockchain.fork_notifier.as_stream();

        let micro_state = ProduceMicroBlockState {
            view_number: blockchain.view_number(),
            view_change_proof: None,
            view_change: None,
        };
        drop(blockchain);

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

        let mempool = Arc::new(Mempool::new(consensus.blockchain.clone(), mempool_config));
        let mempool_state = MempoolState::Inactive;

        let mut this = Self {
            consensus: consensus.proxy(),
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
            parking_state: None,

            macro_producer: None,
            macro_state,

            micro_producer: None,
            micro_state,

            mempool: Arc::clone(&mempool),
            mempool_state,
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

    fn init(&mut self) {
        self.init_epoch();
        self.init_block_producer(None);
    }

    fn init_epoch(&mut self) {
        log::debug!("Initializing epoch");

        // Clear producers here, as this validator might not be active anymore.
        self.macro_producer = None;
        self.micro_producer = None;

        let blockchain = self.consensus.blockchain.read();

        // Check if the transaction was sent
        if let Some(parking_state) = &self.parking_state {
            // Check that the transaction was sent in the validity window
            let staking_state = self.get_staking_state(&*blockchain);
            if staking_state == ValidatorStakingState::Parked
                && blockchain.block_number()
                    >= parking_state.park_tx_validity_window_start + policy::BLOCKS_PER_EPOCH
                && !blockchain.tx_in_validity_window(
                    &parking_state.park_tx_hash,
                    parking_state.park_tx_validity_window_start,
                    None,
                )
            {
                // If we are parked and no transaction has been seen in the expected validity window
                // after an epoch, reset our parking state
                log::debug!("Resetting state to re-send un-park transactions since we are parked and validity window doesn't contain the transaction sent");
                self.parking_state = None;
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

        let voting_keys: Vec<CompressedPublicKey> = validators
            .iter()
            .map(|validator| validator.voting_key.compressed().clone())
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

        let blockchain = self.consensus.blockchain.read();

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
        let next_view_number = head.next_view_number();
        let block_producer = BlockProducer::new(self.signing_key(), self.voting_key());

        debug!(
            next_block_number = next_block_number,
            next_view_number = next_view_number,
            "Initializing block producer"
        );

        self.macro_producer = None;
        self.micro_producer = None;

        match blockchain.get_next_block_type(None) {
            BlockType::Macro => {
                let active_validators = blockchain.current_validators().unwrap();
                let proposal_stream = self.proposal_receiver.clone().boxed();

                drop(blockchain);

                self.macro_producer = Some(ProduceMacroBlock::new(
                    Arc::clone(&self.consensus.blockchain),
                    Arc::clone(&self.network),
                    block_producer,
                    self.validator_slot_band(),
                    active_validators,
                    head.seed().clone(),
                    next_block_number,
                    next_view_number,
                    self.macro_state.take(),
                    proposal_stream,
                ));
            }
            BlockType::Micro => {
                self.micro_state = ProduceMicroBlockState {
                    view_number: next_view_number,
                    view_change_proof: None,
                    view_change: None,
                };

                let fork_proofs = self
                    .blockchain_state
                    .fork_proofs
                    .get_fork_proofs_for_block(Self::FORK_PROOFS_MAX_SIZE);
                let prev_seed = head.seed().clone();

                drop(blockchain);

                self.micro_producer = Some(ProduceMicroBlock::new(
                    Arc::clone(&self.consensus.blockchain),
                    Arc::clone(&self.mempool),
                    Arc::clone(&self.network),
                    block_producer,
                    self.validator_slot_band(),
                    fork_proofs,
                    prev_seed,
                    next_block_number,
                    self.micro_state.view_number,
                    self.micro_state.view_change_proof.clone(),
                    self.micro_state.view_change.clone(),
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
    }

    fn on_blockchain_extended(&mut self, hash: &Blake2bHash) {
        let block = self
            .consensus
            .blockchain
            .read()
            .get_block(hash, true, None)
            .expect("Head block not found");

        // Update mempool and blockchain state
        self.blockchain_state.fork_proofs.apply_block(&block);
        self.mempool
            .mempool_update(&vec![(hash.clone(), block)], &[].to_vec());
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
                            self.consensus.blockchain.upgradable_read(),
                            Block::Macro(block),
                        )
                        .map_err(|e| error!("Failed to push macro block onto the chain: {:?}", e))
                        .ok()
                    } else {
                        Blockchain::push(
                            self.consensus.blockchain.upgradable_read(),
                            Block::Macro(block),
                        )
                        .map_err(|e| error!("Failed to push macro block onto the chain: {:?}", e))
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
                            let block_number = block_copy.header.block_number;
                            trace!(block_number = block_number, "Publishing macro block");

                            if let Err(e) = network
                                .publish::<BlockTopic>(Block::Macro(block_copy))
                                .await
                            {
                                warn!(
                                    block_number = block_number,
                                    error = &e as &dyn Error,
                                    "Failed to publish macro block"
                                );
                            }
                        });
                    }
                }

                // In case of a new state update we need to store the new version of it disregarding
                // any old state which potentially still lingers.
                TendermintReturn::StateUpdate(update) => {
                    trace!("Tendermint state update {:?}", update);
                    let mut write_transaction = WriteTransaction::new(&self.env);
                    let expected_height = self.consensus.blockchain.read().block_number() + 1;
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
                            let block_number = block.header.block_number;
                            trace!(block_number = block_number, "Publishing micro block");

                            if let Err(e) = network.publish::<BlockTopic>(Block::Micro(block)).await
                            {
                                warn!(
                                    block_number = block_number,
                                    error = &e as &dyn Error,
                                    "Failed to publish micro block"
                                );
                            }
                        });
                    }
                }
                ProduceMicroBlockEvent::ViewChange(view_change, view_change_proof) => {
                    self.micro_state.view_number = view_change.new_view_number; // needed?
                    self.micro_state.view_change_proof = Some(view_change_proof);
                    self.micro_state.view_change = Some(view_change);
                }
            }
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

    fn unpark(&self, blockchain: &Blockchain) -> ParkingState {
        // TODO: Get the last view change height instead of the current height
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
        tokio::spawn(async move {
            debug!("Sending unpark transaction");
            if cn.send_transaction(unpark_transaction).await.is_err() {
                error!("Failed to send unpark transaction");
            }
        });

        ParkingState {
            park_tx_hash: tx_hash,
            park_tx_validity_window_start: validity_start_height,
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
                        tokio::spawn(async move {
                            mempool.start_executor(network).await;
                        });
                        self.mempool_state = MempoolState::Active;
                    }
                }
                Ok(ConsensusEvent::Lost) => {
                    if let MempoolState::Active = self.mempool_state {
                        let mempool = Arc::clone(&self.mempool);
                        let network = Arc::clone(&self.consensus.network);
                        tokio::spawn(async move {
                            mempool.stop_executor(network).await;
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
        while let Poll::Ready(Some(event)) = self.fork_event_rx.poll_next_unpin(cx) {
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
            let blockchain = self.consensus.blockchain.read();
            match self.get_staking_state(&*blockchain) {
                ValidatorStakingState::Parked => {
                    if self.parking_state.is_none() {
                        let parking_state = self.unpark(&*blockchain);
                        drop(blockchain);
                        self.parking_state = Some(parking_state);
                    }
                }
                ValidatorStakingState::Active => {
                    drop(blockchain);
                    if self.parking_state.is_some() {
                        self.parking_state = None;
                    }
                }
                _ => {}
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
