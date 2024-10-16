use async_trait::async_trait;
use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature};

use crate::types::{RPCResult, ReturnAccount, ReturnSignature};

#[nimiq_jsonrpc_derive::proxy(name = "WalletProxy", rename_all = "camelCase")]
#[async_trait]
pub trait WalletInterface {
    type Error;

    /// Import an account by its private key, in hexadecimal format, and lock it with the passphrase.
    async fn import_raw_key(
        &mut self,
        key_data: String,
        passphrase: Option<String>,
    ) -> RPCResult<Address, (), Self::Error>;

    /// Returns if an account has been imported.
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_account_imported(&mut self, address: Address) -> RPCResult<bool, (), Self::Error>;

    /// Returns the accounts that have been imported.
    async fn list_accounts(&mut self) -> RPCResult<Vec<Address>, (), Self::Error>;

    /// Locks the account to prevent further usage.
    async fn lock_account(&mut self, address: Address) -> RPCResult<(), (), Self::Error>;

    /// Generates a new account and store it.
    async fn create_account(
        &mut self,
        passphrase: Option<String>,
    ) -> RPCResult<ReturnAccount, (), Self::Error>;

    /// Unlocks the account.
    async fn unlock_account(
        &mut self,
        address: Address,
        passphrase: Option<String>,
        duration: Option<u64>,
    ) -> RPCResult<bool, (), Self::Error>;

    /// Removes an imported account.
    /// IMPORTANT: This action is irreversible, and the account can only be recovered with its private key.
    async fn remove_account(&mut self, address: Address) -> RPCResult<bool, (), Self::Error>;

    /// Returns if the account currently is unlocked.
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_account_unlocked(&mut self, address: Address) -> RPCResult<bool, (), Self::Error>;

    async fn sign(
        &mut self,
        message: String,
        address: Address,
        passphrase: Option<String>,
        is_hex: bool,
    ) -> RPCResult<ReturnSignature, (), Self::Error>;

    /// Verifies the signature based on the provided public key and message.
    async fn verify_signature(
        &mut self,
        message: String,
        public_key: Ed25519PublicKey,
        signature: Ed25519Signature,
        is_hex: bool,
    ) -> RPCResult<bool, (), Self::Error>;
}
