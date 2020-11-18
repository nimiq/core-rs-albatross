use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_database::Environment;
use nimiq_keys::{Address, KeyPair, PublicKey, PrivateKey, Signature};
use nimiq_wallet::{WalletAccount, WalletStore};
use nimiq_utils::otp::Locked;

use crate::{
    Error,
    wallets::UnlockedWallets,
};

fn message_from_maybe_hex(s: String, is_hex: bool) -> Result<Vec<u8>, Error> {
    if is_hex {
        Ok(hex::decode(&s)?)
    }
    else {
        Ok(s.into_bytes())
    }
}


#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnSignature {
    public_key: PublicKey,
    signature: Signature,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnAccount {
    address: Address,
    public_key: PublicKey,
    private_key: PrivateKey,
}


#[async_trait]
pub trait WalletInterface {
    async fn import_raw_key(&self, key_data: String, passphrase: Option<String>) -> Result<Address, Error>;

    async fn list_accounts(&self) -> Result<Vec<Address>, Error>;

    async fn lock_account(&self, address: Address) -> Result<(), Error>;

    async fn create_account(&self, passphrase: Option<String>) -> Result<ReturnAccount, Error>;

    async fn unlock_account(&self, address: Address, passphrase: Option<String>, duration: Option<u64>) -> Result<(), Error>;

    async fn sign(&self, message: String, address: Address, passphrase: Option<String>, is_hex: bool) -> Result<ReturnSignature, Error>;

    async fn verify_signature(&self, message: String, public_key: PublicKey, signature: Signature, is_hex: bool) -> Result<bool, Error>;
}

pub struct WalletDispatcher {
    wallet_store: WalletStore,
    unlocked_wallets: Arc<RwLock<UnlockedWallets>>,
}

impl WalletDispatcher {
    pub fn new(env: Environment) -> Self {
        Self {
            wallet_store: WalletStore::new(env),
            unlocked_wallets: Arc::new(RwLock::new(UnlockedWallets::default())),
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all="mixedCase")]
#[async_trait]
impl WalletInterface for WalletDispatcher {
    async fn import_raw_key(&self, key_data: String, passphrase: Option<String>) -> Result<Address, Error> {
        let passphrase = passphrase.unwrap_or_default();

        let private_key: PrivateKey = Deserialize::deserialize_from_vec(&hex::decode(&key_data)?)?;

        let wallet_account = WalletAccount::from(KeyPair::from(private_key));

        let address = wallet_account.address.clone();

        let wallet_account = Locked::with_defaults(wallet_account, passphrase.as_bytes())?;

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&address, &wallet_account, &mut txn);
        txn.commit();

        Ok(address)
    }

    async fn list_accounts(&self) -> Result<Vec<Address>, Error> {
        Ok(self.wallet_store.list(None))
    }

    async fn lock_account(&self, address: Address) -> Result<(), Error> {
        self.unlocked_wallets.write().remove(&address);
        Ok(())
    }

    async fn create_account(&self, passphrase: Option<String>) -> Result<ReturnAccount, Error> {
        let passphrase = passphrase.unwrap_or_default();
        let account = WalletAccount::generate();
        let address = account.address.clone();
        let locked_account = Locked::with_defaults(account.clone(), passphrase.as_bytes())?;

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&address, &locked_account, &mut txn);
        txn.commit();

        Ok(ReturnAccount {
            address,
            public_key: account.key_pair.public,
            private_key: account.key_pair.private,
        })
    }

    /// # TODO
    ///
    ///  - The duration parameter is ignored.
    ///
    async fn unlock_account(&self, address: Address, passphrase: Option<String>, _duration: Option<u64>) -> Result<(), Error> {
        let passphrase = passphrase.unwrap_or_default();
        let account = self
            .wallet_store
            .get(&address, None)
            .ok_or_else(|| Error::AccountNotFound(address))?;

        let unlocked_account = account.unlock(passphrase.as_bytes())
            .map_err(|_locked| Error::WrongPassphrase)?;

        self.unlocked_wallets.write().insert(unlocked_account);

        Ok(())
    }

    async fn sign(&self, message: String, address: Address, passphrase: Option<String>, is_hex: bool) -> Result<ReturnSignature, Error> {
        let message = message_from_maybe_hex(message, is_hex)?;

        let passphrase = passphrase.unwrap_or_default();

        let wallet_account: WalletAccount;
        let unlocked_wallets = self.unlocked_wallets.read();

        let wallet = if let Some(wallet) = unlocked_wallets.get(&address) {
            wallet
        }
        else {
            wallet_account = self.wallet_store.get(&address, None)
                .ok_or_else(|| Error::AccountNotFound(address))?
                .unlock(passphrase.as_bytes())
                .map_err(|_locked| Error::WrongPassphrase)?
                .key_pair
                .clone()
                .into();
            &wallet_account
        };

        let (public_key, signature) = wallet.sign_message(&message);

        Ok(ReturnSignature {
            public_key,
            signature
        })
    }

    async fn verify_signature(&self, message: String, public_key: PublicKey, signature: Signature, is_hex: bool) -> Result<bool, Error> {
        let message = message_from_maybe_hex(message, is_hex)?;
        Ok(WalletAccount::verify_message(&public_key, &message, &signature))
    }
}
