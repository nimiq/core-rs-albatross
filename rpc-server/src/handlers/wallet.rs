use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use hex;
use json::{JsonValue, Null};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use keys::{Address, KeyPair, PrivateKey, PublicKey, Signature};
use nimiq_database::Environment;
use nimiq_wallet::{WalletAccount, WalletStore};
use utils::otp::{Locked, Unlocked};

use crate::handler::Method;
use crate::handlers::Module;

pub struct UnlockedWalletManager {
    pub unlocked_wallets: HashMap<Address, Unlocked<WalletAccount>>,
}

impl UnlockedWalletManager {
    fn new() -> Self {
        UnlockedWalletManager {
            unlocked_wallets: HashMap::new(),
        }
    }

    fn insert(&mut self, wallet: Unlocked<WalletAccount>) {
        info!("Unlocking {:?}", &wallet.address);
        self.unlocked_wallets.insert(wallet.address.clone(), wallet);
    }

    pub fn get(&self, address: &Address) -> Option<&WalletAccount> {
        info!("Accessing {:?}", address);
        self.unlocked_wallets.get(address).map(|unlocked| Unlocked::unlocked_data(unlocked))
    }

    fn remove(&mut self, address: &Address) -> Option<Unlocked<WalletAccount>> {
        self.unlocked_wallets.remove(address)
    }
}

pub struct WalletHandler {
    wallet_store: WalletStore,
    pub unlocked_wallets: Arc<RwLock<UnlockedWalletManager>>,
}

impl WalletHandler {
    pub fn new(env: Environment) -> Self {
        WalletHandler {
            wallet_store: WalletStore::new(env),
            unlocked_wallets: Arc::new(RwLock::new(UnlockedWalletManager::new())),
        }
    }

    /// Imports a raw hex-string private key.
    /// Parameters:
    /// - key data (string): The hex encoded private key.
    /// - passphrase (optional, string): The passphrase to lock the key with.
    /// Returns the user friendly address corresponding to the private key.
    pub(crate) fn import_raw_key(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let private_key = params.get(0).unwrap_or(&Null).as_str()
            .ok_or_else(|| object!{"message" => "Missing private key data"})?;
        let private_key = PrivateKey::from_str(private_key)
            .map_err(|e| object!{"message" => e.to_string()})?;

        // FIXME: We're not clearing the passphrase right now.
        let passphrase = params.get(1).map(|s: &JsonValue| s.as_str()
                .ok_or_else(|| object!{"message" => "Passphrase must be a string"})
            ).unwrap_or_else(|| Ok(""))?;

        let wallet_account = WalletAccount::from(KeyPair::from(private_key));
        let address = wallet_account.address.clone();
        let wallet_account = Locked::with_defaults(wallet_account, passphrase.as_bytes())
            .map_err(|e| object!{"message" => format!("Error while importing: {:?}", e)})?;

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&address, &wallet_account, &mut txn);
        txn.commit();

