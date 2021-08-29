use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use nimiq_account::Account;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::policy;
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    types::{Block, Inherent, SlashedSlots, Slot, Transaction},
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

    async fn get_latest_block(&mut self, include_transactions: bool) -> Result<Block, Error> {
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

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Transaction>, Error> {
        // Get all the extended transactions that correspond to this block.
        let extended_tx_vec = self
            .blockchain
            .history_store
            .get_block_transactions(block_number, None);

        let mut transactions = vec![];

        for ext_tx in extended_tx_vec {
            if !ext_tx.is_inherent() {
                transactions.push(Transaction::from_blockchain(
                    ext_tx.unwrap_basic().clone(),
                    ext_tx.block_number,
                    ext_tx.block_time,
                    self.blockchain.block_number(),
                ));
            }
        }

        Ok(transactions)
    }

    async fn get_batch_inherents(&mut self, batch_number: u32) -> Result<Vec<Inherent>, Error> {
        let macro_block_number = policy::macro_block_of(batch_number);

        // Check the batch's macro block to see if the batch includes slashes
        let macro_block = self
            .blockchain
            .get_block_at(macro_block_number, true, None) // The lost_reward_set is in the MacroBody
            .ok_or_else(|| Error::BlockNotFound(macro_block_number.into()))?;

        let mut inherent_tx_vec = vec![];

        let macro_body = macro_block.unwrap_macro().body.unwrap();

        if !macro_body.lost_reward_set.is_empty() {
            // Search all micro blocks of the batch to find the slash inherents
            let first_micro_block = policy::first_block_of_batch(batch_number);
            let last_micro_block = macro_block_number - 1;

            for i in first_micro_block..last_micro_block {
                let micro_ext_tx_vec = self
                    .blockchain
                    .history_store
                    .get_block_transactions(i, None);

                for ext_tx in micro_ext_tx_vec {
                    if ext_tx.is_inherent() {
                        inherent_tx_vec.push(ext_tx);
                    }
                }
            }
        }

        // Append inherents of the macro block (we do this after the micro blocks so the inherents are in order)
        inherent_tx_vec.append(
            &mut self
                .blockchain
                .history_store
                .get_block_transactions(macro_block_number, None)
                .into_iter()
                // Macro blocks include validator rewards as regular transactions, filter them out
                .filter(|ext_tx| ext_tx.is_inherent())
                .collect(),
        );

        Ok(inherent_tx_vec
            .into_iter()
            .map(|ext_tx| {
                Inherent::from_transaction(
                    ext_tx.unwrap_inherent().clone(),
                    ext_tx.block_number,
                    ext_tx.block_time,
                )
            })
            .collect())
    }

    async fn get_transaction_receipt(&mut self, _hash: Blake2bHash) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<Blake2bHash>, Error> {
        Ok(self.blockchain.history_store.get_tx_hashes_by_address(
            &address,
            max.unwrap_or(500),
            None,
        ))
    }

    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<Transaction>, Error> {
        let tx_hashes = self.get_transaction_hashes_by_address(address, max).await?;

        let mut txs = vec![];

        // TODO: Use a single database transaction for all queries

        for tx_hash in tx_hashes {
            txs.push(self.get_transaction_by_hash(tx_hash).await?);
        }

        Ok(txs)
    }

    async fn list_stakes(&mut self) -> Result<Vec<Address>, Error> {
        let staking_contract = self.blockchain.get_staking_contract();

        // let active_validators = staking_contract
        //     .active_validators_by_id
        //     .iter()
        //     .map(|(_, validator)| Validator::from_active(validator))
        //     .collect();
        //
        // let inactive_validators = staking_contract
        //     .inactive_validators_by_id
        //     .iter()
        //     .map(|(_, validator)| Validator::from_inactive(validator))
        //     .collect();
        //
        // let inactive_stakes = staking_contract
        //     .inactive_stake_by_address
        //     .iter()
        //     .map(|(address, stake)| Stake {
        //         staker_address: address.clone(),
        //         balance: stake.balance,
        //         retire_time: Some(stake.retire_time),
        //     })
        //     .collect();

        Ok(vec![])
    }

    #[stream]
    async fn head_subscribe(&mut self) -> Result<BoxStream<'static, Blake2bHash>, Error> {
        let stream = self.blockchain.notifier.write().as_stream();
        Ok(stream
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

    async fn get_account(&mut self, address: Address) -> Result<Option<Account>, Error> {
        let account = self.blockchain.get_account(&address);
        match account {
            Some(Account::Staking(_)) => Err(Error::GetAccountUnsupportedStakingContract),
            _ => Ok(account),
        }
    }
}
