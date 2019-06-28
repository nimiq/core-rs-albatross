use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};

use keys::{KeyPair, PrivateKey};
use nimiq_wallet::{WalletAccount, WalletStore};

use crate::rpc_not_implemented;

pub struct WalletHandler {
    pub wallet_store: Arc<WalletStore<'static>>,
}

impl WalletHandler {
    /// Imports a raw hex-string private key.
    /// Parameters:
    /// - key data (string): The hex encoded private key.
    /// - passphrase (optional, string): The passphrase to lock the key with.
    /// Returns the user friendly address corresponding to the private key.
    pub fn import_raw_key(&self, params: Array) -> Result<JsonValue, JsonValue> {
        let private_key = params.get(0).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Missing private key data"})?;
        let private_key = PrivateKey::from_str(private_key)
            .map_err(|e| object!{"message" => e.to_string()})?;
        let wallet_account = WalletAccount::from(KeyPair::from(private_key));

        // TODO: Passphrase is currently ignored.

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&wallet_account, &mut txn);

        Ok(JsonValue::String(wallet_account.address.to_user_friendly_address()))
        // TODO: The unencrypted string might still be in memory. Clear it.
    }

    /// Returns a list of all user friendly addresses in the store.
    pub fn list_accounts(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        Ok(JsonValue::Array(self.wallet_store.list(None).iter().map(|wallet_account| {
            JsonValue::String(wallet_account.address.to_user_friendly_address())
        }).collect()))
    }

    /// Removes the unencrypted private key from memory.
    /// Parameters:
    /// - address (string)
    pub fn lock_account(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    /// Creates a new account.
    /// Parameters:
    /// - passphrase (optional, string): The passphrase to lock the key with.
    /// Returns the user friendly address corresponding to the private key.
    pub fn new_account(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // TODO: Passphrase is currently ignored.

        let wallet_account = WalletAccount::generate();

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&wallet_account, &mut txn);

        Ok(JsonValue::String(wallet_account.address.to_user_friendly_address()))
        // TODO: The unencrypted string might still be in memory. Clear it.
    }

    /// Unlocks a wallet account in memory.
    /// Parameters:
    /// - address (string)
    /// - passphrase (string)
    /// - duration (number)
    pub fn unlock_account(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    /// Signs and sends a transaction, while temporarily unlocking the corresponding private key.
    /// Parameters:
    /// - transaction (object)
    /// - passphrase (string)
    /// Returns the transaction hash.
    pub fn send_transaction(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    pub fn sign(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }
    pub fn ec_recover(&self, _params: Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }
}
