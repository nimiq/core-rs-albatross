use std::sync::Arc;

use async_trait::async_trait;
use nimiq_database::traits::WriteTransaction;
use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature, KeyPair, PrivateKey};
use nimiq_rpc_interface::{
    types::{RPCResult, ReturnAccount, ReturnSignature},
    wallet::WalletInterface,
};
use nimiq_serde::Deserialize;
use nimiq_utils::otp::Locked;
use nimiq_wallet::{WalletAccount, WalletStore};
use parking_lot::RwLock;

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
    ) -> RPCResult<Address, (), Self::Error> {
        let passphrase = passphrase.unwrap_or_default();

        let private_key = PrivateKey::deserialize_from_vec(&hex::decode(key_data)?)?;

        let wallet_account = WalletAccount::from(KeyPair::from(private_key));

        let address = wallet_account.address.clone();

        let wallet_account = Locked::with_defaults(wallet_account, passphrase.as_bytes())?;

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.put(&address, &wallet_account, &mut txn);
        txn.commit();

        Ok(address.into())
    }

    async fn is_account_imported(&mut self, address: Address) -> RPCResult<bool, (), Self::Error> {
        let is_imported = self.wallet_store.get(&address, None).is_some();

        Ok(is_imported.into())
    }

    async fn list_accounts(&mut self) -> RPCResult<Vec<Address>, (), Self::Error> {
        Ok(self.wallet_store.list(None).into())
    }

    async fn lock_account(&mut self, address: Address) -> RPCResult<(), (), Self::Error> {
        self.unlocked_wallets.write().remove(&address);
        Ok(().into())
    }

    async fn create_account(
        &mut self,
        passphrase: Option<String>,
    ) -> RPCResult<ReturnAccount, (), Self::Error> {
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
        }
        .into())
    }

    // # TODO The duration parameter is ignored.
    async fn unlock_account(
        &mut self,
        address: Address,
        passphrase: Option<String>,
        _duration: Option<u64>,
    ) -> RPCResult<bool, (), Self::Error> {
        let passphrase = passphrase.unwrap_or_default();
        let account = self
            .wallet_store
            .get(&address, None)
            .ok_or(Error::AccountNotFound(address))?;

        let unlocked_account = account
            .unlock(passphrase.as_bytes())
            .map_err(|_locked| Error::WrongPassphrase)?;

        self.unlocked_wallets.write().insert(unlocked_account);

        Ok(true.into())
    }

    async fn is_account_unlocked(&mut self, address: Address) -> RPCResult<bool, (), Self::Error> {
        let unlocked_wallets = self.unlocked_wallets.read();
        let is_unlocked = unlocked_wallets.get(&address).is_some();

        Ok(is_unlocked.into())
    }

    async fn remove_account(&mut self, address: Address) -> RPCResult<bool, (), Self::Error> {
        let _account = self
            .wallet_store
            .get(&address, None)
            .ok_or(Error::AccountNotFound(address.clone()))?;

        let mut txn = self.wallet_store.create_write_transaction();
        self.wallet_store.remove(&address, &mut txn);
        txn.commit();

        Ok(true.into())
    }

    async fn sign(
        &mut self,
        message: String,
        address: Address,
        passphrase: Option<String>,
        is_hex: bool,
    ) -> RPCResult<ReturnSignature, (), Self::Error> {
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
        }
        .into())
    }

    async fn verify_signature(
        &mut self,
        message: String,
        public_key: Ed25519PublicKey,
        signature: Ed25519Signature,
        is_hex: bool,
    ) -> RPCResult<bool, (), Self::Error> {
        let message = message_from_maybe_hex(message, is_hex)?;
        Ok(WalletAccount::verify_message(&public_key, &message, &signature).into())
    }
}
