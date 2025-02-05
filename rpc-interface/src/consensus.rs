use async_trait::async_trait;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{AnyHash, PreImage};

use crate::types::{RPCResult, Transaction, ValidityStartHeight};

#[nimiq_jsonrpc_derive::proxy(name = "ConsensusProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ConsensusInterface {
    type Error;

    /// Returns whether the node has established consensus with the network.
    ///
    /// A node is considered to have consensus when it has synchronized with the blockchain.
    ///
    /// **Returns**:
    /// - `bool`: `true` if the node has consensus, `false` otherwise.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": bool,
    ///     "metadata": null
    ///   },
    ///   "id": 1
    /// }
    /// ```
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_consensus_established(&mut self) -> RPCResult<bool, (), Self::Error>;

    /// Given a serialized transaction, returns the corresponding transaction struct.
    ///
    /// **Parameters**:
    /// - `raw_tx` (`String`): The serialized transaction in hex format.
    ///
    /// **Returns**:
    /// - `Transaction`: The structured transaction object containing transaction details.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": {
    ///     "hash": "string",
    ///     "size": number,
    ///     "relatedAddresses": [
    ///       "string"
    ///     ],
    ///     "from": "string",
    ///     "fromType": number,
    ///     "to": "string",
    ///     "toType": number,
    ///     "value": number,
    ///     "fee": number,
    ///     "senderData": "string",
    ///     "recipientData": "string",
    ///     "flags": number,
    ///     "validityStartHeight": number,
    ///     "proof": "string",
    ///     "networkId": number
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    async fn get_raw_transaction_info(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Transaction, (), Self::Error>;

    /// Sends the given serialized transaction to the network.
    ///
    /// **Parameters**:
    /// - `raw_tx` (`String`): The serialized transaction to be sent.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash of the sent transaction.
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
    async fn send_raw_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized basic transaction.
    /// This method **only** creates the transaction but does not send it to the network.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address.
    /// - `recipient` (`Address`): The recipient's address.
    /// - `value` (`Coin`): The amount to be sent.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: The serialized basic transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a basic transaction to the network.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address.
    /// - `recipient` (`Address`): The recipient's address.
    /// - `value` (`Coin`): The amount to be sent.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash, confirming that the transaction was broadcasted.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized basic transaction with an arbitrary data field.
    ///
    /// This method **creates** a basic transaction that includes an arbitrary data field but does not send it to the network.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address.
    /// - `recipient` (`Address`): The recipient's address.
    /// - `data` (`Vec<u8>`): Arbitrary data to be included in the transaction.
    /// - `value` (`Coin`): The amount to be sent.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: The serialized transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a basic transaction with an arbitrary data field to the network.
    ///
    /// This method **sends** a previously created basic transaction containing an arbitrary data field.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address.
    /// - `recipient` (`Address`): The recipient's address.
    /// - `data` (`Vec<u8>`): Arbitrary data to be included in the transaction.
    /// - `value` (`Coin`): The amount to be sent.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction creating a new vesting contract.
    ///
    /// This method **creates** but does not send a transaction to establish a new vesting contract.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that funds the vesting contract.
    /// - `owner` (`Address`): The address of the contract's owner who can redeem the funds.
    /// - `start_time` (`u64`): The timestamp (Unix time in seconds) when vesting starts.
    /// - `time_step` (`u64`): The interval (in seconds) between each vesting step.
    /// - `num_steps` (`u32`): The total number of vesting steps.
    /// - `value` (`Coin`): The total amount to be vested.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: The serialized vesting contract creation transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_new_vesting_transaction(
        &mut self,
        wallet: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction creating a new vesting contract to the network.
    ///
    /// This method **broadcasts** a transaction that establishes a new vesting contract.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that funds the vesting contract.
    /// - `owner` (`Address`): The address of the contract's owner who can redeem the funds.
    /// - `start_time` (`u64`): The timestamp (Unix time in seconds) when vesting starts.
    /// - `time_step` (`u64`): The interval (in seconds) between each vesting step.
    /// - `num_steps` (`u32`): The total number of vesting steps.
    /// - `value` (`Coin`): The total amount to be vested.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the sent transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_new_vesting_transaction(
        &mut self,
        wallet: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming a vesting contract.
    ///
    /// This method **creates but does not send** a transaction that redeems funds from a vesting contract.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that initiates the redemption.
    /// - `contract_address` (`Address`): The address of the vesting contract.
    /// - `recipient` (`Address`): The address that will receive the redeemed funds.
    /// - `value` (`Coin`): The amount to be redeemed.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized representation of the redeem vesting transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_redeem_vesting_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming a vesting contract to the network.
    ///
    /// This method broadcasts a previously created redeem vesting transaction to the network.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that initiates the redemption.
    /// - `contract_address` (`Address`): The address of the vesting contract.
    /// - `recipient` (`Address`): The address that will receive the redeemed funds.
    /// - `value` (`Coin`): The amount to be redeemed.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash after broadcasting to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_redeem_vesting_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;


    /// Returns a serialized transaction creating a new HTLC contract.
    ///
    /// This method **builds but does not send** a transaction that creates a new HTLC.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address funding the HTLC contract.
    /// - `htlc_sender` (`Address`): The designated sender in the HTLC contract.
    /// - `htlc_recipient` (`Address`): The designated recipient who can claim the funds.
    /// - `hash_root` (`AnyHash`): The hash root used to verify pre-images.
    /// - `hash_count` (`u8`): The number of hash iterations.
    /// - `timeout` (`u64`): The block height after which the contract can be refunded.
    /// - `value` (`Coin`): The amount of funds locked in the HTLC.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: The serialized transaction data ready to be signed and sent.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_new_htlc_transaction(
        &mut self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction creating a new HTLC contract to the network.
    ///
    /// This method submits a transaction to create a HTLC on the blockchain.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address funding the HTLC contract.
    /// - `htlc_sender` (`Address`): The designated sender in the HTLC contract.
    /// - `htlc_recipient` (`Address`): The designated recipient who can claim the funds.
    /// - `hash_root` (`AnyHash`): The hash root used to verify pre-images.
    /// - `hash_count` (`u8`): The number of hash iterations.
    /// - `timeout` (`u64`): The block height after which the contract can be refunded.
    /// - `value` (`Coin`): The amount of funds locked in the HTLC.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash identifying the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_new_htlc_transaction(
        &mut self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming an HTLC contract using the `RegularTransfer` method.
    ///
    /// This method generates a transaction to redeem funds from an existing HTLC by 
    /// providing the correct pre-image that matches the hash lock.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that signs the redemption transaction.
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds from the HTLC contract.
    /// - `pre_image` (`PreImage`): The pre-image corresponding to the `hash_root`, proving authorization to unlock funds.
    /// - `hash_root` (`AnyHash`): The hash root used to verify the pre-image.
    /// - `hash_count` (`u8`): The number of hash iterations applied to the pre-image.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized transaction that can be broadcasted to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_redeem_regular_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming an HTLC contract using the `RegularTransfer` method to the network.
    ///
    /// This method broadcasts a transaction to redeem funds from an existing HTLC by 
    /// providing the correct pre-image that matches the hash lock.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that signs the redemption transaction.
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds from the HTLC contract.
    /// - `pre_image` (`PreImage`): The pre-image corresponding to the `hash_root`, proving authorization to unlock funds.
    /// - `hash_root` (`AnyHash`): The hash root used to verify the pre-image.
    /// - `hash_count` (`u8`): The number of hash iterations applied to the pre-image.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_redeem_regular_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming an HTLC contract using the `TimeoutResolve` method.
    ///
    /// This method creates a transaction to redeem funds from a HTLC after the timeout has passed.
    /// The sender claims the funds because the recipient did not redeem them before the expiration.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address that initiates the redemption transaction.
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds after the timeout.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized transaction that can be broadcast to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming an HTLC contract using the `TimeoutResolve` method to the network.
    ///
    /// This method broadcasts a transaction that allows the sender to redeem funds from a HTLC
    /// after the timeout period has elapsed. The sender claims the funds because the recipient did not redeem them before expiration.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The sender's address initiating the redemption transaction.
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds after the timeout.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the transaction after being successfully sent to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming an HTLC contract using the `EarlyResolve` method.
    ///
    /// This method creates a transaction that allows both the sender and recipient of a HTLC
    /// to cooperatively redeem the funds before the timeout period elapses. Both parties must provide valid signatures.
    ///
    /// **Parameters**:
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds.
    /// - `htlc_sender_signature` (`String`): The signature provided by the HTLC sender.
    /// - `htlc_recipient_signature` (`String`): The signature provided by the HTLC recipient.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized transaction that must be broadcast to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_redeem_early_htlc_transaction(
        &mut self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming an HTLC contract using the `EarlyResolve` method to the network.
    ///
    /// This method broadcasts a transaction that allows both the sender and recipient of a HTLC
    /// to cooperatively redeem the funds before the timeout period expires. Both parties must provide valid signatures.
    ///
    /// **Parameters**:
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds.
    /// - `htlc_sender_signature` (`String`): The signature provided by the HTLC sender.
    /// - `htlc_recipient_signature` (`String`): The signature provided by the HTLC recipient.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_redeem_early_htlc_transaction(
        &mut self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized signature that can be used to redeem funds from an HTLC contract using the `EarlyResolve` method.
    ///
    /// This method generates a signature that allows the sender or recipient of a HTLC
    /// to redeem the funds before the timeout period expires. This signature must be provided along with the
    /// `send_redeem_early_htlc_transaction` request to authorize the transaction.
    ///
    /// **Parameters**:
    /// - `wallet` (`Address`): The address of the wallet signing the transaction.
    /// - `contract_address` (`Address`): The address of the HTLC contract being redeemed.
    /// - `recipient` (`Address`): The address receiving the funds.
    /// - `value` (`Coin`): The amount to be redeemed from the HTLC contract.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized signature that must be included in the final HTLC redemption transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn sign_redeem_early_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Returns a serialized `new_staker` transaction.
    ///
    /// This method creates a transaction to register a new staker. The transaction must be signed and sent
    /// to the network using `send_new_staker_transaction`. The sender wallet must be a basic account and will
    /// cover the transaction fee.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The basic account paying the transaction fee.
    /// - `staker_wallet` (`Address`): The wallet address of the new staker.
    /// - `delegation` (`Option<Address>`): The validator address the staker is delegating to. If `None`, the staker remains undelegated.
    /// - `value` (`Coin`): The amount to be staked.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized transaction that must be signed and sent to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `new_staker` transaction to the network.
    ///
    /// This method submits a transaction to register a new staker on the blockchain. The sender wallet must be a basic account
    /// and will cover the transaction fee. The transaction must include the amount to be staked and the validator the staker
    /// wishes to delegate to, if any.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The basic account paying the transaction fee.
    /// - `staker_wallet` (`Address`): The wallet address of the new staker.
    /// - `delegation` (`Option<Address>`): The validator address the staker is delegating to. If `None`, the staker remains undelegated.
    /// - `value` (`Coin`): The amount to be staked.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash of the submitted `new_staker` transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `stake` transaction.
    ///
    /// This method creates a transaction to stake funds on behalf of a staker. The funds being staked and the transaction fee 
    /// will be deducted from the sender wallet. The resulting serialized transaction can be broadcast to the network using 
    /// `send_stake_transaction`.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The basic account paying the transaction fee and staking the funds.
    /// - `staker_address` (`Address`): The staker account receiving the staked funds.
    /// - `value` (`Coin`): The amount of NIM to be staked.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized `stake` transaction that can be sent to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `stake` transaction to the network.
    ///
    /// This method submits a transaction to stake funds on behalf of a staker. The funds being staked and the transaction fee 
    /// will be deducted from the sender wallet. This is the equivalent of broadcasting a transaction previously created using 
    /// `create_stake_transaction`.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The basic account paying the transaction fee and staking the funds.
    /// - `staker_address` (`Address`): The staker account receiving the staked funds.
    /// - `value` (`Coin`): The amount of NIM to be staked.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the sent transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `update_staker` transaction.
    ///
    /// This method creates a transaction to update the delegation of a staker. The transaction fee can be paid 
    /// either from a basic account (by providing the `sender_wallet`) or from the staker's balance (by omitting 
    /// the `sender_wallet`). If `new_delegation` is set to `None`, the staker will no longer delegate to a validator.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Option<Address>`): The basic account paying the transaction fee. If `None`, the fee is paid 
    ///   from the staker account's balance.
    /// - `staker_wallet` (`Address`): The staker account to be updated.
    /// - `new_delegation` (`Option<Address>`): The new validator to delegate to. If `None`, the staker will stop 
    ///   delegating.
    /// - `reactivate_all_stake` (`bool`): If `true`, all inactive stake will be reactivated.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `String`: A serialized transaction that can be broadcasted to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_update_staker_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends an `update_staker` transaction to the network.
    ///
    /// This method updates the delegation of a staker. The transaction fee can be paid either from a basic account 
    /// (by providing the `sender_wallet`) or from the staker's balance (by omitting the `sender_wallet`). 
    /// If `new_delegation` is set to `None`, the staker will no longer delegate to a validator.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Option<Address>`): The basic account paying the transaction fee. If `None`, the fee is paid 
    ///   from the staker account's balance.
    /// - `staker_wallet` (`Address`): The staker account to be updated.
    /// - `new_delegation` (`Option<Address>`): The new validator to delegate to. If `None`, the staker will stop 
    ///   delegating.
    /// - `reactivate_all_stake` (`bool`): If `true`, all inactive stake will be reactivated.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height at which the transaction becomes valid.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the broadcasted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_update_staker_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `set_active_stake` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not providing a sender wallet).
    ///
    /// **Behavior**:  
    /// Sets the desired active stake, which automatically adjusts the inactive balance accordingly.
    /// For example, if a staker has 500 NIM and sets the active stake to 300 NIM, the inactive stake will be adjusted to 200 NIM.
    /// The inactive balance then becomes locked to account for potential validator misbehavior.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Option<Address>`): The address paying the transaction fee. If `None`, the fee is deducted from the staker account.
    /// - `staker_wallet` (`Address`): The address of the staker adjusting the active balance.
    /// - `new_active_balance` (`Coin`): The new amount of active stake to set.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `String`: The serialized transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `set_active_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not providing a sender wallet).
    ///
    /// **Behavior**:  
    /// Sets the desired active stake, which automatically adjusts the inactive balance accordingly.
    /// For example, if a staker has 500 NIM and sets the active stake to 300 NIM, the inactive stake will be adjusted to 200 NIM.
    /// The inactive balance then becomes locked to account for potential validator misbehavior.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Option<Address>`): The address paying the transaction fee. If `None`, the fee is deducted from the staker account.
    /// - `staker_wallet` (`Address`): The address of the staker adjusting the active balance.
    /// - `new_active_balance` (`Coin`): The new amount of active stake to set.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash of the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `retire_stake` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not providing a sender wallet).
    ///
    /// **Behavior**:  
    /// Moves funds from inactive to retired balance, making them eligible for withdrawal.  
    /// Only inactive funds released (post lock-up period) can be retired.  
    /// A staker can either retire all the non-retired stake or leave at least the minimum stake in the non-retired balance; otherwise, the transaction fails.  
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Option<Address>`): The address paying the transaction fee. If `None`, the fee is deducted from the staker account.
    /// - `staker_wallet` (`Address`): The address of the staker retiring the stake.
    /// - `retire_stake` (`Coin`): The amount of stake to retire (move to retired balance).
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `String`: A serialized transaction that can be sent to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_retire_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `retire_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not providing a sender wallet).
    ///
    /// **Behavior**:  
    /// Moves funds from inactive to retired balance, making them eligible for withdrawal.  
    /// Only inactive funds released (post lock-up period) can be retired.  
    /// A staker can either retire all the non-retired stake or leave at least the minimum stake in the non-retired balance; otherwise, the transaction fails.  
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Option<Address>`): The address paying the transaction fee. If `None`, the fee is deducted from the staker account.
    /// - `staker_wallet` (`Address`): The address of the staker retiring the stake.
    /// - `retire_stake` (`Coin`): The amount of stake to retire (move to retired balance).
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash of the sent transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_retire_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `remove_stake` transaction. The transaction fee will be paid from the funds being removed.
    ///
    /// **Behavior**:
    /// Withdraws funds from the retired stake balance and transfers them to the specified recipient.
    /// The transaction will fail if there are insufficient retired funds.
    /// The transaction fee is deducted from the withdrawn amount.
    ///
    /// **Parameters**:
    /// - `staker_wallet` (`Address`): The address of the staker removing the stake.
    /// - `recipient` (`Address`): The address to receive the withdrawn funds.
    /// - `value` (`Coin`): The amount of stake to remove.
    /// - `fee` (`Coin`): The transaction fee, which is deducted from the withdrawn amount.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `String`: A serialized `remove_stake` transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_remove_stake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `remove_stake` transaction to the network. The transaction fee will be paid from the funds being removed.
    ///
    /// **Behavior**:
    /// Withdraws the retired balance from a staker's account, removing it from the staking contract.
    /// The transaction must remove the entire retired balance; partial removals are not allowed.
    /// If the total balance drops to zero, the staker's account is deleted.
    ///
    /// **Parameters**:
    /// - `staker_wallet` (`Address`): The address of the staker removing the stake.
    /// - `recipient` (`Address`): The address to receive the withdrawn funds.
    /// - `value` (`Coin`): The amount of stake to remove (must be the entire retired balance).
    /// - `fee` (`Coin`): The transaction fee, which is deducted from the withdrawn amount.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The hash of the sent `remove_stake` transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_remove_stake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `new_validator` transaction. This transaction registers a new validator in the staking contract.
    ///
    /// **Behavior**:
    /// Registers a new validator with the provided signing and voting keys.
    /// The transaction must be sent from a basic account (`sender_wallet`), which pays the transaction fee and validator deposit.
    /// The validator deposit is locked and can only be withdrawn after retiring and deleting the validator.
    /// The `reward_address` is where staking rewards are credited.
    /// The `signal_data` field is optional and allows validators to attach additional metadata.
    ///
    /// **Special Handling for `signal_data`**:
    /// - Since JSON doesn't support a primitive for `Option`, the following work-around is used:
    ///   - `""` → Sets the signal data field to `None`.
    ///   - `"0x29a4b..."` → Sets the signal data field to `Some(0x29a4b...)`.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the validator deposit and transaction fee.
    /// - `validator_wallet` (`Address`): The address of the new validator.
    /// - `signing_secret_key` (`String`): The validator’s signing key.
    /// - `voting_secret_key` (`String`): The validator’s voting key.
    /// - `reward_address` (`Address`): The address to receive staking rewards.
    /// - `signal_data` (`String`): Optional metadata associated with the validator.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `String`: A serialized `new_validator` transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_new_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_secret_key: String,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `new_validator` transaction to the network, registering a new validator in the staking contract.
    ///
    /// **Behavior**:
    /// Registers a new validator with the provided signing and voting keys.
    /// The transaction must be sent from a basic account (`sender_wallet`), which pays the validator deposit and transaction fee.
    /// The validator deposit is locked and can only be withdrawn after retiring and deleting the validator.
    /// The `reward_address` is where staking rewards are credited.
    /// The `signal_data` field is optional and allows validators to attach additional metadata.
    ///
    /// **Special Handling for `signal_data`**:
    /// - Since JSON doesn't support a primitive for `Option`, the following work-around is used:
    ///   - `""` → Sets the signal data field to `None`.
    ///   - `"0x29a4b..."` → Sets the signal data field to `Some(0x29a4b...)`.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the validator deposit and transaction fee.
    /// - `validator_wallet` (`Address`): The address of the new validator.
    /// - `signing_secret_key` (`String`): The validator’s signing key.
    /// - `voting_secret_key` (`String`): The validator’s voting key.
    /// - `reward_address` (`Address`): The address to receive staking rewards.
    /// - `signal_data` (`String`): Optional metadata associated with the validator.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash of the submitted `new_validator` transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_new_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_secret_key: String,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `update_validator` transaction, modifying an existing validator's details in the staking contract.
    ///
    /// **Behavior**:
    /// Allows the validator to update details such as the signing key, voting key, reward address, and signal data.
    /// The validator must be active to apply these changes.
    /// The transaction must be sent from a basic account (`sender_wallet`), which pays the transaction fee.
    /// If a field is set to `None`, it remains unchanged; otherwise, it updates with the new value.
    ///
    /// **Special Handling for `new_signal_data`**:
    /// - Since JSON doesn't support a primitive for `Option`, the following work-around is used:
    ///   - `null` → No change in the signal data field.
    ///   - `""` → Clears the signal data field (sets it to `None`).
    ///   - `"0x29a4b..."` → Updates the signal data field to `Some(0x29a4b...)`.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_wallet` (`Address`): The address of the validator to update.
    /// - `new_signing_secret_key` (`Option<String>`): The new signing key (if changed).
    /// - `new_voting_secret_key` (`Option<String>`): The new voting key (if changed).
    /// - `new_reward_address` (`Option<Address>`): The new reward address (if changed).
    /// - `new_signal_data` (`Option<String>`): The new signal data (if changed).
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `String`: The serialized transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_update_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_secret_key: Option<String>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends an `update_validator` transaction to the network, modifying an existing validator's details in the staking contract.
    ///
    /// **Behavior**:
    /// Allows the validator to update details such as the signing key, voting key, reward address, and signal data.
    /// The validator must be active to apply these changes.
    /// The transaction must be sent from a basic account (`sender_wallet`), which pays the transaction fee.
    /// If a field is set to `None`, it remains unchanged; otherwise, it updates with the new value.
    ///
    /// **Special Handling for `new_signal_data`**:
    /// - Since JSON doesn't support `Option<Option<T>>`, the following work-around is used:
    ///   - `null` → No change in the signal data field.
    ///   - `""` → Clears the signal data field (sets it to `None`).
    ///   - `"0x29a4b..."` → Updates the signal data field to `Some(0x29a4b...)`.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_wallet` (`Address`): The address of the validator to update.
    /// - `new_signing_secret_key` (`Option<String>`): The new signing key (if changed).
    /// - `new_voting_secret_key` (`Option<String>`): The new voting key (if changed).
    /// - `new_reward_address` (`Option<Address>`): The new reward address (if changed).
    /// - `new_signal_data` (`Option<String>`): The new signal data (if changed).
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:  
    /// - `Blake2bHash`: The transaction hash of the update transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_update_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_secret_key: Option<String>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `deactivate_validator` transaction.
    ///
    /// **Behavior**:
    /// This transaction moves the validator to an inactive state, where it no longer participates in block validation.
    /// 
    /// Deactivation is scheduled to take effect at the next election block, meaning the validator remains active until then.
    /// Deactivation is a necessary step before a validator can be retired or deleted.
    /// 
    /// If a validator remains offline for an extended period, it is automatically deactivated and loses any rewards for the time it remains inactive.
    /// The validator's stake remains locked until the deactivation period ends.
    /// This transaction must be sent from a basic account (`sender_wallet`), which pays the transaction fee.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_address` (`Address`): The address of the validator to deactivate.
    /// - `signing_secret_key` (`String`): The secret key of the validator for authorization.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:
    /// - `String`: The serialized transaction data.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_deactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `deactivate_validator` transaction to the network.
    ///
    /// **Behavior**:
    /// This transaction moves the validator to an inactive state, where it no longer participates in block validation.
    /// 
    /// Deactivation takes effect at the next election block, meaning the validator remains active until then.
    /// Deactivation is required before a validator can be retired or deleted.
    /// 
    /// If a validator remains offline for an extended period, it is automatically deactivated and forfeits any rewards for that period.
    /// The validator's stake remains locked until the deactivation period ends.
    /// The transaction fee is paid from the `sender_wallet`, which must be a basic account.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_address` (`Address`): The address of the validator to deactivate.
    /// - `signing_secret_key` (`String`): The secret key of the validator for authorization.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:
    /// - `Blake2bHash`: The transaction hash of the submitted deactivation.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_deactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `reactivate_validator` transaction.
    ///
    /// **Behavior**:
    /// A validator can send this transaction to transition from an inactive state back to active.
    /// Reactivation is possible after the punishment period has ended (e.g., due to delayed block production or a temporary jail period).
    /// 
    /// The validator must meet the following conditions:
    ///   - Must be currently inactive.
    ///   - Must not be retired.
    ///   - Must not be jailed at the time of reactivation.
    /// 
    /// Reactivating a validator restores its ability to participate in block validation and earn rewards.
    /// This transaction is **not required** if the validator has the `automatic_reactivate` setting enabled.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_address` (`Address`): The address of the validator to reactivate.
    /// - `signing_secret_key` (`String`): The secret key of the validator for authorization.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:
    /// - `String`: The serialized transaction data.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `reactivate_validator` transaction to the network.
    ///
    /// **Behavior**:
    /// A validator can send this transaction to transition from an inactive state back to active.
    /// Reactivation is possible after the punishment period has ended (e.g., due to delayed block production or a temporary jail period).
    /// 
    /// The validator must meet the following conditions:
    ///   - Must be currently inactive.
    ///   - Must not be retired.
    ///   - Must not be jailed at the time of reactivation.
    /// 
    /// Reactivating a validator restores its ability to participate in block validation and earn rewards.
    /// This transaction is **not required** if the validator has the `automatic_reactivate` setting enabled.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_address` (`Address`): The address of the validator to reactivate.
    /// - `signing_secret_key` (`String`): The secret key of the validator for authorization.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:
    /// - `Blake2bHash`: The transaction hash of the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    async fn send_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `retire_validator` transaction.
    ///
    /// **Behavior**:
    /// Retiring a validator is an **irreversible action** that prevents further participation in block validation.
    /// This is the **first step** in the process of deleting a validator.
    /// This transaction prepares the validator for **eventual deletion** and the return of the deposit.
    /// Once a validator is retired, **it cannot be reactivated**.
    /// If the validator is still active, retirement **automatically transitions it to an inactive state**.
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_wallet` (`Address`): The address of the validator being retired.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:
    /// - `String`: A serialized transaction that can be signed and sent to the network.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn create_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `retire_validator` transaction to the network.
    ///
    /// **Behavior**:
    /// Retiring a validator is an **irreversible action** that prevents further participation in block validation.
    /// This is the **first step** in the process of deleting a validator.
    /// This transaction prepares the validator for **eventual deletion** and the return of the deposit.
    /// Once a validator is retired, **it cannot be reactivated**.
    /// If the validator is still active, retirement **automatically transitions it to an inactive state**.
    ///
    ///
    /// **Parameters**:
    /// - `sender_wallet` (`Address`): The address paying the transaction fee.
    /// - `validator_wallet` (`Address`): The address of the validator being retired.
    /// - `fee` (`Coin`): The transaction fee.
    /// - `validity_start_height` (`ValidityStartHeight`): The starting block height for transaction validity.
    ///
    /// **Returns**:
    /// - `Blake2bHash`: The transaction hash of the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": "string",
    ///   "metadata": null
    /// }
    /// ```
    async fn send_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Creates a serialized `delete_validator` transaction. This method generates a `delete_validator` transaction but does **not** broadcast it to the network.
    /// To broadcast the transaction, use [`send_delete_validator_transaction`].
    /// 
    /// The transaction fee will be paid from the validator deposit that is being returned.
    /// For the transaction to be accepted, `fee + value` must be **equal to the validator deposit**.
    /// Failed delete validator transactions **may reduce the validator deposit**.
    /// 
    /// **Behavior**:
    /// 
    /// Returns the validator's deposit to the specified recipient address.
    /// 
    /// A validator can only be deleted if:
    ///   - It has completed the cooldown period after retirement.
    ///   - All delegations have been withdrawn.
    /// 
    /// If there are remaining stakers who haven’t withdrawn, a tombstone record is created to track the remaining stake.
    ///
    /// **Parameters**:
    /// - `validator_wallet` (`Address`): The address of the validator wallet submitting the deletion transaction.
    /// - `recipient` (`Address`): The address where the remaining funds will be sent.
    /// - `fee` (`Coin`): The transaction fee to be deducted from the validator deposit.
    /// - `value` (`Coin`): The amount to be withdrawn after the validator is deleted.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height from which the transaction is valid.
    ///
    /// **Returns**:
    /// - `String`: The serialized transaction, ready to be signed and sent.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": "string",
    ///     "metadata": null
    ///   }
    /// }
    /// ```
    async fn create_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `delete_validator` transaction to the network.
    ///
    /// The transaction fee will be paid from the validator deposit that is being returned.
    /// 
    /// **Behavior**:
    /// 
    /// Returns the validator's deposit to the specified recipient address.
    /// 
    /// A validator can only be deleted if:
    ///   - It has completed the cooldown period after retirement.
    ///   - All delegations have been withdrawn.
    /// 
    /// If there are remaining stakers who haven’t withdrawn, a tombstone record is created to track the remaining stake.
    ///
    /// **Parameters**:
    /// - `validator_wallet` (`Address`): The address of the validator wallet submitting the deletion transaction.
    /// - `recipient` (`Address`): The address where the remaining funds will be sent.
    /// - `fee` (`Coin`): The transaction fee to be deducted from the validator deposit.
    /// - `value` (`Coin`): The amount to be withdrawn after the validator is deleted.
    /// - `validity_start_height` (`ValidityStartHeight`): The block height from which the transaction is valid.
    ///
    /// **Returns**:
    /// - `Blake2bHash`: The hash of the submitted transaction.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": "string",
    ///     "metadata": null
    /// }
    /// ```
    async fn send_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;
}
