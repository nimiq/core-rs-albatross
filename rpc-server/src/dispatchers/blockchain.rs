use async_trait::async_trait;
use futures::{future, stream::BoxStream, StreamExt};
use nimiq_account::{BlockLog as BBlockLog, TransactionLog};
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{key_nibbles::KeyNibbles, policy::Policy};
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    types::{
        is_of_log_type_and_related_to_addresses, Account, Block, BlockLog, BlockchainState,
        ExecutedTransaction, Inherent, LogType, PenalizedSlots, RPCData, RPCResult, Slot, Staker,
        Validator,
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
        let staking_contract = blockchain
            .get_staking_contract_if_complete(None)
            .ok_or(Error::NoConsensus)?;
        let data_store = blockchain.get_staking_contract_store();
        let db_txn = blockchain.read_transaction();
        let validator = staking_contract
            .get_validator(&data_store.read(&db_txn), address)
            .ok_or_else(|| Error::ValidatorNotFound(address.clone()))?;

        Ok(RPCData::with_blockchain(
            Validator::from_validator(&validator),
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

    async fn get_block_number(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(self.blockchain.read().block_number().into())
    }

    async fn get_batch_number(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::batch_at(self.blockchain.read().block_number()).into())
    }

    async fn get_epoch_number(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::epoch_at(self.blockchain.read().block_number()).into())
    }

    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error> {
        get_block_by_hash(&self.blockchain.read(), &hash, include_body)
    }

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

    async fn get_slot_at(
        &mut self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> RPCResult<Slot, BlockchainState, Self::Error> {
        let blockchain = self.blockchain.read();

        let offset = if let Some(offset) = offset_opt {
            offset
        } else {
            blockchain
                .get_block_at(block_number, false)
                .map_err(|_| Error::BlockNotFound(block_number))?
                .vrf_offset()
        };

        let slot = Slot::from_block_number(&blockchain, block_number, offset)
            .map_err(|_| Error::BlockNotFound(block_number))?;

        Ok(RPCData::with_blockchain(slot, &blockchain))
    }

    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Get all the historic transactions that correspond to this hash.
            let mut historic_tx_vec = blockchain.history_store.get_hist_tx_by_hash(&hash, None);

            // Unpack the transaction or raise an error.
            let historic_tx = match historic_tx_vec.len() {
                0 => {
                    return Err(Error::TransactionNotFound(hash));
                }
                1 => historic_tx_vec.pop().unwrap(),
                _ => {
                    return Err(Error::MultipleTransactionsFound(hash));
                }
            };

            // Convert the historic transaction into a regular transaction. This will also convert
            // reward inherents.
            let block_number = historic_tx.block_number;
            let timestamp = historic_tx.block_time;

            return match historic_tx.into_transaction() {
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

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Get all the historic transactions that correspond to this block.
            let historic_tx_vec = blockchain
                .history_store
                .get_block_transactions(block_number, None);

            // Get the timestamp of the block from one of the historic transactions. This complicated
            // setup is because we might not have any transactions.
            let timestamp = historic_tx_vec.first().map(|x| x.block_time).unwrap_or(0);

            // Convert the historic transactions into regular transactions. This will also convert
            // reward inherents.
            let mut transactions = vec![];

            for hist_tx in historic_tx_vec {
                if let Ok(tx) = hist_tx.into_transaction() {
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

    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Get all the historic transactions that correspond to this block.
            let historic_tx_vec = blockchain
                .history_store
                .get_block_transactions(block_number, None);

            // Get only the inherents. This includes reward inherents.
            let mut inherents = vec![];

            for hist_tx in historic_tx_vec {
                if let Some(inherent) = Inherent::try_from(hist_tx) {
                    inherents.push(inherent);
                }
            }

            Ok(inherents.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

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
                let hist_txs = blockchain.history_store.get_block_transactions(i, None);

                // Get the timestamp of the block from one of the historic transactions. This complicated
                // setup is because we might not have any transactions.
                let timestamp = hist_txs.first().map(|x| x.block_time).unwrap_or(0);

                // Convert the historic transactions into regular transactions. This will also convert
                // reward inherents.
                for hist_tx in hist_txs {
                    if let Ok(tx) = hist_tx.into_transaction() {
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

    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            let macro_block_number = Policy::macro_block_of(batch_number).ok_or(
                Error::InvalidArgument("Batch number out of bounds".to_string()),
            )?;

            let mut inherent_tx_vec = vec![];

            // Search all micro blocks of the batch to find the punishment inherents.
            let first_micro_block = Policy::first_block_of_batch(batch_number).ok_or(
                Error::InvalidArgument("Batch number out of bounds".to_string()),
            )?;
            let last_micro_block = macro_block_number - 1;

            for i in first_micro_block..=last_micro_block {
                let micro_hist_tx_vec = blockchain.history_store.get_block_transactions(i, None);

                for hist_tx in micro_hist_tx_vec {
                    if let Some(inherent) = Inherent::try_from(hist_tx) {
                        inherent_tx_vec.push(inherent);
                    }
                }
            }

            // Append inherents of the macro block (we do this after the micro blocks so the inherents are in order)
            inherent_tx_vec.extend(
                blockchain
                    .history_store
                    .get_block_transactions(macro_block_number, None)
                    .into_iter()
                    .filter_map(Inherent::try_from),
            );

            Ok(inherent_tx_vec.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

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
                // Get all the historic transactions that correspond to this hash.
                let mut historic_tx_vec = blockchain.history_store.get_hist_tx_by_hash(&hash, None);

                // Unpack the transaction or raise an error.
                let historic_tx = match historic_tx_vec.len() {
                    0 => {
                        return Err(Error::TransactionNotFound(hash));
                    }
                    1 => historic_tx_vec.pop().unwrap(),
                    _ => {
                        return Err(Error::MultipleTransactionsFound(hash));
                    }
                };

                // Convert the historic transaction into a regular transaction. This will also convert
                // reward inherents.
                let block_number = historic_tx.block_number;
                let timestamp = historic_tx.block_time;

                if let Ok(tx) = historic_tx.into_transaction() {
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

    async fn get_account_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Account, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let account = blockchain
                .get_account_if_complete(&address)
                .ok_or(Error::NoConsensus)?;
            Ok(Account::from_account_with_state(
                address,
                account,
                BlockchainState::new(blockchain.block_number(), blockchain.head_hash()),
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn get_accounts(&mut self) -> RPCResult<Vec<Account>, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let db_txn = blockchain.read_transaction();
            let mut start = Some(KeyNibbles::default());
            let mut accounts = vec![];
            while start.is_some() {
                let chunk = blockchain.get_accounts_chunk(Some(&db_txn), start.unwrap(), 1000);
                start = chunk.end_key;
                for account in chunk.accounts {
                    accounts.push(Account::from_account(account.0, account.1));
                }
            }
            Ok(RPCData::with_blockchain(accounts, &blockchain_proxy))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn get_active_validators(
        &mut self,
    ) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;

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

    async fn get_current_penalized_slots(
        &mut self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let block_number = blockchain.block_number();
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;

            Ok(RPCData::with_blockchain(
                PenalizedSlots {
                    block_number,
                    disabled: staking_contract
                        .punished_slots
                        .current_batch_punished_slots(),
                },
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn get_previous_penalized_slots(
        &mut self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let block_number = blockchain.block_number();
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;

            Ok(RPCData::with_blockchain(
                PenalizedSlots {
                    block_number,
                    disabled: staking_contract
                        .punished_slots
                        .current_batch_punished_slots(),
                },
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn get_validator_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Validator, BlockchainState, Self::Error> {
        get_validator_by_address(&self.blockchain.read(), &address)
    }

    async fn get_validators(&mut self) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();

        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;
            let data_store = blockchain.get_staking_contract_store();
            let db_txn = blockchain.read_transaction();
            let validators = staking_contract.get_validators(&data_store.read(&db_txn));

            Ok(RPCData::with_blockchain(
                validators.iter().map(Validator::from_validator).collect(),
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn get_stakers_by_validator_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Vec<Staker>, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();

        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;
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

    async fn get_staker_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Staker, BlockchainState, Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let staking_contract = blockchain
                .get_staking_contract_if_complete(None)
                .ok_or(Error::NoConsensus)?;
            let data_store = blockchain.get_staking_contract_store();
            let db_txn = blockchain.read_transaction();
            let staker = staking_contract
                .get_staker(&data_store.read(&db_txn), &address)
                .ok_or(Error::StakerNotFound(address))?;

            Ok(RPCData::with_blockchain(
                Staker::from_staker(&staker),
                &blockchain_proxy,
            ))
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

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
                .ok();
                future::ready(result)
            })
            .boxed())
    }

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
                    BlockchainEvent::Finalized(_) | BlockchainEvent::EpochFinalized(_) => None,
                    BlockchainEvent::Rebranched(_, new_branch) => {
                        Some(new_branch.into_iter().last().unwrap().0.into())
                    }
                    BlockchainEvent::Stored(_block) => None,
                };
                future::ready(result)
            })
            .boxed())
    }

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
                        get_validator_by_address(&blockchain_rg, &address).ok()
                    }
                    _ => None,
                };
                future::ready(result)
            })
            .boxed())
    }

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
