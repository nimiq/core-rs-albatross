use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use nimiq_account::Account;
use nimiq_blockchain_albatross::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::policy;
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    types::{Block, SlashedSlots, Slot, Stake, Stakes, Transaction, Validator},
};

use crate::error::Error;

pub struct BlockchainDispatcher {
    blockchain: Arc<Blockchain>,
}

impl BlockchainDispatcher {
    pub fn new(blockchain: Arc<Blockchain>) -> Self {
        Self { blockchain }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl BlockchainInterface for BlockchainDispatcher {
    type Error = Error;

    async fn get_block_number(&mut self) -> Result<u32, Error> {
        Ok(self.blockchain.block_number())
    }

    async fn get_epoch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::epoch_at(self.blockchain.block_number()))
    }

    async fn get_batch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::batch_at(self.blockchain.block_number()))
    }

    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        self.blockchain
            .get_block(&hash, true, None)
            .map(|block| Block::from_block(&self.blockchain, block, include_transactions))
            .ok_or_else(|| Error::BlockNotFound(hash.into()))
    }

    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        let block = self
            .blockchain
            .get_block_at(block_number, true, None)
            .ok_or_else(|| Error::BlockNotFound(block_number.into()))?;

        Ok(Block::from_block(
            &self.blockchain,
            block,
            include_transactions,
        ))
    }

    async fn get_latest_block(
        &mut self,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        let block = self.blockchain.head();

        Ok(Block::from_block(
            &self.blockchain,
            block,
            include_transactions,
        ))
    }

    async fn get_slot_at(
        &mut self,
        block_number: u32,
        view_number_opt: Option<u32>,
    ) -> Result<Slot, Error> {
        // Check if it's not a macro block
        //
        // TODO: Macro blocks have a slot too. It's just only for the proposal.
        if policy::is_macro_block_at(block_number) {
            return Err(Error::UnexpectedMacroBlock(block_number.into()));
        }

        let view_number = if let Some(view_number) = view_number_opt {
            view_number
        } else {
            self.blockchain
                .chain_store
                .get_block_at(block_number, false, None)
                .ok_or_else(|| Error::BlockNotFound(block_number.into()))?
                .view_number()
        };

        Ok(Slot::from_producer(
            &self.blockchain,
            block_number,
            view_number,
        ))
    }

    async fn get_slashed_slots(&mut self) -> Result<SlashedSlots, Error> {
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

    async fn get_raw_transaction_info(&mut self, _raw_tx: String) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn get_transaction_by_hash(&mut self, hash: Blake2bHash) -> Result<Transaction, Error> {
        // TODO: Check mempool for the transaction, too

        let extended_tx = self
            .blockchain
            .history_store
            .get_ext_tx_by_hash(&hash, None)
            .ok_or(Error::TransactionNotFound(hash))?;

        let transaction = extended_tx.unwrap_basic(); // Because we found the extended_tx above, this cannot be None

        Ok(Transaction::from_blockchain(
            transaction.clone(),
            extended_tx.block_number,
            extended_tx.block_time,
            self.blockchain.block_number(),
        ))
    }

    async fn get_transaction_receipt(&mut self, _hash: Blake2bHash) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn list_stakes(&mut self) -> Result<Stakes, Error> {
        let staking_contract = self.blockchain.get_staking_contract();

        let active_validators = staking_contract
            .active_validators_by_id
            .iter()
            .map(|(_, validator)| Validator::from_active(&validator))
            .collect();

        let inactive_validators = staking_contract
            .inactive_validators_by_id
            .iter()
            .map(|(_, validator)| Validator::from_inactive(&validator))
            .collect();

        let inactive_stakes = staking_contract
            .inactive_stake_by_address
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
            inactive_stakes,
        })
    }

    #[stream]
    async fn head_subscribe(&mut self) -> Result<BoxStream<'static, Blake2bHash>, Error> {
        Ok(self
            .blockchain
            .notifier
            .write()
            .as_stream()
            .map(|event| match event {
                BlockchainEvent::Extended(hash) => hash,
                BlockchainEvent::Finalized(hash) => hash,
                BlockchainEvent::EpochFinalized(hash) => hash,
                BlockchainEvent::Rebranched(_, new_branch) => {
                    new_branch.into_iter().last().unwrap().0
                }
            })
            .boxed())
    }

    async fn get_account(&mut self, address: Address) -> Result<Account, Error> {
        let account = self.blockchain.get_account(&address);
        if matches!(account, Account::Staking(_)) {
            Err(Error::GetAccountUnsupportedStakingContract)
        }
        else {
            Ok(account)
        }
    }
}
