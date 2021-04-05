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
    types::{
        Block, ExtendedTransaction, Inherent, OrLatest, SlashedSlots, Slot, Stake, Stakes,
        Transaction, Validator,
    },
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

    async fn block_number(&mut self) -> Result<u32, Error> {
        Ok(self.blockchain.block_number())
    }

    async fn epoch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::epoch_at(self.blockchain.block_number()))
    }

    async fn batch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::batch_at(self.blockchain.block_number()))
    }

    async fn block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        self.blockchain
            .get_block(&hash, true, None)
            .map(|block| Block::from_block(&self.blockchain, block, include_transactions))
            .ok_or_else(|| Error::BlockNotFound(hash.into()))
    }

    async fn block_by_number(
        &mut self,
        block_number: OrLatest<u32>,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        let block = match block_number {
            OrLatest::Value(block_number) => self
                .blockchain
                .get_block_at(block_number, true, None)
                .ok_or_else(|| Error::BlockNotFound(block_number.into()))?,
            OrLatest::Latest => self.blockchain.head(),
        };

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

    async fn slashed_slots(&mut self) -> Result<SlashedSlots, Error> {
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

        // Get all the extended transactions that correspond to this hash.
        let mut extended_tx_vec = self
            .blockchain
            .history_store
            .get_ext_tx_by_hash(&hash, None);

        // If we get more than 1 extended transaction, we panic. This shouldn't happen.
        assert!(extended_tx_vec.len() < 2);

        // Unpack the transaction or raise an error.
        let extended_tx = if extended_tx_vec.is_empty() {
            return Err(Error::TransactionNotFound(hash));
        } else {
            extended_tx_vec.pop().unwrap()
        };

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

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<ExtendedTransaction>, Error> {
        // Get all the extended transactions that correspond to this block.
        let extended_tx_vec = self
            .blockchain
            .history_store
            .get_block_transactions(block_number, None);

        Ok(extended_tx_vec
            .into_iter()
            .enumerate()
            .map(|(_, extended_tx)| {
                if extended_tx.is_inherent() {
                    ExtendedTransaction::Inherent(Inherent::from_inherent(
                        extended_tx.unwrap_inherent().clone(),
                        extended_tx.block_number,
                        extended_tx.block_time,
                    ))
                } else {
                    let transaction = extended_tx.unwrap_basic();
                    ExtendedTransaction::Transaction(Transaction::from_blockchain(
                        transaction.clone(),
                        extended_tx.block_number,
                        extended_tx.block_time,
                        self.blockchain.block_number(),
                    ))
                }
            })
            .collect::<Vec<ExtendedTransaction>>())
    }

    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
        epoch_number: u32,
    ) -> Result<Vec<ExtendedTransaction>, Self::Error> {
        // Get all the extended transactions that correspond to this batch.
        let extended_tx_vec =
            self.blockchain
                .history_store
                .get_batch_transactions(batch_number, epoch_number, None);

        Ok(extended_tx_vec
            .into_iter()
            .enumerate()
            .map(|(_, extended_tx)| {
                if extended_tx.is_inherent() {
                    ExtendedTransaction::Inherent(Inherent::from_inherent(
                        extended_tx.unwrap_inherent().clone(),
                        extended_tx.block_number,
                        extended_tx.block_time,
                    ))
                } else {
                    let transaction = extended_tx.unwrap_basic();
                    ExtendedTransaction::Transaction(Transaction::from_blockchain(
                        transaction.clone(),
                        extended_tx.block_number,
                        extended_tx.block_time,
                        self.blockchain.block_number(),
                    ))
                }
            })
            .collect::<Vec<ExtendedTransaction>>())
    }

    async fn get_transactions_by_epoch_number(
        &mut self,
        epoch_number: u32,
    ) -> Result<Vec<ExtendedTransaction>, Error> {
        // Get all the extended transactions that correspond to this epoch.
        let extended_tx_vec = self
            .blockchain
            .history_store
            .get_epoch_transactions(epoch_number, None);

        Ok(extended_tx_vec
            .into_iter()
            .enumerate()
            .map(|(_, extended_tx)| {
                if extended_tx.is_inherent() {
                    ExtendedTransaction::Inherent(Inherent::from_inherent(
                        extended_tx.unwrap_inherent().clone(),
                        extended_tx.block_number,
                        extended_tx.block_time,
                    ))
                } else {
                    let transaction = extended_tx.unwrap_basic();
                    ExtendedTransaction::Transaction(Transaction::from_blockchain(
                        transaction.clone(),
                        extended_tx.block_number,
                        extended_tx.block_time,
                        self.blockchain.block_number(),
                    ))
                }
            })
            .collect::<Vec<ExtendedTransaction>>())
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

    async fn get_account(&mut self, account: Address) -> Result<Account, Error> {
        Ok(self.blockchain.get_account(&account))
    }
}
