use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
    DatabaseProxy, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_keys::Address;
use nimiq_utils::otp::Locked;

use crate::wallet_account::WalletAccount;

#[derive(Debug)]
pub struct WalletStore {
    env: DatabaseProxy,
    wallet_db: TableProxy,
}

impl WalletStore {
    const WALLET_DB_NAME: &'static str = "Wallet";

    pub fn new(env: DatabaseProxy) -> Self {
        let wallet_db = env.open_table(Self::WALLET_DB_NAME.to_string());
        WalletStore { env, wallet_db }
    }

    pub fn create_read_transaction(&self) -> TransactionProxy {
        self.env.read_transaction()
    }

    pub fn create_write_transaction(&self) -> WriteTransactionProxy {
        self.env.write_transaction()
    }

    pub fn list(&self, txn_option: Option<&TransactionProxy>) -> Vec<Address> {
        let read_txn;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut wallets = Vec::new();
        let mut cursor = txn.cursor(&self.wallet_db);
        let mut wallet: Option<(Address, Locked<WalletAccount>)> = cursor.first();

        while let Some((address, _)) = wallet {
            wallets.push(address);
            wallet = cursor.next();
        }

        wallets
    }

    pub fn get(
        &self,
        address: &Address,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<Locked<WalletAccount>> {
        match txn_option {
            Some(txn) => txn.get(&self.wallet_db, address),
            None => self.env.read_transaction().get(&self.wallet_db, address),
        }
    }

    pub fn put(
        &self,
        address: &Address,
        wallet: &Locked<WalletAccount>,
        txn: &mut WriteTransactionProxy,
    ) {
        txn.put_reserve(&self.wallet_db, address, wallet);
    }
}
