use block::{Block, MacroBlock, MicroBlock};
use database::{AsDatabaseBytes, Database, Environment, FromDatabaseValue, ReadTransaction, WriteTransaction};
use primitives::policy;
use primitives::validators::Slots;
use primitives::coin::Coin;

pub struct RewardPot<'env> {
    env: &'env Environment,
    reward_pot: Database<'env>,
}

impl<'env> RewardPot<'env> {
    const REWARD_POT_DB_NAME: &'static str = "RewardPot";
    const CURRENT_EPOCH_KEY: &'static str = "curr";
    const PREVIOUS_EPOCH_KEY: &'static str = "prev";

    pub fn new(env: &'env Environment) -> Self {
        let reward_pot = env.open_database(RewardPot::REWARD_POT_DB_NAME.to_string());

        Self {
            env,
            reward_pot,
        }
    }

    #[inline]
    pub fn commit_block(&self, block: &Block, slots: &Slots, txn: &mut WriteTransaction) {
        match block {
            Block::Macro(ref macro_block) => self.commit_macro_block(macro_block, txn),
            Block::Micro(ref micro_block) => self.commit_micro_block(micro_block, slots, txn),
        }
    }

    fn commit_macro_block(&self, block: &MacroBlock, txn: &mut WriteTransaction) {
        // TODO: Do we want to check that reward corresponds to the value in the MacroExtrinsics?
        let current_reward: u64 = txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0);
        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &0u64);
        txn.put(&self.reward_pot, Self::PREVIOUS_EPOCH_KEY, &current_reward);
    }

    fn commit_micro_block(&self, block: &MicroBlock, slots: &Slots, txn: &mut WriteTransaction) {
        // The total reward of a block is composed of the block reward, transaction fees and slashes.
        let mut reward = RewardPot::reward_for_block(block, slots);

        // Add to current reward pot of epoch.
        reward += Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0));
        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &u64::from(reward));
    }

    #[inline]
    pub fn revert_block(&self, block: &Block, slots: &Slots, txn: &mut WriteTransaction) {
        if let Block::Micro(ref block) = block {
            self.revert_micro_block(block, slots, txn)
        } else {
            // We should never revert macro blocks.
            unreachable!()
        }
    }

    fn revert_micro_block(&self, block: &MicroBlock, slots: &Slots, txn: &mut WriteTransaction) {
        // The total reward of a block is composed of the block reward, transaction fees and slashes.
        let mut reward = Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0));

        // Add to current reward pot of epoch.
        reward -= RewardPot::reward_for_block(block, slots);
        txn.put(&self.reward_pot, Self::CURRENT_EPOCH_KEY, &u64::from(reward));
    }

    fn reward_for_block(block: &MicroBlock, slots: &Slots) -> Coin {
        // The total reward of a block is composed of the block reward, transaction fees and slashes.
        let mut reward = policy::block_reward_at(block.header.block_number);

        // Transaction fees.
        let extrinsics = block.extrinsics.as_ref().unwrap();
        for transaction in extrinsics.transactions.iter() {
            reward += transaction.fee;
        }

        // Fork proofs (have already been validated).
        reward += match slots.slash_fine().checked_mul(extrinsics.fork_proofs.len() as u64) {
            Some(r) => r,
            None => unreachable!(),
        };

        reward
    }

    pub fn current_reward_pot(&self) -> Coin {
        let txn = ReadTransaction::new(self.env);
        Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::CURRENT_EPOCH_KEY).unwrap_or(0))
    }

    pub fn previous_reward_pot(&self) -> Coin {
        let txn = ReadTransaction::new(self.env);
        Coin::from_u64_unchecked(txn.get(&self.reward_pot, Self::PREVIOUS_EPOCH_KEY).unwrap_or(0))
    }
}
