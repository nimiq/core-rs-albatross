use std::sync::Arc;

use async_trait::async_trait;

use nimiq_blockchain_albatross::Blockchain;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;

use crate::{
    Error,
    types::{Block, OrLatest, Slot, SlashedSlots, Stakes, Validator, Stake},
};


#[async_trait]
pub trait BlockchainInterface {
    async fn block_number(&self) -> Result<u32, Error>;

    async fn epoch_number(&self) -> Result<u32, Error>;

    async fn batch_number(&self) -> Result<u32, Error>;

    async fn block_by_hash(&self, hash: Blake2bHash, include_transactions: bool) -> Result<Block, Error>;

    async fn block_by_number(&self, block_number: OrLatest<u32>, include_transactions: bool) -> Result<Block, Error>;

    async fn get_slot_at(&self, block_number: u32, view_number: Option<u32>) -> Result<Slot, Error>;

    // TODO: Previously called `slot_state`. Where is this used?
    async fn slashed_slots(&self) -> Result<SlashedSlots, Error>;

    async fn get_raw_transaction_info(&self, raw_tx: String) -> Result<(), Error>;

    async fn get_transaction_by_hash(&self, hash: Blake2bHash) -> Result<(), Error>;

    async fn get_transaction_receipt(&self, hash: Blake2bHash) -> Result<(), Error>;

    async fn list_stakes(&self) -> Result<Stakes, Error>;
}


pub struct BlockchainDispatcher {
    blockchain: Arc<Blockchain>,
}

impl BlockchainDispatcher {
    pub fn new(blockchain: Arc<Blockchain>) -> Self {
        Self {
            blockchain
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all="mixedCase")]
#[async_trait]
impl BlockchainInterface for BlockchainDispatcher {
    async fn block_number(&self) -> Result<u32, Error> {
        Ok(self.blockchain.block_number())
    }

    async fn epoch_number(&self) -> Result<u32, Error> {
        Ok(policy::epoch_at(self.blockchain.block_number()))
    }

    async fn batch_number(&self) -> Result<u32, Error> {
        Ok(policy::batch_at(self.blockchain.block_number()))
    }

    async fn block_by_hash(&self, hash: Blake2bHash, include_transactions: bool) -> Result<Block, Error> {
        self.blockchain
            .get_block(&hash, true)
            .map(|block| Block::from_block(&self.blockchain, block, include_transactions))
            .ok_or_else(|| Error::BlockNotFound(hash.into()))
    }

    async fn block_by_number(&self, block_number: OrLatest<u32>, include_transactions: bool) -> Result<Block, Error> {
        let block = match block_number {
            OrLatest::Value(block_number) => {
                self.blockchain
                    .get_block_at(block_number, true)
                    .ok_or_else(|| Error::BlockNotFound(block_number.into()))?
            },
            OrLatest::Latest => {
                self.blockchain.head().clone()
            }
        };

        Ok(Block::from_block(&self.blockchain, block, include_transactions))
    }

    async fn get_slot_at(&self, block_number: u32, view_number_opt: Option<u32>) -> Result<Slot, Error> {
        // Check if it's not a macro block
        //
        // TODO: Macro blocks have a slot too. It's just only for the proposal.
        if policy::is_macro_block_at(block_number) {
            return Err(Error::UnexpectedMacroBlock(block_number.into()));
        }

        let view_number = if let Some(view_number) = view_number_opt {
            view_number
        }
        else {
            self
                .blockchain
                .chain_store
                .get_block_at(block_number, false, None)
                .ok_or_else(|| Error::BlockNotFound(block_number.into()))?
                .view_number()
        };

        Ok(Slot::from_producer(&self.blockchain, block_number, view_number))
    }

    async fn slashed_slots(&self) -> Result<SlashedSlots, Error> {
        // FIXME: Race condition
        let block_number = self.blockchain.block_number();
        let staking_contract = self.blockchain.get_staking_contract();

        let current_slashed_set =
            staking_contract.current_lost_rewards() & staking_contract.current_disabled_slots();

        let previous_slashed_set =
            staking_contract.previous_lost_rewards() & staking_contract.previous_disabled_slots();

        Ok(SlashedSlots {
            block_number,
            current: current_slashed_set,
            previous: previous_slashed_set,
        })
    }

    async fn get_raw_transaction_info(&self, _raw_tx: String) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn get_transaction_by_hash(&self, _hash: Blake2bHash) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn get_transaction_receipt(&self, _hash: Blake2bHash) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn list_stakes(&self) -> Result<Stakes, Error> {
        let staking_contract = self.blockchain.get_staking_contract();

        let active_validators = staking_contract.active_validators_by_key
            .iter()
            .map(|(public_key, validator)| Validator::from_active(public_key.clone(), &validator))
            .collect();

        let inactive_validators = staking_contract.inactive_validators_by_key
            .iter()
            .map(|(public_key, validator)| Validator::from_inactive(public_key.clone(), &validator))
            .collect();

        let inactive_stakes = staking_contract.inactive_stake_by_address
            .iter()
            .map(|(address, stake)| Stake {
                staker_address: address.clone(),
                balance: stake.balance,
                retire_time: Some(stake.retire_time),
            })
            .collect();

        Ok(Stakes {
            active_validators,
            inactive_validators,
            inactive_stakes
        })
    }
}
