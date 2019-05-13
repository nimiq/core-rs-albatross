use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::RwLock;

use account::Receipt;
use block_albatross::{
    Block,
    BlockType,
    ForkProof,
    MacroBlock,
    MacroHeader,
    MicroBlock,
    MicroExtrinsics,
    MicroHeader,
    MicroJustification,
    PbftCommitMessage,
    PbftPrepareMessage,
    PbftProposal,
    SignedPbftCommitMessage,
    SignedPbftPrepareMessage,
    SignedPbftProposal,
    SignedViewChange,
    ValidatorSlots,
    ViewChange,
    ViewChangeProof,
};
use blockchain_albatross::Blockchain;
use blockchain_base::BlockchainEvent;
use bls::bls12_381::{PublicKey, SecretKey};
use consensus::{Consensus, ConsensusEvent};
use database::Environment;
use hash::{Blake2bHash, SerializeContent};
use mempool::MempoolConfig;
use network::NetworkConfig;
use network_primitives::networks::NetworkId;
use network_primitives::time::NetworkTime;
use utils::key_store::{Error as KeyStoreError, KeyStore};
use utils::mutable_once::MutableOnce;
use utils::timers::Timers;

use crate::error::Error;
use crate::slash::ForkProofPool;
use crate::validator_network::{ValidatorNetwork, ValidatorNetworkEvent};
use nimiq_block_production_albatross::BlockProducer;

#[derive(Debug)]
pub enum ViewInfo {
    ViewNumber(u32),
    ViewChange(ViewChange),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidatorStatus {
    None,
    Synced, // Already reached consensus with peers but we're not still a validator
    Potential,
    Active,
}

pub struct Validator {
    blockchain: Arc<Blockchain<'static>>,
    block_producer: BlockProducer<'static>,
    consensus: Arc<Consensus>,
    //validator_network: Arc<ValidatorNetwork>,
    validator_key: SecretKey,

    timers: Timers<ValidatorTimer>,

    state: RwLock<ValidatorState>,

    self_weak: MutableOnce<Weak<Validator>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ValidatorTimer {
    ViewChange,
}

pub struct ValidatorState {
    current_view_number: u32,
    pk_idx: Option<u16>,
    status: ValidatorStatus,
    fork_proof_pool: ForkProofPool,
}

impl Validator {
    const BLOCK_TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig, mempool_config: MempoolConfig) -> Result<Arc<Self>, Error> {
        unimplemented!()
//        let network_time = Arc::new(NetworkTime::new());
//        let blockchain = Arc::new(Blockchain::new(env, network_id, network_time.clone())?);
//        let consensus = Consensus::new(env, network_id, network_config, mempool_config)?;
//        let validator_network = ValidatorNetwork::new(Arc::clone(&consensus.network), /*Arc::clone(&consensus.blockchain)*/ unimplemented!());
//
//        // FIXME: May be improve KeyStore the use the same file for all keys?
//        let key_store = KeyStore::new("validator_key.db".to_string());
//        let validator_key = match key_store.load_key() {
//            Err(KeyStoreError::IoError(_)) => {
//                let secret_key = SecretKey::generate(&mut rand::thread_rng());
//                key_store.save_key(&secret_key)?;
//                Ok(secret_key)
//            },
//            res => res,
//        }?;
//
//        let block_producer = BlockProducer::new(blockchain.clone(), consensus.mempool.clone(), validator_key);
//
//        let this = Arc::new(Validator {
//            blockchain,
//            block_producer,
//            consensus,
//            validator_network,
//
//            validator_key,
//            timers: Timers::new(),
//
//            state: RwLock::new(ValidatorState {
//                current_view_number: 0,
//                pk_idx: None,
//                status: ValidatorStatus::None,
//                fork_proof_pool: ForkProofPool::new(),
//            }),
//
//            self_weak: MutableOnce::new(Weak::new()),
//        });
//        Validator::init_listeners(&this);
//        Ok(this)
    }

    pub fn init_listeners(this: &Arc<Validator>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        // Setup event handlers for blockchain events
        let weak = Arc::downgrade(this);
        this.consensus.notifier.write().register(move |e: &ConsensusEvent| {
            let this = upgrade_weak!(weak);
            match e {
                ConsensusEvent::Established => this.on_consensus_established(),
                ConsensusEvent::Lost => this.on_consensus_lost(),
                _ => {},
            }
        });

        // Set up event handlers for blockchain events
        let weak = Arc::downgrade(this);
        this.blockchain.register_listener(move |e: &BlockchainEvent<Block>| {
            let this = upgrade_weak!(weak);
            this.on_blockchain_event(e);
        });

        // Set up event handlers for validator network events
        let weak = Arc::downgrade(this);
        this.validator_network.notifier.write().register(move |e: ValidatorNetworkEvent| {
            let this = upgrade_weak!(weak);
            this.on_validator_network_event(e);
        });

        // Set up the view change timer in case there's a block timeout
        // Note: In start_view_change() we check so that it's only executed if we are an active validator
        let weak = Arc::downgrade(this);
        this.timers.set_interval(ValidatorTimer::ViewChange, move || {
            let this = upgrade_weak!(weak);
            this.start_view_change();
        }, Self::BLOCK_TIMEOUT);
    }

