use async_trait::async_trait;

use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey};
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{AnyHash, HashAlgorithm};

use crate::types::{Transaction, ValidityStartHeight};

#[nimiq_jsonrpc_derive::proxy(name = "ConsensusProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ConsensusInterface {
    type Error;

    async fn is_consensus_established(&mut self) -> Result<bool, Self::Error>;

    async fn get_raw_transaction_info(
        &mut self,
        raw_tx: String,
    ) -> Result<Transaction, Self::Error>;

    async fn send_raw_transaction(&mut self, raw_tx: String) -> Result<Blake2bHash, Self::Error>;

    async fn create_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_basic_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: Vec<u8>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_basic_transaction_with_data(
        &mut self,
        wallet: Address,
        recipient: Address,
        data: Vec<u8>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

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
    ) -> Result<String, Self::Error>;

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
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_redeem_vesting_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_redeem_vesting_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_new_htlc_transaction(
        &mut self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        hash_algorithm: HashAlgorithm,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_new_htlc_transaction(
        &mut self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        hash_algorithm: HashAlgorithm,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_redeem_regular_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: AnyHash,
        hash_root: AnyHash,
        hash_count: u8,
        hash_algorithm: HashAlgorithm,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_redeem_regular_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: AnyHash,
        hash_root: AnyHash,
        hash_count: u8,
        hash_algorithm: HashAlgorithm,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_redeem_timeout_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_redeem_early_htlc_transaction(
        &mut self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_redeem_early_htlc_transaction(
        &mut self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn sign_redeem_early_htlc_transaction(
        &mut self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn create_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_new_staker_transaction(
        &mut self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_stake_transaction(
        &mut self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_update_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_update_transaction(
        &mut self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_unstake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_unstake_transaction(
        &mut self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_new_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_key: PublicKey,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_new_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_key: PublicKey,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_update_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_key: Option<PublicKey>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_update_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_key: Option<PublicKey>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_inactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_inactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_reactivate_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_unpark_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_unpark_validator_transaction(
        &mut self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_delete_validator_transaction(
        &mut self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;
}
