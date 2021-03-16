use async_trait::async_trait;

use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::account::ValidatorId;
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

    async fn create_stake_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_stake_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_rededicate_transaction(
        &mut self,
        wallet: Address,
        from_validator_id: ValidatorId,
        to_validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_rededicate_transaction(
        &mut self,
        wallet: Address,
        from_validator_id: ValidatorId,
        to_validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_retire_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_retire_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_reactivate_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_reactivate_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
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
        reward_address: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_new_validator_transaction(
        &mut self,
        wallet: Address,
        reward_address: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_update_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        new_reward_address: Option<Address>,
        old_validator_secret_key: String,
        new_validator_secret_key: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_update_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        new_reward_address: Option<Address>,
        old_validator_secret_key: String,
        new_validator_secret_key: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_retire_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_retire_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_reactivate_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_drop_validator_transaction(
        &mut self,
        validator_id: ValidatorId,
        recipient: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_drop_validator_transaction(
        &mut self,
        validator_id: ValidatorId,
        recipient: Address,
        validator_secret_key: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_unpark_validator_transaction(
        &mut self,
        wallet: Address,
        validator_id: ValidatorId,
        validator_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;
}