    pub fn on_consensus_established(&self) {
        let mut state = self.state.write();

        // FIXME: Should we reset our state here?

        // FIXME: use the real validator registry here
        if self.are_we_potential_validator() {
            state.status = ValidatorStatus::Potential;
        } else {
            // FIXME Set up everything to keep checking if we are with every validator registry change event
            state.status = ValidatorStatus::Synced;
        }
    }

    pub fn on_consensus_lost(&self) {
        // FIXME: Should we reset our state here?
        let mut state = self.state.write();
        state.status = ValidatorStatus::None;
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent<Block>) {
        let state = self.state.read();
        let status = &state.status;

        // Blockchain events are only intersting to validators (potential or active)
        if *status == ValidatorStatus::None || *status == ValidatorStatus::Synced {
            return;
        }

        // Reset the view change timeout because we received a valid block
        let weak = self.self_weak.clone();
        self.timers.reset_interval(ValidatorTimer::ViewChange, move || {
            let this = upgrade_weak!(weak);
            this.start_view_change();
        }, Self::BLOCK_TIMEOUT);

        // Handle each block type (which is directly related to each event type)
        match event {
            BlockchainEvent::Finalized => self.on_blockchain_finalized(), // i.e. a macro block was accepted
            BlockchainEvent::Extended(hash) => self.on_blockchain_extended(hash), // i.e. a micro block was accepted
            BlockchainEvent::Rebranched(old_chain, new_chain) =>
                self.on_blockchain_rebranched(old_chain.to_vec(), new_chain.to_vec()), // FIXME: why .to_vec()?
        }

        // If we're an active validator, we need to check if we're the next block producer
        if *status == ValidatorStatus::Active {
            self.on_slot_change(ViewInfo::ViewNumber(state.current_view_number));
        }
    }

    // Resets the state and checks if we are on the new validator list
    pub fn on_blockchain_finalized(&self) {
        let mut state = self.state.write();

        // FIXME: Are we sure about this? Should we set it to the view number of the received macro block?
        state.current_view_number = 0;

        state.pk_idx = self.get_pk_idx(&self.validator_key);

        match state.pk_idx {
            Some(_) => { state.status = ValidatorStatus::Active },
            None => { state.status = ValidatorStatus::Potential },
        }
    }

    // Sets the state according to the information on the block
    pub fn on_blockchain_extended(&self, hash: &Blake2bHash) {

        let block = self.blockchain.get_block(hash, false, false).unwrap_or_else(|| panic!("We got the block hash ({}) from an event from the blockchain itself", &hash));

        let mut state = self.state.write();
        state.current_view_number = 0;
        state.fork_proof_pool.apply_block(&block);
    }

    // Sets the state according to the rebranch
    pub fn on_blockchain_rebranched(&self, old_chain: Vec<(Blake2bHash, Block)>, new_chain: Vec<(Blake2bHash, Block)>) {
        let mut state = self.state.write();
        for (hash, block) in old_chain.iter() {
            state.fork_proof_pool.revert_block(block);
        }
        for (hash, block) in new_chain.iter() {
            state.fork_proof_pool.apply_block(&block);
        }
        unimplemented!()
    }

    fn on_validator_network_event(&self, event: ValidatorNetworkEvent) {
        let mut state = self.state.write();

        // Validator network events are only intersting to active validators
        if state.status != ValidatorStatus::Active {
            return;
        }

        match event {
            ValidatorNetworkEvent::ViewChangeComplete(view_change) => {
                // Ignore this event if the block number doesn't match our blockchain height
                // FIXME: Check the current view change updating logic as it may be overwritten by lower numbers
                if self.blockchain.height() == view_change.block_number {
                    state.current_view_number = view_change.new_view_number;
                    self.on_slot_change(ViewInfo::ViewChange(view_change));
                }
            },
            ValidatorNetworkEvent::PbftProposal(macro_block) => self.on_pbft_proposal(macro_block),
            ValidatorNetworkEvent::PbftPrepareComplete(hash) => self.on_pbft_prepare_complete(hash),
            ValidatorNetworkEvent::PbftCommitComplete(hash) => self.on_pbft_commit_complete(hash),
            ValidatorNetworkEvent::ForkProof(proof) => self.on_fork_proof(proof),
        }
    }

    fn on_fork_proof(&self, fork_proof: ForkProof) {
        self.state.write().fork_proof_pool.insert(fork_proof);
    }

