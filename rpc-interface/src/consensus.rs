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

    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_consensus_established(&mut self) -> RPCResult<bool, (), Self::Error>;

    async fn get_raw_transaction_info(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Transaction, (), Self::Error>;

    async fn send_raw_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

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

    async fn create_redeem_vesting_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_redeem_vesting_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

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

    async fn create_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

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

    async fn sign_redeem_early_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn create_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_update_staker_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_update_staker_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_set_active_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_retire_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_retire_stake_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_remove_stake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_remove_stake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

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

    async fn create_deactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_deactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_retire_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    async fn create_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    async fn send_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;
}
