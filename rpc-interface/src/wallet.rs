use async_trait::async_trait;

use nimiq_keys::{Address, PrivateKey, PublicKey, Signature};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnSignature {
    pub public_key: PublicKey,
    pub signature: Signature,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnAccount {
    pub address: Address,
    pub public_key: PublicKey,
    pub private_key: PrivateKey,
}

#[cfg_attr(
    feature = "proxy",
    nimiq_jsonrpc_derive::proxy(name = "WalletProxy", rename_all = "camelCase")
)]
#[async_trait]
pub trait WalletInterface {
    type Error;

    async fn import_raw_key(
        &mut self,
        key_data: String,
        passphrase: Option<String>,
    ) -> Result<Address, Self::Error>;

    async fn is_account_imported(&mut self, address: Address) -> Result<bool, Self::Error>;

    async fn list_accounts(&mut self) -> Result<Vec<Address>, Self::Error>;

    async fn lock_account(&mut self, address: Address) -> Result<(), Self::Error>;

    async fn create_account(
        &mut self,
        passphrase: Option<String>,
    ) -> Result<ReturnAccount, Self::Error>;

    async fn unlock_account(
        &mut self,
        address: Address,
        passphrase: Option<String>,
        duration: Option<u64>,
    ) -> Result<(), Self::Error>;

    async fn is_account_unlocked(&mut self, address: Address) -> Result<bool, Self::Error>;

    async fn sign(
        &mut self,
        message: String,
        address: Address,
        passphrase: Option<String>,
        is_hex: bool,
    ) -> Result<ReturnSignature, Self::Error>;

    async fn verify_signature(
        &mut self,
        message: String,
        public_key: PublicKey,
        signature: Signature,
        is_hex: bool,
    ) -> Result<bool, Self::Error>;
}
