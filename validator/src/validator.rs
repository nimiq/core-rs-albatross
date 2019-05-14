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
use consensus::{AlbatrossConsensusProtocol, Consensus, ConsensusEvent};
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
pub enum SlotChange {
    MicroBlock,
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
    consensus: Arc<Consensus<AlbatrossConsensusProtocol>>,
    validator_network: Arc<ValidatorNetwork>,
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
    pk_idx: Option<u16>,
    status: ValidatorStatus,
    fork_proof_pool: ForkProofPool,
}

impl Validator {
    const BLOCK_TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new(consensus: Arc<Consensus<AlbatrossConsensusProtocol>>) -> Result<Arc<Self>, Error> {
        let validator_network = ValidatorNetwork::new(consensus.network.clone(), consensus.blockchain.clone());

        // FIXME: May be improve KeyStore the use the same file for all keys?
        let key_store = KeyStore::new("validator_key.db".to_string());
        let validator_key = match key_store.load_key() {
            Err(KeyStoreError::IoError(_)) => {
                let secret_key = SecretKey::generate(&mut rand::thread_rng());
                key_store.save_key(&secret_key)?;
                Ok(secret_key)
            },
            res => res,
        }?;

        let block_producer = BlockProducer::new(consensus.blockchain.clone(), consensus.mempool.clone(), validator_key.clone());

        let this = Arc::new(Validator {
            blockchain: consensus.blockchain.clone(),
            block_producer,
            consensus,
            validator_network,

            validator_key,
            timers: Timers::new(),

            state: RwLock::new(ValidatorState {
                pk_idx: None,
                status: ValidatorStatus::None,
                fork_proof_pool: ForkProofPool::new(),
            }),

            self_weak: MutableOnce::new(Weak::new()),
        });
        Validator::init_listeners(&this);
        Ok(this)
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
        this.blockchain.notifier.write().register(move |e: &BlockchainEvent<Block>| {
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

        // TODO: Sync fork proof pool?

        // FIXME: use the real validator registry here.
        if self.is_potential_validator() {
            state.status = ValidatorStatus::Potential;
        } else {
            // FIXME Set up everything to keep checking if we are with every validator registry change event.
            state.status = ValidatorStatus::Synced;
        }
    }

    pub fn on_consensus_lost(&self) {
        let mut state = self.state.write();
        state.status = ValidatorStatus::None;
    }

    fn reset_view_change_interval(&self) {
        let weak = self.self_weak.clone();
        self.timers.reset_interval(ValidatorTimer::ViewChange, move || {
            let this = upgrade_weak!(weak);
            this.start_view_change();
        }, Self::BLOCK_TIMEOUT);
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent<Block>) {
        let state = self.state.read();
        let status = &state.status;

        // Blockchain events are only intersting to validators (potential or active).
        if *status == ValidatorStatus::None || *status == ValidatorStatus::Synced {
            return;
        }

        // Reset the view change timeout because we received a valid block.
        self.reset_view_change_interval();

        // Handle each block type (which is directly related to each event type).
        match event {
            BlockchainEvent::Finalized => self.on_blockchain_finalized(), // i.e. a macro block was accepted
            BlockchainEvent::Extended(hash) => self.on_blockchain_extended(hash), // i.e. a micro block was accepted
            BlockchainEvent::Rebranched(old_chain, new_chain) =>
                self.on_blockchain_rebranched(old_chain, new_chain),
        }

        // If we're an active validator, we need to check if we're the next block producer.
        if *status == ValidatorStatus::Active {
            self.on_slot_change(SlotChange::MicroBlock);
        }
    }

    // Resets the state and checks if we are on the new validator list
    pub fn on_blockchain_finalized(&self) {
        let mut state = self.state.write();

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
        state.fork_proof_pool.apply_block(&block);
    }

    // Sets the state according to the rebranch
    pub fn on_blockchain_rebranched(&self, old_chain: &Vec<(Blake2bHash, Block)>, new_chain: &Vec<(Blake2bHash, Block)>) {
        let mut state = self.state.write();
        for (hash, block) in old_chain.iter() {
            state.fork_proof_pool.revert_block(block);
        }
        for (hash, block) in new_chain.iter() {
            state.fork_proof_pool.apply_block(&block);
        }
    }

    fn on_validator_network_event(&self, event: ValidatorNetworkEvent) {
        let mut state = self.state.write();

        // Validator network events are only intersting to active validators
        if state.status != ValidatorStatus::Active {
            return;
        }

        match event {
            ValidatorNetworkEvent::ViewChangeComplete(view_change) => {
                self.on_slot_change(SlotChange::ViewChange(view_change));
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

    pub fn on_slot_change(&self, slot_change: SlotChange) {
        let (view_number, view_change_proof) = match slot_change {
            SlotChange::MicroBlock => (self.blockchain.view_number(), None),
            SlotChange::ViewChange(view_change) => {
                let view_change_proof = self.validator_network.get_view_change_proof(&view_change);

                // Inform blockchain about the view change, so that it can keep track of it
                // and remove blocks made invalid by the proof.
                self.blockchain.push_known_view_change(view_change.block_number, view_change.new_view_number);
                // Reset view change interval again.
                self.reset_view_change_interval();

                (view_change.new_view_number, view_change_proof)
            },
        };

        // Check if we are the next block producer and act accordingly
        let (_, slot) = self.blockchain.get_next_block_producer();
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

        // The number of the block that timed out.
        let block_number = self.blockchain.height() + 1;
        let new_view_number = self.blockchain.view_number() + 1;

        let message = ViewChange { block_number, new_view_number };
        let pk_idx = state.pk_idx.expect("Checked above that we are an active validator");
        let view_change_message = SignedViewChange::from_message(message, &self.validator_key, pk_idx);

        let validator = self.blockchain.get_current_validator_by_idx(pk_idx).expect("Checked above that we are an active validator");
        let public_key = &PublicKey::from_secret(&self.validator_key);

        // Broadcast our view change number message to the other validators.
        self.validator_network.commit_view_change(view_change_message, public_key, validator.slots);
     }

    fn get_pk_idx(&self, secret_key: &SecretKey) -> Option<u16> {
        let public_key = PublicKey::from_secret(secret_key);
        let validator_list = self.blockchain.get_next_validator_set();

        for (i, validator) in validator_list.iter().enumerate() {
            if validator.public_key == public_key {
                return Some(i as u16);
            }
        }
        None
    }

    fn produce_macro_block(&self, view_change: Option<ViewChangeProof>) {
        let timestamp = self.consensus.network.network_time.now();

        // TODO: Slashing amount.
        let pbft_proposal = self.block_producer.next_macro_block_proposal(timestamp, 0, view_change);

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
        let timestamp = self.consensus.network.network_time.now();

        let block = self.block_producer.next_micro_block(fork_proofs, timestamp, vec![], view_change_proof);

        self.blockchain.push(Block::Micro(block));
    }

    fn is_potential_validator(&self) -> bool {
        // TODO: Use StakingContract to determine if we are a potential validator.
        //self.blockchain.state().accounts().get()
        false
    }
}
