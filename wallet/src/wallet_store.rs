use database::{Database, Environment, ReadTransaction, Transaction, WriteTransaction};
use database::cursor::ReadCursor;
use keys::Address;

use crate::wallet_account::WalletAccount;
use nimiq_utils::otp::Locked;

#[derive(Debug)]
pub struct WalletStore<'env> {
    env: &'env Environment,
    wallet_db: Database<'env>,
}

impl<'env> WalletStore<'env> {
    const WALLET_DB_NAME: &'static str = "Wallet";
    const DEFAULT_KEY: &'static str = "default";

    pub fn create_read_transaction(&self) -> ReadTransaction {
        ReadTransaction::new(self.env)
    }

    pub fn create_write_transaction(&self) -> WriteTransaction {
        WriteTransaction::new(self.env)
    }

    pub fn new(env: &'env Environment) -> Self {
        let wallet_db = env.open_database(Self::WALLET_DB_NAME.to_string());
        WalletStore { env, wallet_db }
    }

    pub fn list(&self, txn_option: Option<&Transaction>) -> Vec<Address> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let mut wallets= Vec::new();
        let mut cursor = txn.cursor(&self.wallet_db);
        let mut wallet: Option<(Address, Locked<WalletAccount>)> = cursor.first();

        while let Some((address, _)) = wallet {
            wallets.push(address);
            wallet = cursor.next();
        }

        wallets
    }

    pub fn get(&self, address: &Address, txn_option: Option<&Transaction>) -> Option<Locked<WalletAccount>> {
        match txn_option {
            Some(txn) => txn.get(&self.wallet_db, address),
            None => ReadTransaction::new(self.env).get(&self.wallet_db, address)
        }
    }

    pub fn put(&self, address: &Address, wallet: &Locked<WalletAccount>, txn: &mut WriteTransaction) {
        txn.put_reserve(&self.wallet_db, address, wallet);
    }
}
