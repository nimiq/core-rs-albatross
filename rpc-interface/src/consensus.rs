use async_trait::async_trait;

use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;

use crate::types::ValidityStartHeight;

#[cfg_attr(
    feature = "proxy",
    nimiq_jsonrpc_derive::proxy(name = "ConsensusProxy", rename_all = "camelCase")
)]
#[async_trait]
pub trait ConsensusInterface {
    type Error;

    async fn is_established(&mut self) -> Result<bool, Self::Error>;

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

    async fn create_new_staker_transaction(
        &mut self,
        wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_new_staker_transaction(
        &mut self,
        wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_stake_transaction(
        &mut self,
        wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_stake_transaction(
        &mut self,
        wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_update_transaction(
        &mut self,
        wallet: Address,
        new_delegation: Option<Address>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_update_transaction(
        &mut self,
        wallet: Address,
        new_delegation: Option<Address>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_retire_transaction(
        &mut self,
        wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_retire_transaction(
        &mut self,
        wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_reactivate_transaction(
        &mut self,
        wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_reactivate_transaction(
        &mut self,
        wallet: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_unstake_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_unstake_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_new_validator_transaction(
        &mut self,
        wallet: Address,
        warm_key: Address,
        validator_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_new_validator_transaction(
        &mut self,
        wallet: Address,
        warm_key: Address,
        validator_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_update_validator_transaction(
        &mut self,
        wallet: Address,
        new_warm_address: Option<Address>,
        new_validator_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_update_validator_transaction(
        &mut self,
        wallet: Address,
        new_warm_address: Option<Address>,
        new_validator_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_retire_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_retire_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        warm_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_drop_validator_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_drop_validator_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;
}
