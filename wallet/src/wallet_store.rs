use nimiq_database::{
    declare_table,
    mdbx::{MdbxDatabase, MdbxReadTransaction, MdbxWriteTransaction, OptionalTransaction},
    traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
};
use nimiq_keys::Address;
use nimiq_utils::otp::Locked;

use crate::wallet_account::WalletAccount;

declare_table!(WalletTable, "Wallet", Address => Locked<WalletAccount>);

#[derive(Debug)]
pub struct WalletStore {
    env: MdbxDatabase,
    table: WalletTable,
}

impl WalletStore {
    pub fn new(env: MdbxDatabase) -> Self {
        let wallet_table = WalletTable;
        env.create_regular_table(&wallet_table);
        WalletStore {
            env,
            table: wallet_table,
        }
    }

    pub fn create_read_transaction(&self) -> MdbxReadTransaction {
        self.env.read_transaction()
    }

    pub fn create_write_transaction(&self) -> MdbxWriteTransaction {
        self.env.write_transaction()
    }

    pub fn list(&self, txn_option: Option<&MdbxReadTransaction>) -> Vec<Address> {
        let txn = txn_option.or_new(&self.env);

        let cursor = txn.cursor(&self.table);
        cursor
            .into_iter_start()
            .map(|(address, _)| address)
            .collect()
    }

    pub fn get(
        &self,
        address: &Address,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<Locked<WalletAccount>> {
        let txn = txn_option.or_new(&self.env);
        txn.get(&self.table, address)
    }

    pub fn put(
        &self,
        address: &Address,
        wallet: &Locked<WalletAccount>,
        txn: &mut MdbxWriteTransaction,
    ) {
        txn.put_reserve(&self.table, address, wallet);
    }
}
