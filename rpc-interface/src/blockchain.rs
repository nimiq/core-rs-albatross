use async_trait::async_trait;
use futures::stream::BoxStream;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

use crate::types::{
    Account, Block, BlockLog, BlockchainState, ExecutedTransaction, Inherent, LogType,
    PenalizedSlots, RPCData, RPCResult, Slot, Staker, Validator,
};

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait BlockchainInterface {
    type Error;

    /// Returns the block number for the current head.
    /// 
    /// **Parameters**: None. This method does not require any input.
    /// 
    /// **Returns**: 
    /// A `u32` value representing the block number of the current head block.
    /// 
    /// **Response Example**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": number
    /// }
    /// ```
    async fn get_block_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Returns the batch number for the current head.
    /// 
    /// **Parameters**: None. This method does not require any input.
    /// 
    /// **Returns**: 
    /// A `u32` value representing the batch number of the current head block.
    /// 
    /// **Response Example**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": number
    /// }
    /// ```
    async fn get_batch_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Returns the epoch number for the current head.
    /// 
    /// **Parameters**: None. This method does not require any input.
    /// 
    /// **Returns**: 
    /// A `u32` value representing the epoch number of the current head block.
    /// 
    /// **Response Example**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": number
    /// }
    /// ```
    async fn get_epoch_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Fetches a block from the main chain by its block hash. It has an option to include the transactions in the
    /// block, which defaults to false.
    /// 
    /// **Parameters**:
    /// - `block_hash` (`Blake2bHash`): The hash of the block to fetch.
    ///     - `include_body` (`bool`): Whether to include the full block body (with transactions).
    ///
    /// **Returns**:
    /// - The details of the requested block.
    /// 
    /// **Response Example** without `include_body`:
    /// 
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "timestamp": number,
    ///     "state_hash": "string",
    ///     "transactions": null
    ///   },
    /// }
    /// ```
    ///
    /// **With `include_body`:**
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "transactions": [
    ///       {
    ///         "hash": "string",
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": number,
    ///         "fee": number
    ///       },
    ///       {
    ///         "hash": "string",
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": number,
    ///         "fee": number
    ///       }
    ///     ]
    ///   },
    /// }
    /// ```
    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Fetches a block from the main chain by its block number. It has an option to include the transactions in the
    /// block, which defaults to false.
    /// 
    /// **Parameters**:
    /// - `block_number` (`u32`): The number of the block to fetch.
    ///     - `include_body` (`bool`): Whether to include the full block body (with transactions).
    ///
    /// **Returns**:
    /// - The details of the requested block.
    /// 
    /// **Response Example**:
    /// 
    /// **Without `include_body`:**
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "timestamp": number,
    ///     "state_hash": "string",
    ///     "transactions": null
    ///   },
    /// }
    /// ```
    ///
    /// **With `include_body`:**
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "transactions": [
    ///       {
    ///         "hash": "string",
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": number,
    ///         "fee": number
    ///       },
    ///       {
    ///         "hash": "string",
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": number,
    ///         "fee": number
    ///       }
    ///     ]
    ///   },
    /// }
    /// ```
    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Fetches the latest block from the main chain. It has an option to include the transactions in the
    /// block, which defaults to false.
    /// 
    /// **Parameters**:
    /// - Optionally `include_body` (`bool`): Whether to include the full block body (with transactions).
    ///
    /// **Returns**:
    /// - The details of the requested block.
    /// 
    /// **Response Example**:
    /// 
    /// **Without `include_body`:**
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "timestamp": number,
    ///     "state_hash": "string",
    ///     "transactions": null
    ///   },
    /// }
    /// ```
    ///
    /// **With `include_body`:**
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "transactions": [
    ///       {
    ///         "hash": "string",
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": number,
    ///         "fee": number
    ///       },
    ///       {
    ///         "hash": "string",
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": 0,
    ///         "fee": 0
    ///       }
    ///     ]
    ///   },
    /// }
    /// ```
    async fn get_latest_block(
        &mut self,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Returns information about the proposer slot at the given block height and offset.
    /// The offset is optional and defaults to the offset of the existing block at the specified height.
    /// We only have this information available for the last 2 batches at most.
    ///
    /// **Parameters**:  
    /// - `block_number` (required, `u32`): The block height at which to retrieve the slot information.  
    /// - `offset_opt` (optional, `u32`): The offset for the slot at the specified block height. If omitted, it defaults to the offset used by the existing block at that height.
    ///
    /// **Returns**:  
    /// - `Slot`: Contains the following information about the proposer slot:  
    ///   - `slot_number` (`u16`): The slot number at the given height.  
    ///   - `validator` (`Address`): The Nimiq address of the validator assigned to this slot.  
    ///   - `public_key` (`CompressedPublicKey`): The compressed public key associated with the validator.  
    /// - Metadata:  
    ///   - `block_number` (`u32`): The block height where the slot was recorded.  
    ///   - `block_hash` (`Blake2bHash`): The hash of the block at that height.  
    ///
    /// **Notes**:  
    /// - This function is limited to retrieving slot information from the last **two batches** at most.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "slot_number": 0,
    ///     "validator": "string",
    ///     "public_key": "string"
    ///   },
    ///   "metadata": {
    ///     "block_number": number,
    ///     "block_hash": "string"
    ///   },
    /// }
    /// ```
    async fn get_slot_at(
        &mut self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> RPCResult<Slot, BlockchainState, Self::Error>;

    /// Fetches a transaction by its hash, including reward transactions.
    ///
    /// **Parameters**:  
    /// - `hash` (`Blake2bHash`, required): The hash of the transaction to retrieve.  
    ///
    /// **Returns**:  
    /// - `ExecutedTransaction` (`transaction`): The retrieved transaction and its execution result.  
    /// - `execution_result` (`bool`): Indicates whether the transaction was successfully executed (`true`) or not (`false`).
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "transaction": {
    ///       "hash": "string",
    ///       "block_number": number,
    ///       "timestamp": number,
    ///       "confirmations": number,
    ///       "size": 0,
    ///       "related_addresses": [
    ///         "string",
    ///         "string"
    ///       ],
    ///       "from": "string",
    ///       "from_type": number,
    ///       "to": "string",
    ///       "to_type": number,
    ///       "value": number,
    ///       "fee": number,
    ///       "sender_data": "string",
    ///       "recipient_data": "string",
    ///       "flags": number,
    ///       "validity_start_height": number,
    ///       "proof": "string",
    ///       "network_id": number
    ///     },
    ///     "execution_result": true
    ///   },
    /// }
    /// ```
    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error>;

    /// Fetches all transactions within a given block, including reward transactions.
    ///
    /// **Parameters**:  
    /// - `block` (`u32`, required): The block of the transactions to retrieve.  
    ///
    /// **Returns**:  
    /// - `Vec<ExecutedTransaction>`: A list of transactions found in the specified block.  
    ///   - Each `ExecutedTransaction` includes:
    ///     - `transaction`: The transaction data.
    ///     - `execution_result`: A boolean indicating whether the transaction was successfully executed (`true`) or failed (`false`).
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "transaction1": {
    ///       "hash": "string",
    ///       "block_number": number,
    ///       "timestamp": number,
    ///       "confirmations": number,
    ///       "size": number,
    ///       "related_addresses": [
    ///         "string",
    ///         "string"
    ///       ],
    ///       "from": "string",
    ///       "from_type": number,
    ///       "to": "string",
    ///       "to_type": number,
    ///       "value": number,
    ///       "fee": number,
    ///       "sender_data": "string",
    ///       "recipient_data": "string",
    ///       "flags": number,
    ///       "validity_start_height": number,
    ///       "proof": "string",
    ///       "network_id": number
    ///     },
    ///     "execution_result": true
    ///   }, ...
    /// ```
    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Returns all the inherents (including reward inherents) for the given block number.
    ///
    /// **Parameters**:  
    /// - `block_number` (`u32`, required): The block number from which to retrieve inherents.  
    ///
    /// **Returns**:  
    /// - A list of `Inherent` objects. Each inherent represents a protocol-level transaction. Can be a reward, jail or penalize inherent.
    ///   - `Reward`: Represents a block reward distribution. Contains:  
    ///     - `block_number` (`u32`): The block number where the reward was issued.  
    ///     - `block_time` (`u64`): The timestamp when the block was created.  
    ///     - `validator_address` (`Address`): The address of the validator receiving the reward.  
    ///     - `target` (`Address`): The address where the reward is credited.  
    ///     - `value` (`Coin`): The amount of coins rewarded.  
    ///     - `hash` (`Blake2bHash`): The hash identifying the inherent.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "type": "string",
    ///       "block_number": number,
    ///       "block_time": number,
    ///       "validator_address": "string",
    ///       "target": "string",
    ///       "value": number,
    ///       "hash": "string"
    ///     },
    ///   ]
    /// }
    /// ```
    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    /// Returns all transactions (including reward transactions) for a given batch number.
    ///
    /// **Parameters**:  
    /// - `batch_number` (`u32`, required): The batch number to retrieve transactions from.  
    ///
    /// **Returns**:  
    /// - A list of `ExecutedTransaction` objects, each containing:  
    ///   - `transaction`: Full transaction details (`Transaction`).  
    ///   - `execution_result` (`bool`): Indicates whether the transaction was successfully executed.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "transaction": {
    ///         "hash": "string",
    ///         "block_number": number,
    ///         "timestamp": number,
    ///         "confirmations": number,
    ///         "size": number,
    ///         "related_addresses": [
    ///           "string",
    ///           "string"
    ///         ],
    ///         "from": "string",
    ///         "from_type": number,
    ///         "to": "string",
    ///         "to_type": number,
    ///         "value": number,
    ///         "fee": number,
    ///         "sender_data": "string",
    ///         "recipient_data": "string",
    ///         "flags": number,
    ///         "validity_start_height": number,
    ///         "proof": "string",
    ///         "network_id": number
    ///       },
    ///       "execution_result": true
    ///     },
    ///   ]
    /// }
    /// ```
    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Retrieves all inherents (including reward inherents) for the specified batch number.
    /// If no inherents exist for the specified batch, the result will be an empty list. 
    ///
    /// **Parameters**:  
    /// - `batch_number` (`u32`, required): The batch number to fetch inherents from.  
    ///
    /// **Returns**:  
    /// - A list of `Inherent` objects.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "block_number": number,
    ///       "block_time": number,
    ///       "validator_address": "string",
    ///       "target": "string",
    ///       "value": number,
    ///       "hash": "string"
    ///     },
    ///   ]
    /// }
    /// ```
    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    /// Retrieves the hashes of the latest transactions for a given address.
    /// The default limit is 500, but it can be adjusted using the `max` parameter.
    /// Both sent and received transactions are included. Reward transactions are also returned.
    ///  
    /// **Parameters**:  
    /// - `address` (`Address`, required): The Nimiq address for which to fetch transaction hashes.  
    /// - `max` (`Option<u16>`, optional): The maximum number of transaction hashes to return. Defaults to 500.  
    /// - `start_at` (`Option<Blake2bHash>`, optional): The transaction hash to start fetching from (exclusive).  
    ///
    /// **Returns**:  
    /// - A list of transaction hashes (`Blake2bHash`) in descending order (latest transaction first).  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     "string",
    ///     "string",
    ///     "string"
    ///   ]
    /// }
    /// ```
    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
        start_at: Option<Blake2bHash>,
    ) -> RPCResult<Vec<Blake2bHash>, (), Self::Error>;

    /// Retrieves the latest transactions for a given address.
    /// The default limit is 500, but it can be adjusted using the `max` parameter.
    /// Both sent and received transactions are included. Reward transactions are also returned.
    ///  
    /// **Parameters**:  
    /// - `address` (`Address`, required): The Nimiq address for which to fetch transactions.  
    /// - `max` (`Option<u16>`, optional): The maximum number of transactions to return. Defaults to 500.  
    /// - `start_at` (`Option<Blake2bHash>`, optional): The transaction hash to start fetching from (exclusive).  
    ///   - If this hash is not found or does not belong to this address, an empty list is returned.  
    ///
    /// **Returns**:  
    /// - `Vec<ExecutedTransaction>`: A list of transactions found in the specified block.  
    ///   - Each `ExecutedTransaction` includes:
    ///     - `transaction`: The transaction data.
    ///     - `execution_result`: A boolean indicating whether the transaction was successfully executed (`true`) or failed (`false`). 
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "transaction": {
    ///         "hash": "string",
    ///         "block_number": number,
    ///         "timestamp": number,
    ///         "confirmations": number,
    ///         "size": number,
    ///         "related_addresses": [
    ///           "string",
    ///           "string"
    ///         ],
    ///         "from": "string",
    ///         "to": "string",
    ///         "value": number,
    ///         "fee": number,
    ///         "flags": number,
    ///         "validity_start_height": number,
    ///         "network_id": number
    ///       },
    ///       "execution_result": true
    ///     },
    ///   ]
    /// }
    /// ```
    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
        start_at: Option<Blake2bHash>,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Fetches the account details for the given address.
    ///
    /// **Parameters**:  
    /// - `address` (`Address`, required): The Nimiq address of the account to retrieve.  
    ///
    /// **Returns**:  
    /// - `Account`: The account details, including:  
    ///   - `address` (`Address`): The Nimiq address associated with the account.  
    ///   - `balance` (`Coin`): The account balance.  
    ///   - `account_additional_fields` (`AccountAdditionalFields`): The type of account (e.g., `"basic"` for standard accounts).  
    /// - `BlockchainState`: Metadata about the blockchain state when retrieving the account, including:  
    ///   - `blockNumber` (`u32`): The block height at which the account data was retrieved.  
    ///   - `blockHash` (`Blake2bHash`): The hash of the block at that height.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": {
    ///       "address": "string",
    ///       "balance": number,
    ///       "type": "string"
    ///     },
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   },
    /// }
    /// ```
    async fn get_account_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Account, BlockchainState, Self::Error>;

    /// Fetches all accounts in the accounts tree.
    ///
    /// **IMPORTANT**:  
    /// This operation iterates over all accounts in the accounts tree and is **extremely computationally expensive**.
    /// It should only be used when absolutely necessary. If you need specific account details, consider fetching individual accounts instead of querying all at once.
    ///
    /// **Returns**:  
    /// - A list of `Account` objects, each containing:  
    ///   - `address` (`Address`): The Nimiq address associated with the account.  
    ///   - `balance` (`Coin`): The account balance.  
    ///   - `account_additional_fields` (`AccountAdditionalFields`): The type of account (e.g., `"basic"` for standard accounts).   
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "address": "string",
    ///       "balance": number,
    ///       "type": "string"
    ///     },
    ///     {
    ///       "address": "string",
    ///       "balance": 0,
    ///       "type": "string"
    ///     },
    ///   ]
    /// }
    /// ```
    async fn get_accounts(&mut self) -> RPCResult<Vec<Account>, BlockchainState, Self::Error>;

    /// Fetches a collection of the currently active validators, along with their addresses and balances.
    /// This function retrieves only active validators. Retired or jailed validators may still be present but marked accordingly.
    ///
    /// **Returns**:  
    /// - A list of `Validator` objects.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": [
    ///       {
    ///         "address": "string",
    ///         "signingKey": "string",
    ///         "votingKey": "string",
    ///         "rewardAddress": "string",
    ///         "signalData": null,
    ///         "balance": number,
    ///         "numStakers": number,
    ///         "inactivityFlag": null,
    ///         "retired": false,
    ///         "jailedFrom": null
    ///       },
    ///     ],
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ``` 
    async fn get_active_validators(
        &mut self,
    ) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    /// Fetches information about the currently penalized slots, including slots that have lost rewards or were disabled.
    /// If `disabled` is an empty array, it means no slots were penalized at that block height.
    ///
    /// **Returns**:  
    /// - `PenalizedSlots`: The list of currently penalized slots.  
    /// - `BlockchainState`: Metadata about the blockchain state at the time of retrieval.  

    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": {
    ///       "blockNumber": number,
    ///       "disabled": [number, number]
    ///     },
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ```
    async fn get_current_penalized_slots(
        &mut self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error>;

    /// Returns information about the penalized slots of the previous batch. This includes slots that lost rewards and those that were disabled.
    ///
    /// **Returns**:  
    /// - `PenalizedSlots`: The list of penalized slots from the previous batch.  
    /// - `BlockchainState`: Metadata about the blockchain state at the time of retrieval.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": {
    ///       "blockNumber": 0,
    ///       "disabled": [number, number]
    ///     },
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ```
    async fn get_previous_penalized_slots(
        &mut self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error>;

    /// Retrieves information about a specific validator using its address.
    ///
    /// **Parameters**:  
    /// - `address` (`Address`, required): The Nimiq address of the validator to query.  
    ///
    /// **Returns**:  
    /// - `Validator` object.  
    /// - `BlockchainState`.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": {
    ///       "address": "string",
    ///       "signingKey": "string",
    ///       "votingKey": "string",
    ///       "rewardAddress": "string",
    ///       "signalData": null,
    ///       "balance": number,
    ///       "numStakers": number,
    ///       "inactivityFlag": null,
    ///       "retired": false,
    ///       "jailedFrom": null
    ///     },
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ```
    async fn get_validator_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Validator, BlockchainState, Self::Error>;

    /// Retrieves all validators in the staking contract, including active, inactive, jailed, and retired validators. Due to its nature,  
    /// this function is computationally expensive.  
    ///
    /// **Returns**:  
    /// - A list of `Validator` objects containing full validator details.  
    /// - `BlockchainState`: Metadata about the blockchain state at retrieval.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": [
    ///       {
    ///         "address": "string",
    ///         "signingKey": "string",
    ///         "votingKey": "string",
    ///         "rewardAddress": "string",
    ///         "signalData": null,
    ///         "balance": number,
    ///         "numStakers": number,
    ///         "inactivityFlag": null,
    ///         "retired": false,
    ///         "jailedFrom": null
    ///       },
    ///     ],
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ```
    async fn get_validators(&mut self) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    /// Fetches all stakers for a given validator.
    ///
    /// **Returns**:  
    /// - A list of `Staker` objects containing full staking details.  
    /// - `BlockchainState`: Metadata about the blockchain state at retrieval.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": [
    ///       {
    ///         "address": "string",
    ///         "balance": number,
    ///         "delegation": "string",
    ///         "inactiveBalance": number,
    ///         "inactiveFrom": number,
    ///         "retiredBalance": number
    ///       },
    ///     ],
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ```
    async fn get_stakers_by_validator_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Vec<Staker>, BlockchainState, Self::Error>;

    /// Fetches information about a staker given their address.
    ///
    /// **Returns**:  
    /// - `Staker`: The staker's details, including delegation status and balances.  
    /// - `BlockchainState`: Metadata about the blockchain state at retrieval.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": {
    ///       "address": "string",
    ///       "balance": number,
    ///       "delegation": "string",
    ///       "inactiveBalance": number,
    ///       "inactiveFrom": number,
    ///       "retiredBalance": number
    ///     },
    ///     "metadata": {
    ///       "blockNumber": number,
    ///       "blockHash": "string"
    ///     }
    ///   }
    /// }
    /// ```
    async fn get_staker_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Staker, BlockchainState, Self::Error>;

    /// Subscribes to new block events, streaming full block details as they are produced.
    ///
    /// **Behavior**:
    /// - Streams new blocks as they are added to the chain.
    /// - Includes block headers and, optionally, the body (transactions and inherents).
    /// - The subscription remains active until manually unsubscribed.
    ///
    /// **Parameters**:
    /// - `include_body` (`Option<bool>`):  
    ///   - `true`: Includes transactions and inherents.  
    ///   - `false` or `None`: Returns only the block header.
    ///
    /// **Returns**:  
    /// A stream of `Block` objects containing:  
    /// - `hash` (`Blake2bHash`): The block hash.  
    /// - `number` (`u32`): The block height.  
    /// - `timestamp` (`u64`): The block’s timestamp.  
    /// - `state_hash` (`Blake2bHash`): The blockchain state hash after this block.  
    /// - `transactions` (`Option<Vec<ExecutedTransaction>>`): List of transactions if `include_body = true`.  
    ///
    /// **Example Response** (Header only):  
    /// ```json
    /// {
    ///   "data": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "timestamp": number,
    ///     "state_hash": "string",
    ///     "transactions": null
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    ///
    /// **Example Response** (With Body):  
    /// ```json
    /// {
    ///   "data": {
    ///     "hash": "string",
    ///     "number": number,
    ///     "timestamp": number,
    ///     "state_hash": "string",
    ///     "transactions": [ { "hash": "string", "from": "string", "to": "string", "value": number } ]
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    #[stream]
    async fn subscribe_for_head_block(
        &mut self,
        include_body: Option<bool>,
    ) -> Result<BoxStream<'static, RPCData<Block, ()>>, Self::Error>;

    /// Subscribes to new block events, streaming only the block hash.
    ///
    /// **Behavior**:
    /// - Streams the hash of each new block as it is produced.
    /// - Does not include block details such as height, timestamp, or transactions.
    /// - The subscription remains active until manually unsubscribed.
    ///
    /// **Parameters**: None.
    ///
    /// **Returns**:  
    /// A stream of hashes objects containing:  
    /// - `hash` (`Blake2bHash`): The unique identifier of the block.
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": {
    ///     "hash": "string"
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    #[stream]
    async fn subscribe_for_head_block_hash(
        &mut self,
    ) -> Result<BoxStream<'static, RPCData<Blake2bHash, ()>>, Self::Error>;

    /// Subscribes to validator election events for a specific address.
    ///
    /// **Behavior**:
    /// - Streams updates when the given validator address is involved in an upcoming election.
    /// - Provides information about the validator and the current blockchain state.
    /// - The subscription remains active until manually unsubscribed.
    ///
    /// **Parameters**:
    /// - `address` (`Address`): The validator address to track in election events.
    ///
    /// **Returns**:  
    /// A stream of objects containing:  
    /// - `data` (`Validator`): The validator's details, including its address, signing key, and status.  
    /// - `metadata` (`BlockchainState`): The current state of the blockchain at the time of the event.  
    ///
    /// **Example Response**:  
    /// ```json
    /// {
    ///   "data": {
    ///     "address": "string",
    ///     "signing_key": "string",
    ///     "voting_key": "string",
    ///     "reward_address": "string",
    ///     "is_active": true
    ///   },
    ///   "metadata": {
    ///     "epoch": number,
    ///     "batch": number,
    ///     "block_number": number
    ///   }
    /// }
    /// ``` 
    #[stream]
    async fn subscribe_for_validator_election_by_address(
        &mut self,
        address: Address,
    ) -> Result<BoxStream<'static, RPCData<Validator, BlockchainState>>, Self::Error>;

    /// Subscribes to log events related to a given list of addresses and log types.
    ///
    /// **Behavior**:
    /// - If `addresses` is empty (`[]`), logs from **all** addresses will be included.
    /// - If `log_types` is empty (`[]`), logs of **all** types will be included.
    /// - If both parameters are empty, the subscription will return **all logs**.
    ///
    /// **Parameters**:
    /// - `addresses` (`Vec<Address>`): A list of addresses to filter log events. If empty, no filtering by address occurs.
    /// - `log_types` (`Vec<LogType>`): A list of log types to filter events. If empty, all log types are included.
    ///
    /// **Returns**:
    /// - A stream of `BlockLog` events wrapped in `RPCData`, with an associated `BlockchainState`.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": {
    ///     "block_hash": "string",
    ///     "block_number": number,
    ///     "logs": [
    ///       {
    ///         "address": "string",
    ///         "topics": ["string"],
    ///         "data": "string",
    ///         "log_type": "string"
    ///       }
    ///     ]
    ///   },
    ///   "metadata": {
    ///     "blockchain_state": {
    ///       "epoch": number,
    ///       "batch": number
    ///     }
    ///   }
    /// }
    /// ```
    #[stream]
    async fn subscribe_for_logs_by_addresses_and_types(
        &mut self,
        addresses: Vec<Address>,
        log_types: Vec<LogType>,
    ) -> Result<BoxStream<'static, RPCData<BlockLog, BlockchainState>>, Self::Error>;
}
