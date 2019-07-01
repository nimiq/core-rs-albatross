use std::str::FromStr;
use std::sync::Arc;

use json::{Array, JsonValue, Null};

use nimiq_database::Environment;
use keys::{KeyPair, PrivateKey};
use nimiq_wallet::{WalletAccount, WalletStore};

use crate::handlers::Handler;
use crate::rpc_not_implemented;

pub struct WalletHandler {
    pub wallet_store: WalletStore<'static>,
}

impl WalletHandler {
    pub(crate) fn new(env: &'static Environment) -> Self {
        WalletHandler {
            wallet_store: WalletStore::new(env),
        }
    }

    /// Imports a raw hex-string private key.
    /// Parameters:
    /// - key data (string): The hex encoded private key.
    /// - passphrase (optional, string): The passphrase to lock the key with.
    /// Returns the user friendly address corresponding to the private key.
    pub(crate) fn import_raw_key(&self, params: &Array) -> Result<JsonValue, JsonValue> {
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
    pub(crate) fn list_accounts(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        Ok(JsonValue::Array(self.wallet_store.list(None).iter().map(|wallet_account| {
            JsonValue::String(wallet_account.address.to_user_friendly_address())
        }).collect()))
    }

    /// Removes the unencrypted private key from memory.
    /// Parameters:
    /// - address (string)
    pub(crate) fn lock_account(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    /// Creates a new account.
    /// Parameters:
    /// - passphrase (optional, string): The passphrase to lock the key with.
    /// Returns the user friendly address corresponding to the private key.
    pub(crate) fn new_account(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
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
    pub(crate) fn unlock_account(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    /// Signs and sends a transaction, while temporarily unlocking the corresponding private key.
    /// Parameters:
    /// - transaction (object)
    /// - passphrase (string)
    /// Returns the transaction hash.
    pub(crate) fn send_transaction(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    pub(crate) fn sign(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }

    pub(crate) fn ec_recover(&self, _params: &Array) -> Result<JsonValue, JsonValue> {
        // TODO: Implement
        rpc_not_implemented()
    }
}

impl WalletHandler {
    const PREFIX: &'static str = "personal_";
}

impl Handler for WalletHandler {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>> {
        if !name.starts_with(Self::PREFIX) {
            return None;
        }

        match &name[Self::PREFIX.len()..] {
            // Wallet
            "importRawKey" => Some(self.import_raw_key(params)),
            "listAccounts" => Some(self.list_accounts(params)),
            "lockAccount" => Some(self.lock_account(params)),
            "newAccount" => Some(self.new_account(params)),
            "unlockAccount" => Some(self.unlock_account(params)),
            "sendTransaction" => Some(self.send_transaction(params)),
            "sign" => Some(self.sign(params)),
            "ec_recover" => Some(self.ec_recover(params)),

            _ => None
        }
    }
}