        Ok(JsonValue::String(address.to_user_friendly_address()))
    }

    /// Returns a list of all user friendly addresses in the store.
    pub(crate) fn list_accounts(&self, _params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        Ok(JsonValue::Array(self.wallet_store.list(None).iter().map(|address| {
            JsonValue::String(address.to_user_friendly_address())
        }).collect()))
    }

    /// Removes the unencrypted private key from memory.
    /// Parameters:
    /// - address (string)
    pub(crate) fn lock_account(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let account_address = Address::from_any_str(params.get(0)
            .unwrap_or_else(|| &Null).as_str()
            .ok_or_else(|| object!{"message" => "Address must be a string"})?)
            .map_err(|_|  object!{"message" => "Address invalid"})?;
        self.unlocked_wallets.write().remove(&account_address);
        Ok(JsonValue::Boolean(true))
    }

    /// Creates a new account.
    /// Parameters:
    /// - passphrase (optional, string): The passphrase to lock the key with.
    /// Returns the user friendly address corresponding to the private key.
    pub(crate) fn new_account(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        // FIXME: We're not clearing the passphrase right now.
        let passphrase = params.get(0).map(|s: &JsonValue| s.as_str()
                .ok_or_else(|| object!{"message" => "Passphrase must be a string"})
            ).unwrap_or_else(|| Ok(""))?;

        let wallet_account = WalletAccount::generate();
        let address = wallet_account.address.clone();
        let wallet_account = Locked::with_defaults(wallet_account, passphrase.as_bytes())
            .map_err(|e| object!{"message" => format!("Error while importing: {:?}", e)})?;

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&address, &wallet_account, &mut txn);
        txn.commit();

        Ok(JsonValue::String(address.to_user_friendly_address()))
    }

    /// Unlocks a wallet account in memory.
    /// Parameters:
    /// - address (string)
    /// - passphrase (string)
    /// - duration (number, not supported yet)
    pub(crate) fn unlock_account(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let account_address = Address::from_any_str(params.get(0)
            .unwrap_or_else(|| &Null).as_str()
            .ok_or_else(|| object!{"message" => "Address must be a string"})?)
            .map_err(|_|  object!{"message" => "Address invalid"})?;

        // FIXME: We're not clearing the passphrase right now.
        let passphrase = params.get(1).map(|s: &JsonValue| s.as_str()
                .ok_or_else(|| object!{"message" => "Passphrase must be a string"})
            ).unwrap_or_else(|| Ok(""))?;

        // TODO: duration

        let account = self.wallet_store.get(&account_address, None)
            .ok_or_else(|| object!{"message" => "Address does not exist"})?;

        let unlocked_account = account.unlock(passphrase.as_bytes());
        if let Ok(unlocked_account) = unlocked_account {
            self.unlocked_wallets.write().insert(unlocked_account);
            Ok(JsonValue::Boolean(true))
        } else {
            Err(object!{"message" => "Invalid passphrase"})
        }
    }

    /// Signs a message with a given address.
    /// Parameters:
    /// - message (string): If binary data is required, transmit hex encoded and set isHex flag.
    /// - address (string)
    /// - passphrase (string, optional)
    /// - isHex (bool, optional): Default is `false`.
    ///
    /// The return value is an object:
    /// {
    ///     publicKey: string,
    ///     signature: string,
    /// }
    pub(crate) fn sign(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let is_hex = params.get(3).unwrap_or_else(|| &JsonValue::Boolean(false)).as_bool()
            .ok_or_else(|| object!{"message" => "Optional isHex argument must be a boolean value"})?;

        let message = if is_hex {
            hex::decode(params.get(0)
                .unwrap_or(&Null)
                .as_str()
                .ok_or_else(|| object!{"message" => "Message must be a string"})?)
                .map_err(|_| object!{"message" => "Message must be a hex string if isHex is set"} )?
        } else {
            params.get(0)
                .unwrap_or(&Null)
                .as_str()
                .ok_or_else(|| object!{"message" => "Message must be a string"})?
                .as_bytes().to_vec()
        };

        let account_address = Address::from_any_str(params.get(1)
            .unwrap_or_else(|| &Null).as_str()
            .ok_or_else(|| object!{"message" => "Address must be a string"})?)
            .map_err(|_|  object!{"message" => "Address invalid"})?;

        // FIXME: We're not clearing the passphrase right now.
        let passphrase = params.get(1).map(|s: &JsonValue| s.as_str()
            .ok_or_else(|| object!{"message" => "Passphrase must be a string"}))
            .unwrap_or_else(|| Ok(""))?;

        if let Some(wallet) = self.unlocked_wallets.read().get(&account_address) {
            Ok(self.sign_message(&message, &wallet))
        } else {
            let account = self.wallet_store.get(&account_address, None)
                .ok_or_else(|| object!{"message" => "Address does not exist"})?;

            let unlocked_account = account.unlock(passphrase.as_bytes());
            if let Ok(unlocked_account) = unlocked_account {
                Ok(self.sign_message(&message, &unlocked_account))
            } else {
                Err(object!{"message" => "Invalid passphrase"})
            }
        }
    }

    /// Verifies a signature.
    /// Parameters:
    /// - message (string): If binary data is required, transmit hex encoded and set isHex flag.
    /// - publicKey (string)
    /// - signature (string)
    /// - isHex (bool, optional): Default is `false`.
    ///
    /// The return value is a boolean.
    pub(crate) fn verify_signature(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let is_hex = params.get(3).unwrap_or_else(|| &JsonValue::Boolean(false)).as_bool()
            .ok_or_else(|| object!{"message" => "Optional isHex argument must be a boolean value"})?;

        let message = if is_hex {
            hex::decode(params.get(0)
                .unwrap_or(&Null)
                .as_str()
                .ok_or_else(|| object!{"message" => "Message must be a string"})?)
                .map_err(|_| object!{"message" => "Message must be a hex string if isHex is set"} )?
        } else {
            params.get(0)
                .unwrap_or(&Null)
                .as_str()
                .ok_or_else(|| object!{"message" => "Message must be a string"})?
                .as_bytes().to_vec()
        };

        let raw = hex::decode(params.get(1)
            .unwrap_or(&Null)
            .as_str()
            .ok_or_else(|| object!{"message" => "Public key must be a string"} )?)
            .map_err(|_| object!{"message" => "Public key must be a hex string"} )?;
        let public_key: PublicKey = Deserialize::deserialize_from_vec(&raw)
            .map_err(|_| object!{"message" => "Public key can't be deserialized"} )?;

        let raw = hex::decode(params.get(2)
            .unwrap_or(&Null)
            .as_str()
            .ok_or_else(|| object!{"message" => "Signature must be a string"} )?)
            .map_err(|_| object!{"message" => "Signature must be a hex string"} )?;
        let signature: Signature = Deserialize::deserialize_from_vec(&raw)
            .map_err(|_| object!{"message" => "Signature can't be deserialized"} )?;

        Ok(JsonValue::Boolean(WalletAccount::verify_message(&public_key, &message, &signature)))
    }

    fn sign_message(&self, message: &[u8], wallet: &WalletAccount) -> JsonValue {
        let (public_key, signature) = wallet.sign_message(&message);
        let public_key = Serialize::serialize_to_vec(&public_key);
        let signature = Serialize::serialize_to_vec(&signature);
        object!{"publicKey" => hex::encode(public_key), "signature" => hex::encode(signature)}
    }
}

/*
TODO Support prefix
impl WalletHandler {
    const PREFIX: &'static str = "personal_";
}
*/

impl Module for WalletHandler {
    rpc_module_methods! {
        // Wallet
        "importRawKey" => import_raw_key,
        "listAccounts" => list_accounts,
        "lockAccount" => lock_account,
        "newAccount" => new_account,
        "unlockAccount" => unlock_account,
//        "sendTransaction" => send_transaction,
        "sign" => sign,
        "verifySignature" => verify_signature,
    }
}
