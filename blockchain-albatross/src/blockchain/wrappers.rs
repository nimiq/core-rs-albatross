use parking_lot::{MutexGuard, RwLockReadGuard};

use nimiq_account::{Account, StakingContract};
use nimiq_block_albatross::Block;
use nimiq_database::WriteTransaction;
use nimiq_genesis::NetworkInfo;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::policy;
use nimiq_utils::observer::{Listener, ListenerHandle};

use crate::blockchain_state::BlockchainState;
#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::{AbstractBlockchain, Blockchain, BlockchainEvent, Direction};

/// Implements several wrapper functions.
impl Blockchain {
    /// Returns the current state (with a read transaction).
    pub fn state(&self) -> RwLockReadGuard<BlockchainState> {
        self.state.read()
    }

    /// Fetches a given number of blocks, starting at a specific block (by its hash).
    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
    ) -> Vec<Block> {
        self.chain_store
            .get_blocks(start_block_hash, count, include_body, direction, None)
    }

    /// Fetches a given number of macro blocks, starting at a specific block (by its hash).
    /// It can fetch only election macro blocks if desired.
    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
    ) -> Option<Vec<Block>> {
        self.chain_store.get_macro_blocks(
            start_block_hash,
            count,
            include_body,
            direction,
            election_blocks_only,
            None,
        )
    }

    /// Returns the current staking contract.
    pub fn get_staking_contract(&self) -> StakingContract {
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");

        let account = self.state.read().accounts.get(validator_registry, None);

        if let Account::Staking(x) = account {
            x
        } else {
            unreachable!("Account type must be Staking.")
        }
    }

    pub fn write_transaction(&self) -> WriteTransaction {
        WriteTransaction::new(&self.env)
    }

    pub fn register_listener<T: Listener<BlockchainEvent> + 'static>(
        &self,
        listener: T,
    ) -> ListenerHandle {
        self.notifier.write().register(listener)
    }

    pub fn get_account(&self, address: &Address) -> Account {
        self.state.read().accounts.get(address, None)
    }

    /// Checks if we have seen some transaction with this hash inside the validity window. This is
    /// used to prevent replay attacks.
    pub fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool {
        // Get a vector with all transactions corresponding to the given hash.
        let ext_hash_vec = self.history_store.get_ext_tx_by_hash(tx_hash, None);

        // If the vector is empty then we have never seen a transaction with this hash.
        if ext_hash_vec.is_empty() {
            return false;
        }

        // If we have transactions with this same hash then we must be sure that they are outside
        // the validity window.
        let max_block_number = self.block_number() - policy::TRANSACTION_VALIDITY_WINDOW;

        for ext_tx in ext_hash_vec {
            // If the transaction is inside the validity window, return true.
            if ext_tx.block_number >= max_block_number {
                return true;
            }
        }

        // If we didn't see any transaction inside the validity window then we can return false.
        false
    }

    pub fn validator_registry_address(&self) -> Option<&Address> {
        NetworkInfo::from_network_id(self.network_id).validator_registry_address()
    }

    pub fn lock(&self) -> MutexGuard<()> {
        self.push_lock.lock()
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &BlockchainMetrics {
        &self.metrics
    }
}
