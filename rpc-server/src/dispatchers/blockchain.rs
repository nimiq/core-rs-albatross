use std::{collections::HashMap, ops::Deref, sync::Arc};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use parking_lot::RwLock;

use nimiq_account::StakingContract;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, policy};
use nimiq_rpc_interface::types::{ParkedSet, Validator};
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    types::{Account, Block, Inherent, SlashedSlots, Slot, Staker, Transaction},
};

use crate::error::Error;

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

    /// Returns the block number for the current head.
    async fn get_block_number(&mut self) -> Result<u32, Error> {
        Ok(self.blockchain.read().block_number())
    }

    /// Returns the batch number for the current head.
    async fn get_batch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::batch_at(self.blockchain.read().block_number()))
    }

    /// Returns the epoch number for the current head.
    async fn get_epoch_number(&mut self) -> Result<u32, Error> {
        Ok(policy::epoch_at(self.blockchain.read().block_number()))
    }

    /// Tries to fetch a block given its hash. It has an option to include the transactions in the
    /// block, which defaults to false.
    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_transactions: Option<bool>,
    ) -> Result<Block, Error> {
        let blockchain = self.blockchain.read();

        blockchain
            .get_block(&hash, true, None)
            .map(|block| {
                Block::from_block(
                    blockchain.deref(),
                    block,
                    include_transactions.unwrap_or(false),
                )
            })
            .ok_or_else(|| Error::BlockNotFound(hash.into()))
    }

    /// Tries to fetch a block given its number. It has an option to include the transactions in the
    /// block, which defaults to false. Note that this function will only fetch blocks that are part
    /// of the main chain.
    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_transactions: Option<bool>,
    ) -> Result<Block, Error> {
        let blockchain = self.blockchain.read();

        let block = blockchain
            .get_block_at(block_number, true, None)
            .ok_or_else(|| Error::BlockNotFound(block_number.into()))?;

        Ok(Block::from_block(
            blockchain.deref(),
            block,
            include_transactions.unwrap_or(false),
        ))
    }

    /// Returns the block at the head of the main chain. It has an option to include the
    /// transactions in the block, which defaults to false.
    async fn get_latest_block(
        &mut self,
        include_transactions: Option<bool>,
    ) -> Result<Block, Error> {
        let blockchain = self.blockchain.read();
        let block = blockchain.head();

        Ok(Block::from_block(
            blockchain.deref(),
            block,
            include_transactions.unwrap_or(false),
        ))
    }

    /// Returns the information for the slot owner at the given block height and view number. The
    /// view number is optional, it will default to getting the view number for the existing block
    /// at the given height.
    async fn get_slot_at(
        &mut self,
        block_number: u32,
        view_number_opt: Option<u32>,
    ) -> Result<Slot, Error> {
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

        Ok(Slot::from(blockchain.deref(), block_number, view_number))
    }

    /// Returns all the transactions (including reward transactions) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Transaction>, Error> {
        let blockchain = self.blockchain.read();

        // Get all the extended transactions that correspond to this block.
        let extended_tx_vec = blockchain
            .history_store
            .get_block_transactions(block_number, None);

        // Get the timestamp of the block from one of the extended transactions. This complicated
        // setup is because we might not have any transactions.
        let timestamp = extended_tx_vec.first().map(|x| x.block_time).unwrap_or(0);

        // Convert the extended transactions into regular transactions. This will also convert
        // reward inherents.
        let mut transactions = vec![];

        for ext_tx in extended_tx_vec {
            match ext_tx.into_transaction() {
                Ok(tx) => {
                    transactions.push(Transaction::from_blockchain(
                        tx,
                        block_number,
                        timestamp,
                        blockchain.block_number(),
                    ));
                }
                Err(_) => {}
            }
        }

        Ok(transactions)
    }

    /// Returns all the inherents (including reward inherents) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Inherent>, Self::Error> {
        let blockchain = self.blockchain.read();

        // Get all the extended transactions that correspond to this block.
        let extended_tx_vec = blockchain
            .history_store
            .get_block_transactions(block_number, None);

        // Get the timestamp of the block from one of the extended transactions. This complicated
        // setup is because we might not have any transactions.
        let timestamp = extended_tx_vec.first().map(|x| x.block_time).unwrap_or(0);

        // Get only the inherents. This includes reward inherents.
        let mut inherents = vec![];

        for ext_tx in extended_tx_vec {
            if ext_tx.is_inherent() {
                inherents.push(Inherent::from_transaction(
                    ext_tx.unwrap_inherent().clone(),
                    block_number,
                    timestamp,
                ));
            }
        }

        Ok(inherents)
    }

    /// Returns all the transactions (including reward transactions) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> Result<Vec<Transaction>, Self::Error> {
        let blockchain = self.blockchain.read();

        // Calculate the numbers for the micro blocks in the batch.
        let first_block = policy::first_block_of_batch(batch_number);
        let last_block = policy::macro_block_of(batch_number);

        // Search all micro blocks of the batch to find the transactions.
        let mut transactions = vec![];

        for i in first_block..=last_block {
            let ext_txs = blockchain.history_store.get_block_transactions(i, None);

            // Get the timestamp of the block from one of the extended transactions. This complicated
            // setup is because we might not have any transactions.
            let timestamp = ext_txs.first().map(|x| x.block_time).unwrap_or(0);

            // Convert the extended transactions into regular transactions. This will also convert
            // reward inherents.
            for ext_tx in ext_txs {
                match ext_tx.into_transaction() {
                    Ok(tx) => {
                        transactions.push(Transaction::from_blockchain(
                            tx,
                            i,
                            timestamp,
                            blockchain.block_number(),
                        ));
                    }
                    Err(_) => {}
                }
            }
        }

        Ok(transactions)
    }

    /// Returns all the inherents (including reward inherents) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> Result<Vec<Inherent>, Error> {
        let blockchain = self.blockchain.read();

        let macro_block_number = policy::macro_block_of(batch_number);

        // Check the batch's macro block to see if the batch includes slashes.
        let macro_block = blockchain
            .get_block_at(macro_block_number, true, None) // The lost_reward_set is in the MacroBody
            .ok_or_else(|| Error::BlockNotFound(macro_block_number.into()))?;

        let mut inherent_tx_vec = vec![];

        let macro_body = macro_block.unwrap_macro().body.unwrap();

        if !macro_body.lost_reward_set.is_empty() {
            // Search all micro blocks of the batch to find the slash inherents.
            let first_micro_block = policy::first_block_of_batch(batch_number);
            let last_micro_block = macro_block_number - 1;

            for i in first_micro_block..=last_micro_block {
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

    /// Returns the hashes for the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of hashes to
    /// fetch, it defaults to 500.
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

    /// Returns the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of transactions
    /// to fetch, it defaults to 500.
    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<Transaction>, Error> {
        let blockchain = self.blockchain.read();

        // Get the transaction hashes for this address.
        let tx_hashes =
            blockchain
                .history_store
                .get_tx_hashes_by_address(&address, max.unwrap_or(500), None);

        let mut txs = vec![];

        for hash in tx_hashes {
            // Get all the extended transactions that correspond to this hash.
            let mut extended_tx_vec = blockchain.history_store.get_ext_tx_by_hash(&hash, None);

            // Unpack the transaction or raise an error.
            let extended_tx = match extended_tx_vec.len() {
                0 => {
                    return Err(Error::TransactionNotFound(hash));
                }
                1 => extended_tx_vec.pop().unwrap(),
                _ => {
                    return Err(Error::MultipleTransactionsFound(hash));
                }
            };

            // Convert the extended transaction into a regular transaction. This will also convert
            // reward inherents.
            let block_number = extended_tx.block_number;
            let timestamp = extended_tx.block_time;

            match extended_tx.into_transaction() {
                Ok(tx) => {
                    txs.push(Transaction::from_blockchain(
                        tx,
                        block_number,
                        timestamp,
                        blockchain.block_number(),
                    ));
                }
                Err(_) => {}
            }
        }

        Ok(txs)
    }

    /// Tries to fetch the account at the given address.
    async fn get_account_by_address(&mut self, address: Address) -> Result<Account, Error> {
        let result = self.blockchain.read().get_account(&address);

        match result {
            Some(account) => Ok(Account::from_account(address, account)),
            None => Ok(Account::empty(address)),
        }
    }

    /// Returns a map of the currently active validator's addresses and balances.
    async fn get_active_validators(&mut self) -> Result<HashMap<Address, Coin>, Error> {
        let staking_contract = self.blockchain.read().get_staking_contract();

        let mut active_validators = HashMap::new();

        for (address, balance) in staking_contract.active_validators {
            active_validators.insert(address, balance);
        }

        Ok(active_validators)
    }

    /// Returns information about the currently slashed slots. This includes slots that lost rewards
    /// and that were disabled.
    async fn get_current_slashed_slots(&mut self) -> Result<SlashedSlots, Self::Error> {
        let blockchain = self.blockchain.read();

        // FIXME: Race condition
        let block_number = blockchain.block_number();
        let staking_contract = blockchain.get_staking_contract();

        Ok(SlashedSlots {
            block_number,
            lost_rewards: staking_contract.current_lost_rewards(),
            disabled: staking_contract.current_disabled_slots(),
        })
    }

    /// Returns information about the slashed slots of the previous batch. This includes slots that
    /// lost rewards and that were disabled.
    async fn get_previous_slashed_slots(&mut self) -> Result<SlashedSlots, Self::Error> {
        let blockchain = self.blockchain.read();

        // FIXME: Race condition
        let block_number = blockchain.block_number();
        let staking_contract = blockchain.get_staking_contract();

        Ok(SlashedSlots {
            block_number,
            lost_rewards: staking_contract.previous_lost_rewards(),
            disabled: staking_contract.previous_disabled_slots(),
        })
    }

    /// Returns information about the currently parked validators.
    async fn get_parked_validators(&mut self) -> Result<ParkedSet, Self::Error> {
        let blockchain = self.blockchain.read();

        // FIXME: Race condition
        let block_number = blockchain.block_number();
        let staking_contract = blockchain.get_staking_contract();

        Ok(ParkedSet {
            block_number,
            validators: staking_contract.parked_set(),
        })
    }

    /// Tries to fetch a validator information given its address. It has an option to include a map
    /// containing the addresses and stakes of all the stakers that are delegating to the validator.
    async fn get_validator_by_address(
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

        if include_stakers == Some(true) {
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

    /// Tries to fetch a staker information given its address.
    async fn get_staker_by_address(&mut self, address: Address) -> Result<Staker, Error> {
        let blockchain = self.blockchain.read();

        let accounts_tree = &blockchain.state().accounts.tree;
        let db_txn = blockchain.read_transaction();
        let staker = StakingContract::get_staker(accounts_tree, &db_txn, &address);

        match staker {
            Some(s) => Ok(Staker::from_staker(&s)),
            None => Err(Error::StakerNotFound(address)),
        }
    }

    /// Subscribes to blockchain events.
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
}
