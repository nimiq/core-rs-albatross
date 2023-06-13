use async_trait::async_trait;
use futures::{future, stream::BoxStream, StreamExt};
use nimiq_account::{BlockLog as BBlockLog, TransactionLog};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::policy::Policy;
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    types::{
        is_of_log_type_and_related_to_addresses, Account, Block, BlockLog, BlockchainState,
        ExecutedTransaction, Inherent, LogType, ParkedSet, RPCData, RPCResult, SlashedSlots, Slot,
        Staker, Validator,
    },
};
use tokio_stream::wrappers::BroadcastStream;

use crate::error::Error;

pub struct BlockchainDispatcher {
    blockchain: BlockchainProxy,
}

impl BlockchainDispatcher {
    pub fn new(blockchain: BlockchainProxy) -> Self {
        Self { blockchain }
    }
}

/// Tries to fetch a block given its hash. It has an option to include the transactions in the
/// block, which defaults to false.
/// This function requires the read lock acquisition prior to its execution
fn get_block_by_hash(
    blockchain: &BlockchainReadProxy,
    hash: &Blake2bHash,
    include_body: Option<bool>,
) -> RPCResult<Block, (), Error> {
    let include_body = include_body.unwrap_or(matches!(blockchain, BlockchainReadProxy::Full(_)));

    blockchain
        .get_block(hash, include_body)
        .and_then(|block| Block::from_block(blockchain, block, include_body))
        .map_err(|_| Error::BlockNotFoundByHash(hash.clone()))
        .map(|block| block.into())
}

