use block::{MacroBlock, MicroBlock};
use database::{Database, Environment, ReadTransaction, WriteTransaction};
use primitives::coin::Coin;
use primitives::policy;
use std::convert::TryInto;
use transaction::Transaction as BlockchainTransaction;

pub struct RewardPot {
    env: Environment,
    reward_pot: Database,
}

impl RewardPot {
    /// The database name.
    const REWARD_POT_DB_NAME: &'static str = "RewardPot";

    /// The reward for the current epoch.
    const CURRENT_REWARD_KEY: &'static str = "curr_reward";

    /// The reward for the previous epoch.
    const PREVIOUS_REWARD_KEY: &'static str = "prev_reward";

    /// The current supply.
    const SUPPLY_KEY: &'static str = "supply";

    /// The supply at the genesis block.
    const GENESIS_SUPPLY_KEY: &'static str = "gen_supply";

    /// The time at the genesis block.
    const GENESIS_TIME_KEY: &'static str = "gen_time";

    /// Creates a new RewardPot database.
    pub fn new(env: Environment) -> Self {
        let reward_pot = env.open_database(RewardPot::REWARD_POT_DB_NAME.to_string());

        Self { env, reward_pot }
    }

    /// Returns the reward for the current epoch.
    pub fn current_reward(&self) -> Coin {
        let txn = ReadTransaction::new(&self.env);

        Coin::from_u64_unchecked(
            txn.get(&self.reward_pot, Self::CURRENT_REWARD_KEY)
                .unwrap_or(0),
        )
    }

    /// Returns the reward for the previous epoch.
    pub fn previous_reward(&self) -> Coin {
        let txn = ReadTransaction::new(&self.env);

        Coin::from_u64_unchecked(
            txn.get(&self.reward_pot, Self::PREVIOUS_REWARD_KEY)
                .unwrap_or(0),
        )
    }

