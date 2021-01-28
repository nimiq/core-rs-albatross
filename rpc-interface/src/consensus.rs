use async_trait::async_trait;

use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_bls::CompressedPublicKey;

use crate::types::ValidityStartHeight;

#[cfg_attr(feature = "proxy", nimiq_jsonrpc_derive::proxy(name = "ConsensusProxy", rename_all = "camelCase"))]
#[async_trait]
pub trait ConsensusInterface {
    type Error;

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
        validator_key: CompressedPublicKey,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_stake_transaction(
        &mut self,
        wallet: Address,
        validator_key: CompressedPublicKey,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_retire_transaction(
        &mut self,
        wallet: Address,
        validator_key: CompressedPublicKey,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_retire_transaction(
        &mut self,
        wallet: Address,
        validator_key: CompressedPublicKey,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<Blake2bHash, Self::Error>;

    async fn create_reactivate_transaction(
        &mut self,
        wallet: Address,
        validator_key: CompressedPublicKey,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> Result<String, Self::Error>;

    async fn send_reactivate_transaction(
        &mut self,
        wallet: Address,
        validator_key: CompressedPublicKey,
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
        validity_start_height: ValidityStartHeight
    ) -> Result<String, Self::Error>;

    async fn send_unstake_transaction(
        &mut self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight
    ) -> Result<Blake2bHash, Self::Error>;
}
