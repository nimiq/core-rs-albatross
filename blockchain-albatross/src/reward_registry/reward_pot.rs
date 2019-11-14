use block::{MacroBlock, MicroBlock};
use database::{Database, Environment, ReadTransaction, WriteTransaction};
use primitives::coin::Coin;
use primitives::policy;
use transaction::Transaction as BlockchainTransaction;


pub struct RewardPot {
    env: Environment,
    reward_pot: Database,
}

impl RewardPot {
    const REWARD_POT_DB_NAME: &'static str = "RewardPot";
    const CURRENT_EPOCH_KEY: &'static str = "curr";
    const PREVIOUS_EPOCH_KEY: &'static str = "prev";

    pub fn new(env: Environment) -> Self {
        let reward_pot = env.open_database(RewardPot::REWARD_POT_DB_NAME.to_string());

        Self {
            env,
            reward_pot,
        }
    }

    pub(super) fn commit_macro_block(&self, block: &MacroBlock, txn: &mut WriteTransaction) {
        // TODO: Do we want to check that reward corresponds to the value in the MacroExtrinsics?
        let mut current_reward = RewardPot::reward_for_macro_block(block);

        // Add to current reward pot of epoch.
        current_reward += Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0));

        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &0u64);
        txn.put(&self.reward_pot, Self::PREVIOUS_EPOCH_KEY, &u64::from(current_reward));
    }

    pub(super) fn commit_epoch(&self, block_number: u32, transactions: &[BlockchainTransaction], txn: &mut WriteTransaction) {
        assert!(policy::is_macro_block_at(block_number));
        let epoch = policy::epoch_at(block_number);

        let mut reward = Coin::ZERO;

        // All blocks of the epoch.
        for block_number in policy::first_block_of(epoch)..=block_number {
            reward += policy::block_reward_at(block_number);
        }

        // All transactions.
        for transaction in transactions {
            reward += transaction.fee;
        }

        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &0u64);
        txn.put(&self.reward_pot, Self::PREVIOUS_EPOCH_KEY, &u64::from(reward));
    }

    pub(super) fn commit_micro_block(&self, block: &MicroBlock, txn: &mut WriteTransaction) {
        // The total reward of a block is composed of the block reward and transaction fees.
        let mut reward = RewardPot::reward_for_micro_block(block);

        // Add to current reward pot of epoch.
        reward += Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0));
        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &u64::from(reward));
    }

    pub(super) fn revert_micro_block(&self, block: &MicroBlock, txn: &mut WriteTransaction) {
        // The total reward of a block is composed of the block reward and transaction fees.
        let mut reward = Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0));

        // Add to current reward pot of epoch.
        reward -= RewardPot::reward_for_micro_block(block);

        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &u64::from(reward));
    }

    fn reward_for_micro_block(block: &MicroBlock) -> Coin {
        // Block reward
        let mut reward = policy::block_reward_at(block.header.block_number);

        // Transaction fees
        let extrinsics = block.extrinsics.as_ref().unwrap();
        for transaction in extrinsics.transactions.iter() {
            reward += transaction.fee;
        }

        reward
    }

    fn reward_for_macro_block(block: &MacroBlock) -> Coin {
        policy::block_reward_at(block.header.block_number)
    }

    pub fn current_reward_pot(&self) -> Coin {
        let txn = ReadTransaction::new(&self.env);
        Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0))
    }

    pub fn previous_reward_pot(&self) -> Coin {
        let txn = ReadTransaction::new(&self.env);
        Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::PREVIOUS_EPOCH_KEY).unwrap_or(0))
    }
}