/// Tries to fetch a validator information given its address.
/// This function requires the read lock acquisition prior to its execution.
fn get_validator_by_address(
    blockchain_proxy: &BlockchainReadProxy,
    address: &Address,
) -> RPCResult<Validator, BlockchainState, Error> {
    if let BlockchainReadProxy::Full(blockchain) = blockchain_proxy {
        let staking_contract = blockchain.get_staking_contract();
        let data_store = blockchain.get_staking_contract_store();
        let db_txn = blockchain.read_transaction();
        let validator = staking_contract.get_validator(&data_store.read(&db_txn), address);

        if validator.is_none() {
            return Err(Error::ValidatorNotFound(address.clone()));
        }

        Ok(RPCData::with_blockchain(
            Validator::from_validator(&validator.unwrap()),
            blockchain_proxy,
        ))
    } else {
        Err(Error::NotSupportedForLightBlockchain)
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl BlockchainInterface for BlockchainDispatcher {
    type Error = Error;

    /// Returns the block number for the current head.
    async fn get_block_number(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(self.blockchain.read().block_number().into())
    }

    /// Returns the batch number for the current head.
    async fn get_batch_number(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::batch_at(self.blockchain.read().block_number()).into())
    }

    /// Returns the epoch number for the current head.
    async fn get_epoch_number(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::epoch_at(self.blockchain.read().block_number()).into())
    }

    /// Tries to fetch a block given its hash. It has an option to include the transactions in the
    /// block, which defaults to false.
    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error> {
        get_block_by_hash(&self.blockchain.read(), &hash, include_body)
    }

    /// Tries to fetch a block given its number. It has an option to include the transactions in the
    /// block, which defaults to false. Note that this function will only fetch blocks that are part
    /// of the main chain.
    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error> {
        let blockchain = self.blockchain.read();

        let include_body =
            include_body.unwrap_or(matches!(blockchain, BlockchainReadProxy::Full(_)));

        let block = blockchain
            .get_block_at(block_number, include_body)
            .map_err(|_| Error::BlockNotFound(block_number))?;

        Ok(Block::from_block(&blockchain, block, include_body)
            .map_err(|_| Error::BlockNotFound(block_number))?
            .into())
    }

    /// Returns the block at the head of the main chain. It has an option to include the
    /// transactions in the block, which defaults to false.
    async fn get_latest_block(
        &mut self,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error> {
        let blockchain = self.blockchain.read();
        let block = blockchain.head();

        Ok(
            Block::from_block(&blockchain, block, include_body.unwrap_or(false))
                .expect("Should always have the head block.")
                .into(),
        )
    }

    /// Returns the information for the slot owner at the given block height and offset. The
    /// offset is optional, it will default to getting the offset for the existing block
    /// at the given height.
    /// We only have this information available for the last 2 batches at most.
    async fn get_slot_at(
        &mut self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> RPCResult<Slot, BlockchainState, Self::Error> {
        let blockchain = self.blockchain.read();

        let offset = if let Some(offset) = offset_opt {
            offset
        } else {
            let block = blockchain
                .get_block_at(block_number, false)
                .map_err(|_| Error::BlockNotFound(block_number))?;
            if let nimiq_block::Block::Macro(macro_block) = block {
                if let Some(proof) = macro_block.justification {
                    proof.round
                } else {
                    return Err(Error::UnexpectedMacroBlock(block_number));
                }
            } else {
                // Skip and micro block offset is block number
                block_number
            }
        };

        Ok(RPCData::with_blockchain(
            Slot::from(&blockchain, block_number, offset)
                .map_err(|_| Error::BlockNotFound(block_number))?,
            &blockchain,
        ))
    }

    /// Tries to fetch a transaction (including reward transactions) given its hash.
    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
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

            return match extended_tx.into_transaction() {
                Ok(tx) => Ok(ExecutedTransaction::from_blockchain(
                    tx,
                    block_number,
                    timestamp,
                    blockchain.block_number(),
                )
                .into()),
                Err(_) => Err(Error::TransactionNotFound(hash)),
            };
        } else {
            return Err(Error::NotSupportedForLightBlockchain);
        }
    }

    /// Returns all the transactions (including reward transactions) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
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
                if let Ok(tx) = ext_tx.into_transaction() {
                    transactions.push(ExecutedTransaction::from_blockchain(
                        tx,
                        block_number,
                        timestamp,
                        blockchain.block_number(),
                    ));
                }
            }

            Ok(transactions.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns all the inherents (including reward inherents) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
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
                    inherents.push(Inherent::from(
                        ext_tx.unwrap_inherent().clone(),
                        block_number,
                        timestamp,
                    ));
                }
            }

            Ok(inherents.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns all the transactions (including reward transactions) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Calculate the numbers for the micro blocks in the batch.
            let first_block = Policy::first_block_of_batch(batch_number).ok_or(
                Error::InvalidArgument("Batch number out of bounds".to_string()),
            )?;
            let last_block = Policy::macro_block_of(batch_number).ok_or(Error::InvalidArgument(
                "Batch number out of bounds".to_string(),
            ))?;

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
                    if let Ok(tx) = ext_tx.into_transaction() {
                        transactions.push(ExecutedTransaction::from_blockchain(
                            tx,
                            i,
                            timestamp,
                            blockchain.block_number(),
                        ));
                    }
                }
            }

            Ok(transactions.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns all the inherents (including reward inherents) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            let macro_block_number = Policy::macro_block_of(batch_number).ok_or(
                Error::InvalidArgument("Batch number out of bounds".to_string()),
            )?;

            // Check the batch's macro block to see if the batch includes slashes.
            let macro_block = blockchain
                .get_block_at(macro_block_number, true, None) // The lost_reward_set is in the MacroBody
                .map_err(|_| Error::BlockNotFound(macro_block_number))?;

            let mut inherent_tx_vec = vec![];

            let macro_body = macro_block.unwrap_macro().body.unwrap();

            if !macro_body.lost_reward_set.is_empty() {
                // Search all micro blocks of the batch to find the slash inherents.
                let first_micro_block = Policy::first_block_of_batch(batch_number).ok_or(
                    Error::InvalidArgument("Batch number out of bounds".to_string()),
                )?;
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
                    Inherent::from(
                        ext_tx.unwrap_inherent().clone(),
                        ext_tx.block_number,
                        ext_tx.block_time,
                    )
                })
                .collect::<Vec<_>>()
                .into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns the hashes for the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of hashes to
    /// fetch, it defaults to 500.
    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> RPCResult<Vec<Blake2bHash>, (), Self::Error> {
        if let BlockchainProxy::Full(blockchain) = &self.blockchain {
            Ok(blockchain
                .read()
                .history_store
                .get_tx_hashes_by_address(&address, max.unwrap_or(500), None)
                .into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of transactions
    /// to fetch, it defaults to 500.
    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Get the transaction hashes for this address.
            let tx_hashes = blockchain.history_store.get_tx_hashes_by_address(
                &address,
                max.unwrap_or(500),
                None,
            );

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

                if let Ok(tx) = extended_tx.into_transaction() {
                    txs.push(ExecutedTransaction::from_blockchain(
                        tx,
                        block_number,
                        timestamp,
                        blockchain.block_number(),
                    ));
                }
            }

            Ok(txs.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Tries to fetch the account at the given address.
    async fn get_account_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Account, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            if let Some(account) = blockchain.get_account_if_complete(&address) {
                Account::try_from_account(
                    address,
                    account,
                    BlockchainState::new(blockchain.block_number(), blockchain.head_hash()),
                )
                .map_err(Error::Core)
            } else {
                log::warn!("Could not get account for address");
                return Err(Error::NoConsensus);
            }
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns a collection of the currently active validator's addresses and balances.
    async fn get_active_validators(
        &mut self,
    ) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract =
                if let Some(contract) = blockchain.get_staking_contract_if_complete(None) {
                    contract
                } else {
                    return Err(Error::NoConsensus);
                };

            let mut active_validators = vec![];

            for (address, _) in staking_contract.active_validators {
                if let Ok(rpc_result) = get_validator_by_address(&blockchain_proxy, &address) {
                    active_validators.push(rpc_result.data);
                }
            }

            Ok(RPCData::with_blockchain(
                active_validators,
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns information about the currently slashed slots. This includes slots that lost rewards
    /// and that were disabled.
    async fn get_current_slashed_slots(
        &mut self,
    ) -> RPCResult<SlashedSlots, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            // FIXME: Race condition
            let block_number = blockchain.block_number();
            let staking_contract = blockchain.get_staking_contract();

            Ok(RPCData::with_blockchain(
                SlashedSlots {
                    block_number,
                    lost_rewards: staking_contract.current_lost_rewards(),
                    disabled: staking_contract.current_disabled_slots(),
                },
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns information about the slashed slots of the previous batch. This includes slots that
    /// lost rewards and that were disabled.
    async fn get_previous_slashed_slots(
        &mut self,
    ) -> RPCResult<SlashedSlots, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            // FIXME: Race condition
            let block_number = blockchain.block_number();
            let staking_contract = blockchain.get_staking_contract();

            Ok(RPCData::with_blockchain(
                SlashedSlots {
                    block_number,
                    lost_rewards: staking_contract.previous_batch_lost_rewards(),
                    disabled: staking_contract.previous_epoch_disabled_slots(),
                },
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Returns information about the currently parked validators.
    async fn get_parked_validators(
        &mut self,
    ) -> RPCResult<ParkedSet, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let block_number = blockchain.block_number();
            let staking_contract = blockchain.get_staking_contract();

            Ok(RPCData::with_blockchain(
                ParkedSet {
                    block_number,
                    validators: staking_contract.parked_set(),
                },
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Tries to fetch a validator information given its address.
    async fn get_validator_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Validator, BlockchainState, Self::Error> {
        get_validator_by_address(&self.blockchain.read(), &address)
    }

    /// Fetches all stakers for a given validator.
    /// IMPORTANT: This operation iterates over all stakers of the staking contract
    /// and thus is extremely computationally expensive.
    /// This function requires the read lock acquisition prior to its execution.
    async fn get_stakers_by_validator_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Vec<Staker>, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();

        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract = blockchain.get_staking_contract();
            let data_store = blockchain.get_staking_contract_store();
            let db_txn = blockchain.read_transaction();
            let staker =
                staking_contract.get_stakers_for_validator(&data_store.read(&db_txn), &address);

            Ok(RPCData::with_blockchain(
                staker.iter().map(Staker::from_staker).collect(),
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Tries to fetch a staker information given its address.
    async fn get_staker_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Staker, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract = blockchain.get_staking_contract();
            let data_store = blockchain.get_staking_contract_store();
            let db_txn = blockchain.read_transaction();
            let staker = staking_contract.get_staker(&data_store.read(&db_txn), &address);

            match staker {
                Some(s) => Ok(RPCData::with_blockchain(
                    Staker::from_staker(&s),
                    &blockchain_proxy,
                )),
                None => Err(Error::StakerNotFound(address)),
            }
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Subscribes to new block events (retrieves the full block).
    #[stream]
    async fn subscribe_for_head_block(
        &mut self,
        include_body: Option<bool>,
    ) -> Result<BoxStream<'static, RPCData<Block, ()>>, Self::Error> {
        let blockchain = self.blockchain.clone();
        let stream = self.subscribe_for_head_block_hash().await?;

        // Uses the stream to receive hashes of blocks and then requests the actual block.
        // If the block was reverted in between these steps, the stream won't emit any event.
        Ok(stream
            .filter_map(move |rpc_result| {
                let blockchain_rg = blockchain.read();
                let result = get_block_by_hash(
                    &blockchain_rg,
                    &rpc_result.data, //The data contains the hash
                    include_body,
                )
                .map_or_else(|_| None, Some);
                future::ready(result)
            })
            .boxed())
    }

    /// Subscribes to new block events (only retrieves the block hash).
    #[stream]
    async fn subscribe_for_head_block_hash(
        &mut self,
    ) -> Result<BoxStream<'static, RPCData<Blake2bHash, ()>>, Self::Error> {
        let stream = self.blockchain.read().notifier_as_stream();
        Ok(stream
            .filter_map(|event| {
                let result = match event {
                    BlockchainEvent::Extended(hash) => Some(hash.into()),
                    BlockchainEvent::HistoryAdopted(hash) => Some(hash.into()),
                    BlockchainEvent::Finalized(hash) => Some(hash.into()),
                    BlockchainEvent::EpochFinalized(hash) => Some(hash.into()),
                    BlockchainEvent::Rebranched(_, new_branch) => {
                        Some(new_branch.into_iter().last().unwrap().0.into())
                    }
                };
                future::ready(result)
            })
            .boxed())
    }

    /// Subscribes to pre epoch validators events.
    #[stream]
    async fn subscribe_for_validator_election_by_address(
        &mut self,
        address: Address,
    ) -> Result<BoxStream<'static, RPCData<Validator, BlockchainState>>, Self::Error> {
        let blockchain = self.blockchain.clone();
        let stream = self.blockchain.read().notifier_as_stream();

        Ok(stream
            .filter_map(move |event| {
                let result = match event {
                    BlockchainEvent::EpochFinalized(..) => {
                        let blockchain_rg = blockchain.read();
                        get_validator_by_address(&blockchain_rg, &address)
                            .map_or_else(|_| None, Some)
                    }
                    _ => None,
                };
                future::ready(result)
            })
            .boxed())
    }

    /// Subscribes to log events related to a given list of addresses and of any of the log types provided.
    /// If addresses is empty it does not filter by address. If log_types is empty it won't filter by log types.
    /// Thus the behavior is to assume all addresses or log_types are to be provided if the corresponding vec is empty.
    #[stream]
    async fn subscribe_for_logs_by_addresses_and_types(
        &mut self,
        addresses: Vec<Address>,
        log_types: Vec<LogType>,
    ) -> Result<BoxStream<'static, RPCData<BlockLog, BlockchainState>>, Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            let stream = BroadcastStream::new(blockchain.log_notifier.subscribe());

            if addresses.is_empty() && log_types.is_empty() {
                Ok(Box::pin(stream.boxed().filter_map(|event| {
                    let result = match event {
                        Ok(event) => Some(RPCData::with_block_log(event)),
                        Err(_) => None,
                    };
                    future::ready(result)
                })))
            } else {
                Ok(stream
                    .filter_map(move |event| {
                        let result = match event {
                            Ok(BBlockLog::AppliedBlock {
                                mut inherent_logs,
                                block_hash,
                                block_number,
                                timestamp,
                                tx_logs,
                                total_tx_size: _,
                            }) => {
                                // Collects the inherents that are related to any of the addresses specified and of any of the log types provided.
                                inherent_logs.retain(|log| {
                                    is_of_log_type_and_related_to_addresses(
                                        log, &addresses, &log_types,
                                    )
                                });
                                // Since each TransactionLog has its own vec of logs, we iterate over each tx_logs and filter their logs,
                                // if a tx_log has no logs after filtering, it will be filtered out completely.
                                let tx_logs: Vec<TransactionLog> = tx_logs
                                    .into_iter()
                                    .filter_map(|mut tx_log| {
                                        tx_log.logs.retain(|log| {
                                            is_of_log_type_and_related_to_addresses(
                                                log, &addresses, &log_types,
                                            )
                                        });
                                        if tx_log.logs.is_empty() {
                                            None
                                        } else {
                                            Some(tx_log)
                                        }
                                    })
                                    .collect();

                                // If this block has no transaction logs or inherent logs of interest, we return None. Otherwise, we return the filtered BlockLog.
                                // This way the stream only emits an event if a block has at least one log fulfilling the specified criteria.
                                if !inherent_logs.is_empty() || !tx_logs.is_empty() {
                                    Some(RPCData::new(
                                        BlockLog::AppliedBlock {
                                            inherent_logs,
                                            timestamp,
                                            tx_logs,
                                        },
                                        BlockchainState {
                                            block_number,
                                            block_hash,
                                        },
                                    ))
                                } else {
                                    None
                                }
                            }
                            Ok(BBlockLog::RevertedBlock {
                                mut inherent_logs,
                                block_hash,
                                block_number,
                                tx_logs,
                                total_tx_size: _,
                            }) => {
                                // Filters the inherents and tx_logs the same way as the AppliedBlock
                                inherent_logs.retain(|log| {
                                    is_of_log_type_and_related_to_addresses(
                                        log, &addresses, &log_types,
                                    )
                                });
                                let tx_logs: Vec<TransactionLog> = tx_logs
                                    .into_iter()
                                    .filter_map(|mut tx_log| {
                                        tx_log.logs.retain(|log| {
                                            is_of_log_type_and_related_to_addresses(
                                                log, &addresses, &log_types,
                                            )
                                        });
                                        if tx_log.logs.is_empty() {
                                            None
                                        } else {
                                            Some(tx_log)
                                        }
                                    })
                                    .collect();

                                if !inherent_logs.is_empty() || !tx_logs.is_empty() {
                                    Some(RPCData::new(
                                        BlockLog::RevertedBlock {
                                            inherent_logs,
                                            tx_logs,
                                        },
                                        BlockchainState {
                                            block_number,
                                            block_hash,
                                        },
                                    ))
                                } else {
                                    None
                                }
                            }
                            Err(_) => None,
                        };
                        future::ready(result)
                    })
                    .boxed())
            }
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }
}
