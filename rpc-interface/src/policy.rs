use async_trait::async_trait;

use crate::types::{PolicyConstants, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "PolicyProxy", rename_all = "camelCase")]
#[async_trait]
pub trait PolicyInterface {
    type Error;

    /// Returns a bundle of policy constants that define key parameters of the blockchain.
    ///
    /// **Returns**:  
    /// - `PolicyConstants`: A collection of immutable network parameters.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": {
    ///       "stakingContractAddress": "string",
    ///       "coinbaseAddress": "string",
    ///       "transactionValidityWindow": number,
    ///       "maxSizeMicroBody": number,
    ///       "version": number,
    ///       "slots": number,
    ///       "blocksPerBatch": number,
    ///       "batchesPerEpoch": number,
    ///       "blocksPerEpoch": number,
    ///       "validatorDeposit": number,
    ///       "minimumStake": number,
    ///       "totalSupply": number,
    ///       "blockSeparationTime": number,
    ///       "jailEpochs": number,
    ///       "genesisBlockNumber": number
    ///     },
    ///     "metadata": null
    ///   }
    /// }
    /// ```
    async fn get_policy_constants(&mut self) -> RPCResult<PolicyConstants, (), Self::Error>;

    /// Returns the epoch number corresponding to a given block number (height).
    ///
    /// **Parameters**:  
    /// - `block_number` (`u32`, required): The block number for which the epoch is requested.
    ///
    /// **Returns**:  
    /// - `u32`: The epoch number corresponding to the provided block number.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   }
    /// }
    /// ```
    async fn get_epoch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;


    /// Returns the epoch index for a given block number. The epoch index represents the position of a block within its epoch.
    /// For example, the first block of an epoch always has an index of `0`.
    ///
    /// **Parameters**:  
    /// - `block_number` (`u32`, required): The block number for which the epoch index is requested.
    ///
    /// **Returns**:  
    /// - `u32`: The index of the block within its epoch.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   }
    /// }
    /// ```
    async fn get_epoch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the batch number at a given block number.
    ///
    /// **Parameters**:  
    /// - `block_number` (`u32`, required): The block number for which the batch number is requested.
    ///
    /// **Returns**:  
    /// - `u32`: The batch number to which the block belongs.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   }
    /// }
    /// ```
    async fn get_batch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;


    /// Returns the batch index at a given block number.
    /// The batch index represents the position of a block within its batch. The first block of any batch always has an index of `0`.
    ///
    /// **Parameters**:  
    /// - `block_number` (`u32`, required): The block number for which the batch index is requested.
    ///
    /// **Returns**:  
    /// - `u32`: The block's index within its batch.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   }
    /// }
    /// ```
    async fn get_batch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Retrieves the height of the next election macro block following the specified block.
    ///
    /// **Parameters**:  
    /// - `block_number` (`u32`, required): The block height from which to search for the next election macro block.
    ///
    /// **Returns**:  
    /// - `u32`: The block number of the next election macro block.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   }
    /// }
    /// ```
async fn get_election_block_after(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the number (height) of the preceding election macro block before a given block number (height).
    /// If the given block number is an election macro block, it returns the election macro block before it.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to query.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the last election macro block before the given block.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_election_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number (height) of the last election macro block at a given block number (height).
    /// If the given block number is an election macro block, then it returns that block number.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to query.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the last election macro block at or before the given block.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_last_election_block(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns a boolean indicating whether the block at a given block number (height) is an election macro block.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to check.
    ///
    /// **Returns**:
    /// - `bool`: `true` if the block is an election macro block, `false` otherwise.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": false,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn is_election_block_at(&mut self, block_number: u32)
        -> RPCResult<bool, (), Self::Error>;

    /// Returns the block number (height) of the next macro block after a given block number.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number used as a reference.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the next macro block.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_macro_block_after(&mut self, block_number: u32)
        -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number (height) of the preceding macro block before a given block number.
    /// If the given block number is a macro block, it returns the macro block before it.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number used as a reference.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the preceding macro block.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_macro_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number (height) of the last macro block at a given block number.
    /// If the given block number is a macro block, it returns that block number.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number used as a reference.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the last macro block at or before the given block number.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_last_macro_block(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns a boolean indicating whether the block at a given block number is a macro block.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to check.
    ///
    /// **Returns**:
    /// - `bool`: `true` if the block at the given block number is a macro block, `false` if is a micro block.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": false,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn is_macro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error>;

    /// Returns a boolean indicating whether the block at a given block number is a micro block.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to check.
    ///
    /// **Returns**:
    /// - `bool`: `true` if the block at the given block number is a micro block, `false` if is a macro block.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": true,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn is_micro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error>;

    /// Returns the block number of the first block of the given epoch. This block is always a micro block.
    ///
    /// **Parameters**:
    /// - `epoch` (`u32`, required): The epoch number.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the first block of the given epoch.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_first_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number of the first block of the given batch. This block is always a micro block.
    ///
    /// **Parameters**:
    /// - `batch` (`u32`, required): The batch number.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the first block of the given batch.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_first_block_of_batch(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number of the election macro block of the given epoch. This block is always the last block of the epoch.
    ///
    /// **Parameters**:
    /// - `epoch` (`u32`, required): The epoch number.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the election macro block of the given epoch.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_election_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number of the macro block (checkpoint or election) of the given batch. This block is always the last block of the batch.
    ///
    /// **Parameters**:
    /// - `batch` (`u32`, required): The batch number.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the macro block for the given batch.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    ///
    async fn get_macro_block_of(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns whether the batch at a given block number (height) is the first batch of the epoch.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to check.
    ///
    /// **Returns**:
    /// - `bool`: `true` if the batch is the first batch of the epoch, `false` otherwise.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": false,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_first_batch_of_epoch(
        &mut self,
        block_number: u32,
    ) -> RPCResult<bool, (), Self::Error>;

    /// Returns the first block after the reporting window of a given block number has ended.
    /// The reporting window refers to the period during which certain equivocations can be reported.
    /// Once the reporting window for a given block has closed, this function returns the next available block.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to check.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the first block after the reporting window.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_block_after_reporting_window(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the first block after the jail period of a given block number has ended.
    /// The jail period refers to the duration for which a validator is penalized and restricted from participating.
    /// Once the jail period for a given block has ended, this function returns the next available block.
    ///
    /// **Parameters**:
    /// - `block_number` (`u32`, required): The block number to check.
    ///
    /// **Returns**:
    /// - `u32`: The block number of the first block after the jail period.
    ///
    /// **Example Response**:
    ///
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_block_after_jail(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the supply at a given time (as Unix time) in Lunas (1 NIM = 100,000 Lunas).
    /// It is calculated using the following formula:
    /// ```text
    /// supply(t) = total_supply - (total_supply - genesis_supply) * supply_decay^t
    /// ```
    /// Where `t` is the time in milliseconds since the PoS genesis block and `genesis_supply`
    /// is the supply at the genesis of the Nimiq PoS chain.
    ///
    /// **Parameters**:
    /// - `genesis_supply` (`u64`): The supply at the genesis of the PoS chain.
    /// - `genesis_time` (`u64`): The Unix timestamp (seconds) of the PoS genesis block.
    /// - `current_time` (`u64`): The Unix timestamp (seconds) for which to retrieve the supply.
    ///
    /// **Returns**:
    /// - `u64`: The total supply at the given `current_time`, measured in Lunas.
    /// 
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": number,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_supply_at(
        &mut self,
        genesis_supply: u64,
        genesis_time: u64,
        current_time: u64,
    ) -> RPCResult<u64, (), Self::Error>;
}