    /// Returns the current supply.
    pub fn supply(&self) -> Coin {
        let txn = ReadTransaction::new(&self.env);

        Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::SUPPLY_KEY).unwrap_or(0))
    }

    /// Updates the RewardPot database for an entire epoch. All we need is the timestamp (to calculate
    /// the coinbase) and the transaction list (to calculate the transaction fees).
    /// This function is used for the Macro Sync feature.
    pub(super) fn commit_epoch(
        &self,
        timestamp: u64,
        transactions: &[BlockchainTransaction],
        txn: &mut WriteTransaction,
    ) {
        // Start reward at zero.
        let mut reward = 0;

        // Calculate the reward for the macro block (which corresponds to the coinbase) and the new
        // supply.
        let (macro_block_reward, new_supply) = self.reward_for_macro_block(timestamp, txn);

        // Update the reward.
        reward += macro_block_reward;

        // Add all the transaction fees to the reward.
        for transaction in transactions {
            reward += u64::from(transaction.fee);
        }

        // Set the current reward to zero.
        txn.put(&self.reward_pot, Self::CURRENT_REWARD_KEY, &0u64);

        // Set the previous reward to the newly calculated reward.
        txn.put(
            &self.reward_pot,
            Self::PREVIOUS_REWARD_KEY,
            &u64::from(reward),
        );

        // Set the supply to the newly calculated supply.
        txn.put(&self.reward_pot, Self::SUPPLY_KEY, &new_supply);
    }

    /// Updates the RewardPot database for a macro block. It takes the whole macro block as input
    /// but all we need is the timestamp. If the macro block is the genesis block, then it will set
    /// the GENESIS_SUPPLY and GENESIS_TIME constants.
    /// This function is used for normal block syncing.
    pub(super) fn commit_macro_block(&self, block: &MacroBlock, txn: &mut WriteTransaction) {
        // This is supposed to be the tuple (current_reward, new_supply).
        let tuple: (u64, u64);

        // Get the timestamp from the block.
        let timestamp = block.header.timestamp;

        // Check if this is the genesis block.
        if block.header.block_number == 0 {
            // Get the supply from the extra_data field of the genesis block. We assume that the
            // first 8 bytes of the extra_data field will have the supply as a big-endian byte array.
            let extrinsics = block.extrinsics.as_ref().unwrap();

            let bytes = extrinsics.extra_data[..8]
                .try_into()
                .expect("slice has wrong size");

            let supply = u64::from_be_bytes(bytes);

            // Set the genesis supply to be the current supply.
            txn.put(&self.reward_pot, Self::GENESIS_SUPPLY_KEY, &supply);

            // Set the genesis time to be the current time.
            txn.put(&self.reward_pot, Self::GENESIS_TIME_KEY, &timestamp);

            // Initialize the tuple to have current_reward = 0 and new_supply = genesis_supply.
            tuple = (0, supply);
        } else {
            // Calculate the reward for the macro block (which corresponds to the coinbase) and the
            // new supply.
            tuple = self.reward_for_macro_block(timestamp, txn);
        }

        // Set the current reward to zero.
        txn.put(&self.reward_pot, Self::CURRENT_REWARD_KEY, &0u64);

        // Set the previous reward to the newly calculated reward.
        txn.put(&self.reward_pot, Self::PREVIOUS_REWARD_KEY, &tuple.0);

        // Set the supply to the newly calculated supply.
        txn.put(&self.reward_pot, Self::SUPPLY_KEY, &tuple.1);
    }

    /// Updates the RewardPot database for a micro block. It takes the whole micro block as input
    /// since we need all the transactions in it.
    /// This function is used for normal block syncing.
    pub(super) fn commit_micro_block(&self, block: &MicroBlock, txn: &mut WriteTransaction) {
        // Get the current reward from the RewardPot database.
        let mut current_reward = txn
            .get(&self.reward_pot, Self::CURRENT_REWARD_KEY)
            .unwrap_or(0);

        // Get the transactions from the block.
        let extrinsics = block.extrinsics.as_ref().unwrap();

        // Add all the transaction fees to the current reward.
        for transaction in extrinsics.transactions.iter() {
            current_reward += u64::from(transaction.fee);
        }

        // Set the current reward to the newly calculated reward.
        txn.put(&self.reward_pot, Self::CURRENT_REWARD_KEY, &current_reward);
    }

    /// Rollbacks the RewardPot database for a micro block. It takes the whole micro block as input
    /// since we need all the transactions in it.
    /// This function is used for normal block syncing.
    pub(super) fn revert_micro_block(&self, block: &MicroBlock, txn: &mut WriteTransaction) {
        // Get the current reward from the RewardPot database.
        let mut current_reward = txn
            .get(&self.reward_pot, Self::CURRENT_REWARD_KEY)
            .unwrap_or(0);

        // Get the transactions from the block.
        let extrinsics = block.extrinsics.as_ref().unwrap();

        // Subtract all the transaction fees from the current reward.
        for transaction in extrinsics.transactions.iter() {
            current_reward -= u64::from(transaction.fee);
        }

        // Set the current reward to the newly calculated reward.
        txn.put(&self.reward_pot, Self::CURRENT_REWARD_KEY, &current_reward);
    }

    /// Calculates the reward for a macro block (which is just the coinbase) and the new supply after
    /// the end of the corresponding epoch. All it needs is the timestamp of the macro block.
    pub(super) fn reward_for_macro_block(
        &self,
        timestamp: u64,
        txn: &mut WriteTransaction,
    ) -> (u64, u64) {
        // Get the current reward from the RewardPot database.
        let mut reward = txn
            .get(&self.reward_pot, Self::CURRENT_REWARD_KEY)
            .unwrap_or(0);

        // Get the genesis supply from the RewardPot database.
        let gen_supply = txn
            .get(&self.reward_pot, Self::GENESIS_SUPPLY_KEY)
            .unwrap_or(0);

        // Get the genesis time from the RewardPot database.
        let gen_time = txn
            .get(&self.reward_pot, Self::GENESIS_TIME_KEY)
            .unwrap_or(0);

        // Get the current supply from the RewardPot database.
        let current_supply = txn.get(&self.reward_pot, Self::SUPPLY_KEY).unwrap_or(0);

        // Calculate what the new supply should be using the supply curve formula from the
        // policy file.
        let new_supply = policy::supply_at(gen_supply, gen_time, timestamp);

        // Calculate the reward (coinbase) as the difference between the new supply and the
        // current supply.
        reward += new_supply - current_supply;

        // Return the reward and the new supply.
        (reward, new_supply)
    }
}