    pub fn on_slot_change(&self, view_info: ViewInfo) {
        let (view_number, view_change_proof) = match view_info {
            ViewInfo::ViewNumber(view_number) => (view_number, None),
            ViewInfo::ViewChange(view_change) => {
                let view_change_proof = self.validator_network.get_view_change_proof(&view_change);

                (view_change.new_view_number, view_change_proof)
            },
        };

        // Check if we are the next block producer and act accordingly
        let (_, slot) = self.blockchain.get_next_block_producer(view_number);
        let public_key = PublicKey::from_secret(&self.validator_key);

        if slot.public_key == public_key {
            match self.blockchain.get_next_block_type(None) {
                BlockType::Macro => { self.produce_macro_block(view_change_proof) },
                BlockType::Micro => { self.produce_micro_block(view_change_proof) },
            }
        }
    }

    pub fn on_pbft_proposal(&self, _block_proposal: PbftProposal) {
        // FIXME
        let slots = 1u16;
        let public_key = &PublicKey::from_secret(&self.validator_key);

        // Note: we don't verify this hash as the network validator already did
        let block_hash = self.validator_network.get_pbft_proposal_hash().expect("We got the event from the network itself").clone();
        let message = PbftPrepareMessage{ block_hash };
        let pk_idx = self.state.read().pk_idx.expect("Already checked that we are an active validator before calling this function");

        let prepare_message = SignedPbftPrepareMessage::from_message(message, &self.validator_key, pk_idx);

        match self.validator_network.commit_pbft_prepare(prepare_message, public_key, slots) {
            _ => () //FIXME: error handling
        }
    }

    pub fn on_pbft_prepare_complete(&self, hash: Blake2bHash) {
        // FIXME
        let slots = 1u16;
        let public_key = &PublicKey::from_secret(&self.validator_key);

        // Note: we don't verify this hash as the network validator already did
        let message = PbftCommitMessage { block_hash: hash };
        let pk_idx = self.state.read().pk_idx.expect("Already checked that we are an active validator before calling this function");

        let commit_message = SignedPbftCommitMessage::from_message(message, &self.validator_key, pk_idx);

        match self.validator_network.commit_pbft_commit(commit_message, public_key , slots) {
            _ => (),
        }
    }

    pub fn on_pbft_commit_complete(&self, hash: Blake2bHash) {
        let proposal = self.validator_network.get_pbft_proposal().unwrap_or_else(|| panic!("We got the proposal hash ({}) from an event from the network itself", &hash));
        let header = proposal.header.clone();

        // Note: we're not verifying the justification as the validator network already did that
        let justification = self.validator_network.get_pbft_proof().map(|p| p.into_untrusted());

        let block = Block::Macro(MacroBlock { header, justification, extrinsics: None }); // TODO: Extrinsics
        self.blockchain.push(block);

        // FIXME: gossip the new macro block to the network
    }

    fn start_view_change(&self) {
        let mut state = self.state.write();

        // View change messages should only be sent by active validators
        if state.status != ValidatorStatus::Active {
            return;
        }

        // The number of the block that timed out
        let block_number = self.blockchain.height() + 1;
        let new_view_number = state.current_view_number + 1;

        let message = ViewChange { block_number, new_view_number };
        let pk_idx = state.pk_idx.expect("Checked above that we are an active validator");
        let view_change_message = SignedViewChange::from_message(message, &self.validator_key, pk_idx);

        // FIXME
        let slots = 1u16;
        let public_key = &PublicKey::from_secret(&self.validator_key);

        // Broadcast our view change number message to the other validators
        self.validator_network.commit_view_change(view_change_message, public_key, slots);

        // increment current_view_number
        state.current_view_number = new_view_number;
     }

    fn get_pk_idx(&self, secret_key: &SecretKey) -> Option<u16> {
        // FIXME: Check this logic
        let public_key = PublicKey::from_secret(secret_key);
        let validator_list = self.blockchain.get_next_validator_list();

        for (i, validator) in validator_list.iter().enumerate() {
            if validator.public_key == public_key {
                return Some(i as u16);
            }
        }
        None
    }

    fn produce_macro_block(&self, view_change: Option<ViewChangeProof>) {
        let view_number = self.state.read().current_view_number;
        let timestamp = self.consensus.network.network_time.now();

        // TODO: Slashing amount.
        let pbft_proposal = self.block_producer.next_macro_block_proposal(view_number, timestamp, 0, view_change);

        let pk_idx = self.state.read().pk_idx.expect("Checked that we are an active validator before entering this function");

        let signed_proposal = SignedPbftProposal::from_message(pbft_proposal, &self.validator_key, pk_idx);

        self.validator_network.commit_pbft_proposal(signed_proposal);
    }

    fn produce_micro_block(&self, view_change_proof: Option<ViewChangeProof>) {
        let max_size = MicroBlock::MAX_SIZE
            - MicroHeader::SIZE
            - MicroExtrinsics::get_metadata_size(0, 0);

        let state = self.state.read();
        let fork_proofs = state.fork_proof_pool.get_fork_proofs_for_block(max_size);
        let view_number = state.current_view_number;
        let timestamp = self.consensus.network.network_time.now();

        let block = self.block_producer.next_micro_block(fork_proofs, view_number, timestamp, vec![], view_change_proof);

        self.blockchain.push(Block::Micro(block));
    }

    fn are_we_potential_validator(&self) -> bool {
        unimplemented!();
    }
}
