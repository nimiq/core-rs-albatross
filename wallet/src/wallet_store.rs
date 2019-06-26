use database::{Database, DatabaseFlags, Environment, ReadTransaction, Transaction, WriteTransaction};
use crate::wallet::Wallet;


#[derive(Debug)]
pub struct WalletStore<'env> {
    env: &'env Environment,
    wallet_db: Database<'env>,
}

impl<'env> WalletStore<'env> {
    const WALLET_DB_NAME: &'static str = "Wallet";
    const DEFAULT_KEY: &'static str = "default";

    pub fn new(env: &'env Environment) -> Self {
        let wallet_db = env.open_database(Self::WALLET_DB_NAME.to_string());
        WalletStore { env, wallet_db }
    }

    pub fn has_default(&self, txn_option: Option<&Transaction>) -> bool {
        self.get_default_address(txn_option).is_some()
    }

    pub fn get_default(&self, txn: &mut WriteTransaction) -> Wallet {
        if let Some(address) = self.get_default_address(txn_option) {
            self.get(address, Some(txn)).expect("Default address was set, but now wallet was stored.")
        }
        else {
            let default_wallet = Wallet::generate();
            self.set_default_address(&default_wallet.address, txn);
            self.put(wallet, txn);
            wallet
        }
    }

    pub fn get_default_address(&self, txn_option: Option<&Transaction>) -> Option<Address> {
        match txn_option {
            Some(txn) => txn.get(&self.wallet_db, Self::DEFAULT_KEY),
            None => ReadTransaction::new(self.env).get(&self.chain_db, Self::DEFAULT_KEY)
        }
    }

    pub fn set_default_address(&self, address: Address, txn: &mut WriteTransaction) {
        txn.put(&self.wallet_db, Self::DEFAULT_KEY, address);
    }

    pub fn get(&self, address: Address, txn_option: Option<&Transaction>) -> Option<Wallet> {
        match txn_option {
            Some(txn) => txn.get(&self.wallet_db, address),
            None => ReadTransaction::new(self.env).get(&self.wallet_db, address)
        }
    }

    pub fn put(&self, wallet: Wallet, txn: &mut WriteTransaction) {
        let address = wallet.address.clone();
        txn.put(&self.wallet_db, address, wallet);
    }
}
