use std::sync::Arc;

use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainError, BlockchainEvent, ChainInfo, ForkEvent,
};
use nimiq_collections::BitSet;
use nimiq_genesis::NetworkInfo;
use nimiq_primitives::{
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{Slot, Validators},
};
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::{Rng, VrfEntropy, VrfUseCase};
use tokio::sync::broadcast::{channel as broadcast, Sender as BroadcastSender};

use crate::chain_store::ChainStore;

const BROADCAST_MAX_CAPACITY: usize = 256;

/// The Blockchain struct. It stores all information of the blockchain that is known to the Nano
/// nodes.
pub struct LightBlockchain {
    /// The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    /// The OffsetTime struct. It allows us to query the current time.
    pub time: Arc<OffsetTime>,
    /// The head of the main chain.
    pub head: Block,
    /// The last macro block.
    pub macro_head: MacroBlock,
    /// The last election block.
    pub election_head: MacroBlock,
    /// The validators for the current epoch.
    pub current_validators: Option<Validators>,
    /// The genesis block.
    pub genesis_block: Block,
    /// The chain store is a database containing all of the chain infos in the current batch.
    pub chain_store: ChainStore,
    /// The notifier processes events relative to the blockchain.
    pub notifier: BroadcastSender<BlockchainEvent>,
    /// The fork notifier processes fork events.
    pub fork_notifier: BroadcastSender<ForkEvent>,
}

/// Implements methods to start a Blockchain.
impl LightBlockchain {
    /// Creates a new blockchain from a given network ID.
    pub fn new(network_id: NetworkId) -> Self {
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block();
        Self::with_genesis(network_id, genesis_block)
    }

    /// Creates a new blockchain with a given network ID and genesis block.
    pub fn with_genesis(network_id: NetworkId, genesis_block: Block) -> Self {
        let time = Arc::new(OffsetTime::new());

        let chain_info = ChainInfo::new(genesis_block.clone(), true);

        let mut chain_store = ChainStore::default();

        chain_store.put_chain_info(chain_info);

        let (tx, _rx) = broadcast(BROADCAST_MAX_CAPACITY);
        let (tx_fork, _rx_fork) = broadcast(BROADCAST_MAX_CAPACITY);

        LightBlockchain {
            network_id,
            time,
            head: genesis_block.clone(),
            macro_head: genesis_block.clone().unwrap_macro(),
            election_head: genesis_block.clone().unwrap_macro(),
            current_validators: genesis_block.validators(),
            genesis_block,
            chain_store,
            notifier: tx,
            fork_notifier: tx_fork,
        }
    }

    /// Gets the active validators for a given epoch.
    pub fn get_validators_for_epoch(&self, epoch: u32) -> Result<Validators, BlockchainError> {
        let current_epoch = Policy::epoch_at(self.head.block_number());

        if epoch == current_epoch {
            self.current_validators
                .clone()
                .ok_or(BlockchainError::NoValidatorsFound)
        } else if epoch == 0 {
            Err(BlockchainError::InvalidEpoch)
        } else {
            self.get_block_at(
                Policy::election_block_of(epoch - 1).ok_or(BlockchainError::InvalidEpoch)?,
                true,
            )?
            .unwrap_macro()
            .get_validators()
            .ok_or(BlockchainError::NoValidatorsFound)
        }
    }

    // FIXME Duplicated function (also exists in full blockchain)
    pub fn get_proposer(
        &self,
        block_number: u32,
        offset: u32,
        vrf_entropy: VrfEntropy,
    ) -> Result<Slot, BlockchainError> {
        // Fetch the latest macro block that precedes the block at the given block_number.
        // We use the disabled_slots set from that macro block for the slot selection.
        let macro_block = self.get_block_at(Policy::macro_block_before(block_number), true)?;
        let disabled_slots = macro_block
            .unwrap_macro()
            .body
            .unwrap()
            .next_batch_initial_punished_set;

        // Compute the slot number of the next proposer.
        let slot_number = Self::compute_slot_number(offset, vrf_entropy, disabled_slots);

        // Fetch the validators that are active in given block's epoch.
        let epoch_number = Policy::epoch_at(block_number);
        let validators = self.get_validators_for_epoch(epoch_number)?;

        // Get the validator that owns the proposer slot.
        let validator = validators.get_validator_by_slot_number(slot_number);

        // Also get the slot band for convenient access.
        let slot_band = validators.get_band_from_slot(slot_number);

        Ok(Slot {
            number: slot_number,
            band: slot_band,
            validator: validator.clone(),
        })
    }

    // FIXME Duplicated function (also exists in full blockchain)
    fn compute_slot_number(offset: u32, vrf_entropy: VrfEntropy, disabled_slots: BitSet) -> u16 {
        // RNG for slot selection
        let mut rng = vrf_entropy.rng(VrfUseCase::ViewSlotSelection);

        // Create a list of viable slots.
        let mut slots: Vec<u16> = if disabled_slots.len() == Policy::SLOTS as usize {
            // If all slots are disabled, we will accept any slot, since we want the
            // chain to progress.
            (0..Policy::SLOTS).collect()
        } else {
            // Otherwise, we will only accept slots that are not disabled.
            (0..Policy::SLOTS)
                .filter(|slot| !disabled_slots.contains(*slot as usize))
                .collect()
        };

        // Shuffle the slots vector using the Fisher–Yates shuffle.
        for i in (1..slots.len()).rev() {
            let r = rng.next_u64_below((i + 1) as u64) as usize;
            slots.swap(r, i);
        }

        // Now simply take the offset modulo the number of viable slots and that will give us
        // the chosen slot.
        slots[offset as usize % slots.len()]
    }
}
