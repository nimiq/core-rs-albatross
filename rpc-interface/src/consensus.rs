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

    /// Returns a boolean specifying if we have established consensus with the network.
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_consensus_established(&mut self) -> RPCResult<bool, (), Self::Error>;

    /// Given a serialized transaction, it will return the corresponding transaction struct.
    async fn get_raw_transaction_info(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Transaction, (), Self::Error>;

    /// Sends the given serialized transaction to the network.
    async fn send_raw_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized basic transaction.
    async fn create_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a basic transaction to the network.
    async fn send_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized basic transaction with an arbitrary data field.
    async fn create_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a basic transaction, with an arbitrary data field, to the network.
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

    /// Returns a serialized transaction redeeming a HTLC contract
    /// using the `RegularTransfer` method.
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

    /// Sends a transaction redeeming a HTLC contract, using the `RegularTransfer` method, to the
    /// network.
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

    /// Returns a serialized transaction redeeming a HTLC contract using the `TimeoutResolve`
    /// method.
    async fn create_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming a HTLC contract, using the `TimeoutResolve` method, to the
    /// network.
    async fn send_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming a HTLC contract using the `EarlyResolve`
    /// method.
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

    /// Sends a transaction redeeming a HTLC contract, using the `EarlyResolve` method, to the
    /// network.
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

    /// Returns a serialized signature that can be used to redeem funds from a HTLC contract using
    /// the `EarlyResolve` method.
    async fn sign_redeem_early_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Returns a serialized `new_staker` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `new_staker` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `stake` transaction. The funds to be staked and the transaction fee will
    /// be paid from the `sender_wallet`.
    async fn create_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `stake` transaction to the network. The funds to be staked and the transaction fee will
    /// be paid from the `sender_wallet`.
    async fn send_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `update_staker` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_update_staker_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `update_staker` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
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
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `set_active_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `retire_stake` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_retire_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `retire_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_retire_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `remove_stake` transaction. The transaction fee will be paid from the funds
    /// being removed.
    async fn create_remove_stake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `remove_stake` transaction to the network. The transaction fee will be paid from the funds
    /// being removed.
    async fn send_remove_stake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `new_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// "" = Set the signal data field to None.
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
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

    /// Sends a `new_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// "" = Set the signal data field to None.
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
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

    /// Returns a serialized `update_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// null = No change in the signal data field.
    /// "" = Change the signal data field to None.
    /// "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
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

    /// Sends a `update_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// null = No change in the signal data field.
    /// "" = Change the signal data field to None.
    /// "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
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

    /// Returns a serialized `deactivate_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_deactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `deactivate_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_deactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `reactivate_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `reactivate_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `retire_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `retire_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `delete_validator` transaction. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    /// Note in order for this transaction to be accepted fee + value should be equal to the validator deposit, which is not a fixed value:
    /// Failed delete validator transactions can diminish the validator deposit
    async fn create_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `delete_validator` transaction to the network. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    async fn send_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;
}
