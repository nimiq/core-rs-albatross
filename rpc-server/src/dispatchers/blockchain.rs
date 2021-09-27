use std::{collections::HashMap, ops::Deref, sync::Arc};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use parking_lot::RwLock;

use nimiq_account::StakingContract;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, policy};
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    types::{Account, Block, Inherent, SlashedSlots, Slot, Staker, Transaction},
};

use crate::error::Error;
use nimiq_rpc_interface::types::Validator;

pub struct BlockchainDispatcher {
    blockchain: Arc<RwLock<Blockchain>>,
}

impl BlockchainDispatcher {
    pub fn new(blockchain: Arc<RwLock<Blockchain>>) -> Self {
        Self { blockchain }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl BlockchainInterface for BlockchainDispatcher {
    type Error = Error;

    async fn get_block_number(&mut self) -> Result<u32, Error> {
        Ok(self.blockchain.read().block_number())
    }

    async fn get_epoch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::epoch_at(self.blockchain.read().block_number()))
    }

    async fn get_batch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::batch_at(self.blockchain.read().block_number()))
    }

    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        let blockchain = self.blockchain.read();
        blockchain
            .get_block(&hash, true, None)
            .map(|block| Block::from_block(blockchain.deref(), block, include_transactions))
            .ok_or_else(|| Error::BlockNotFound(hash.into()))
    }

    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_transactions: bool,
    ) -> Result<Block, Error> {
        let blockchain = self.blockchain.read();
        let block = blockchain
            .get_block_at(block_number, true, None)
            .ok_or_else(|| Error::BlockNotFound(block_number.into()))?;

        Ok(Block::from_block(
            blockchain.deref(),
            block,
            include_transactions,
        ))
    }

    async fn get_latest_block(&mut self, include_transactions: bool) -> Result<Block, Error> {
        let blockchain = self.blockchain.read();
        let block = blockchain.head();

        Ok(Block::from_block(
            blockchain.deref(),
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

        let blockchain = self.blockchain.read();

        let view_number = if let Some(view_number) = view_number_opt {
            view_number
        } else {
            blockchain
                .chain_store
                .get_block_at(block_number, false, None)
                .ok_or_else(|| Error::BlockNotFound(block_number.into()))?
                .view_number()
        };

        Ok(Slot::from_producer(
            blockchain.deref(),
            block_number,
            view_number,
        ))
    }

    async fn get_slashed_slots(&mut self) -> Result<SlashedSlots, Error> {
        let blockchain = self.blockchain.read();

        // FIXME: Race condition
        let block_number = blockchain.block_number();
        let staking_contract = blockchain.get_staking_contract();

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
        let blockchain = self.blockchain.read();

        // Get all the extended transactions that correspond to this hash.
        let mut extended_tx_vec = blockchain.history_store.get_ext_tx_by_hash(&hash, None);

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
            blockchain.block_number(),
        ))
    }

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Transaction>, Error> {
        let blockchain = self.blockchain.read();
        // Get all the extended transactions that correspond to this block.
        let extended_tx_vec = blockchain
            .history_store
            .get_block_transactions(block_number, None);

        let mut transactions = vec![];

        for ext_tx in extended_tx_vec {
            if !ext_tx.is_inherent() {
                transactions.push(Transaction::from_blockchain(
                    ext_tx.unwrap_basic().clone(),
                    ext_tx.block_number,
                    ext_tx.block_time,
                    blockchain.block_number(),
                ));
            }
        }

        Ok(transactions)
    }

    async fn get_batch_inherents(&mut self, batch_number: u32) -> Result<Vec<Inherent>, Error> {
        let blockchain = self.blockchain.read();

        let macro_block_number = policy::macro_block_of(batch_number);

        // Check the batch's macro block to see if the batch includes slashes
        let macro_block = blockchain
            .get_block_at(macro_block_number, true, None) // The lost_reward_set is in the MacroBody
            .ok_or_else(|| Error::BlockNotFound(macro_block_number.into()))?;

        let mut inherent_tx_vec = vec![];

        let macro_body = macro_block.unwrap_macro().body.unwrap();

        if !macro_body.lost_reward_set.is_empty() {
            // Search all micro blocks of the batch to find the slash inherents
            let first_micro_block = policy::first_block_of_batch(batch_number);
            let last_micro_block = macro_block_number - 1;

            for i in first_micro_block..last_micro_block {
                let micro_ext_tx_vec = blockchain.history_store.get_block_transactions(i, None);

                for ext_tx in micro_ext_tx_vec {
                    if ext_tx.is_inherent() {
                        inherent_tx_vec.push(ext_tx);
                    }
                }
            }
        }

        // Append inherents of the macro block (we do this after the micro blocks so the inherents are in order)
        inherent_tx_vec.append(
            &mut blockchain
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
        Ok(self
            .blockchain
            .read()
            .history_store
            .get_tx_hashes_by_address(&address, max.unwrap_or(500), None))
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

    async fn list_stakes(&mut self) -> Result<HashMap<Address, Coin>, Error> {
        let staking_contract = self.blockchain.read().get_staking_contract();

        let mut active_validators = HashMap::new();

        for (address, balance) in staking_contract.active_validators {
            active_validators.insert(address, balance);
        }

        Ok(active_validators)
    }

    async fn get_validator(
        &mut self,
        address: Address,
        include_stakers: Option<bool>,
    ) -> Result<Validator, Error> {
        let blockchain = self.blockchain.read();
        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();
        let validator = StakingContract::get_validator(accounts_tree, &db_txn, &address);

        if validator.is_none() {
            return Err(Error::ValidatorNotFound(address));
        }

        let mut stakers = None;

        if include_stakers.is_some() && include_stakers.unwrap() {
            let staker_addresses =
                StakingContract::get_validator_stakers(accounts_tree, &db_txn, &address);

            let mut stakers_map = HashMap::new();

            for address in staker_addresses {
                let staker = StakingContract::get_staker(accounts_tree, &db_txn, &address).unwrap();
                if !staker.active_stake.is_zero() {
                    stakers_map.insert(address, staker.active_stake);
                }
            }

            stakers = Some(stakers_map);
        }

        Ok(Validator::from_validator(&validator.unwrap(), stakers))
    }

    async fn get_staker(&mut self, address: Address) -> Result<Staker, Error> {
        let blockchain = self.blockchain.read();
        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();
        let staker = StakingContract::get_staker(accounts_tree, &db_txn, &address);

        match staker {
            Some(s) => Ok(Staker::from_staker(&s)),
            None => Err(Error::StakerNotFound(address)),
        }
    }

    #[stream]
    async fn head_subscribe(&mut self) -> Result<BoxStream<'static, Blake2bHash>, Error> {
        let stream = self.blockchain.write().notifier.as_stream();
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

    async fn get_account(&mut self, address: Address) -> Result<Account, Error> {
        let result = self.blockchain.read().get_account(&address);
        match result {
            Some(account) => match account {
                nimiq_account::Account::Staking(_) => {
                    Err(Error::GetAccountUnsupportedStakingContract)
                }
                _ => Ok(Account::from_account(address, account)),
            },
            None => Ok(Account::empty(address)),
        }
    }
}
