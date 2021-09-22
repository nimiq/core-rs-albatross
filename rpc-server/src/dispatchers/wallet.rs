use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey, Signature};
use nimiq_rpc_interface::wallet::{ReturnAccount, ReturnSignature, WalletInterface};
use nimiq_utils::otp::Locked;
use nimiq_wallet::{WalletAccount, WalletStore};

use crate::{error::Error, wallets::UnlockedWallets};

fn message_from_maybe_hex(s: String, is_hex: bool) -> Result<Vec<u8>, Error> {
    if is_hex {
        Ok(hex::decode(&s)?)
    } else {
        Ok(s.into_bytes())
    }
}

pub struct WalletDispatcher {
    wallet_store: Arc<WalletStore>,
    pub unlocked_wallets: Arc<RwLock<UnlockedWallets>>,
}

impl WalletDispatcher {
    pub fn new(wallet_store: Arc<WalletStore>) -> Self {
        Self {
            wallet_store,
            unlocked_wallets: Arc::new(RwLock::new(UnlockedWallets::default())),
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl WalletInterface for WalletDispatcher {
    type Error = Error;

    async fn import_raw_key(
        &mut self,
        key_data: String,
        passphrase: Option<String>,
    ) -> Result<Address, Error> {
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

    async fn is_account_imported(&mut self, address: Address) -> Result<bool, Error> {
        let is_imported = self.wallet_store.get(&address, None).is_some();

        Ok(is_imported)
    }

    async fn list_accounts(&mut self) -> Result<Vec<Address>, Error> {
        Ok(self.wallet_store.list(None))
    }

    async fn lock_account(&mut self, address: Address) -> Result<(), Error> {
        self.unlocked_wallets.write().remove(&address);
        Ok(())
    }

    async fn create_account(&mut self, passphrase: Option<String>) -> Result<ReturnAccount, Error> {
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
    async fn unlock_account(
        &mut self,
        address: Address,
        passphrase: Option<String>,
        _duration: Option<u64>,
    ) -> Result<(), Error> {
        let passphrase = passphrase.unwrap_or_default();
        let account = self
            .wallet_store
            .get(&address, None)
            .ok_or(Error::AccountNotFound(address))?;

        let unlocked_account = account
            .unlock(passphrase.as_bytes())
            .map_err(|_locked| Error::WrongPassphrase)?;

        self.unlocked_wallets.write().insert(unlocked_account);

        Ok(())
    }

    async fn is_account_unlocked(&mut self, address: Address) -> Result<bool, Error> {
        let unlocked_wallets = self.unlocked_wallets.read();
        let is_unlocked = unlocked_wallets.get(&address).is_some();

        Ok(is_unlocked)
    }

    async fn sign(
        &mut self,
        message: String,
        address: Address,
        passphrase: Option<String>,
        is_hex: bool,
    ) -> Result<ReturnSignature, Error> {
        let message = message_from_maybe_hex(message, is_hex)?;

        let passphrase = passphrase.unwrap_or_default();

        let wallet_account: WalletAccount;
        let unlocked_wallets = self.unlocked_wallets.read();

        let wallet = if let Some(wallet) = unlocked_wallets.get(&address) {
            wallet
        } else {
            wallet_account = self
                .wallet_store
                .get(&address, None)
                .ok_or(Error::AccountNotFound(address))?
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
            signature,
        })
    }

    async fn verify_signature(
        &mut self,
        message: String,
        public_key: PublicKey,
        signature: Signature,
        is_hex: bool,
    ) -> Result<bool, Error> {
        let message = message_from_maybe_hex(message, is_hex)?;
        Ok(WalletAccount::verify_message(
            &public_key,
            &message,
            &signature,
        ))
    }
}
