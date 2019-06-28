use database::{Database, Environment, ReadTransaction, Transaction, WriteTransaction};
use crate::wallet_account::WalletAccount;
use keys::Address;
use database::cursor::ReadCursor;


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

    pub fn has_default(&self, txn_option: Option<&Transaction>) -> bool {
        self.get_default_address(txn_option).is_some()
    }

    pub fn get_default(&self, txn: &mut WriteTransaction) -> WalletAccount {
        if let Some(address) = self.get_default_address(Some(txn)) {
            self.get(&address, Some(txn)).expect("Default address was set, but now wallet was stored.")
        }
        else {
            let default_wallet = WalletAccount::generate();
            self.set_default_address(default_wallet.address.clone(), txn);
            self.put(&default_wallet, txn);
            default_wallet
        }
    }

    pub fn get_default_address(&self, txn_option: Option<&Transaction>) -> Option<Address> {
        match txn_option {
            Some(txn) => txn.get(&self.wallet_db, Self::DEFAULT_KEY),
            None => ReadTransaction::new(self.env).get(&self.wallet_db, Self::DEFAULT_KEY)
        }
    }

    pub fn set_default_address(&self, address: Address, txn: &mut WriteTransaction) {
        txn.put(&self.wallet_db, Self::DEFAULT_KEY, &address);
    }

    pub fn list(&self, txn_option: Option<&Transaction>) -> Vec<WalletAccount> {
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
        let mut wallet: Option<(Address, WalletAccount)> = cursor.first();

        while let Some((_, account)) = wallet {
            wallets.push(account);
            wallet = cursor.next();
        }

        wallets
    }

    pub fn get(&self, address: &Address, txn_option: Option<&Transaction>) -> Option<WalletAccount> {
        match txn_option {
            Some(txn) => txn.get(&self.wallet_db, address),
            None => ReadTransaction::new(self.env).get(&self.wallet_db, address)
        }
    }

    pub fn put(&self, wallet: &WalletAccount, txn: &mut WriteTransaction) {
        txn.put_reserve(&self.wallet_db, &wallet.address, wallet);
    }
}
