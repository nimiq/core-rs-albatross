use database::{Database, DatabaseFlags, Environment, ReadTransaction, Transaction, WriteTransaction, FromDatabaseValue, IntoDatabaseValue};
use hash::Blake2bHash;
use primitives::block::Block;
use primitives::transaction::Transaction as NimiqTransaction;

use beserial::{Serialize, Deserialize};
use std::os::raw::c_uint;
use std::io;
use keys::Address;
use hash::Hash;

pub mod blockchain;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TransactionInfo {
    pub transaction_hash: Blake2bHash,
    pub block_hash: Blake2bHash,
    pub block_height: u32,
    pub index: u16,
}

impl FromDatabaseValue for TransactionInfo {
    fn copy_from_database(bytes: &[u8]) -> Result<Self, io::Error> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        return Ok(Deserialize::deserialize(&mut cursor)?);
    }
}

impl IntoDatabaseValue for TransactionInfo {
    fn database_byte_size(&self) -> usize {
        return self.serialized_size();
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl TransactionInfo {
    pub fn from_block(block: &Block) -> Vec<(&NimiqTransaction, TransactionInfo)> {
        let mut transactions = Vec::with_capacity(
            block.body.as_ref()
                .map(|body| body.transactions.len())
                .unwrap_or_default()
        );

        let block_hash: Blake2bHash = block.header.hash();

        if let Some(ref body) = block.body {
            for (index, tx) in body.transactions.iter().enumerate() {
                transactions.push((tx, TransactionInfo {
                    transaction_hash: tx.hash(),
                    block_hash: block_hash.clone(),
                    block_height: block.header.height,
                    index: index as u16,
                }));
            }
        }

        transactions
    }
}

#[derive(Debug)]
pub struct TransactionStore<'env> {
    env: &'env Environment,
    transaction_db: Database<'env>,
    sender_idx: Database<'env>,
    recipient_idx: Database<'env>,
    transaction_hash_idx: Database<'env>,
}

impl<'env> TransactionStore<'env> {
    const TRANSACTION_DB_NAME: &'static str = "TransactionData";
    const SENDER_IDX_NAME: &'static str = "SenderIdx";
    const RECIPIENT_IDX_NAME: &'static str = "RecipientIdx";
    const TRANSACTION_HASH_IDX_NAME: &'static str = "TransactionHashIdx";
    const HEAD_KEY: c_uint = 0;
    const HEAD_DEFAULT: c_uint = 1;

    pub fn new(env: &'env Environment) -> Self {
        let transaction_db = env.open_database_with_flags(
            Self::TRANSACTION_DB_NAME.to_string(),
            DatabaseFlags::UINT_KEYS
        );
        let sender_idx = env.open_database_with_flags(
            Self::SENDER_IDX_NAME.to_string(),
            DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_FIXED_SIZE_VALUES | DatabaseFlags::DUP_UINT_VALUES
        );
        let recipient_idx = env.open_database_with_flags(
            Self::RECIPIENT_IDX_NAME.to_string(),
            DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_FIXED_SIZE_VALUES | DatabaseFlags::DUP_UINT_VALUES
        );
        let transaction_hash_idx = env.open_database(
            Self::TRANSACTION_HASH_IDX_NAME.to_string()
        );
        return TransactionStore { env, transaction_db, sender_idx, recipient_idx, transaction_hash_idx };
    }

    fn get_head(&self, txn_option: Option<&Transaction>) -> c_uint {
        match txn_option {
            Some(txn) => txn.get(&self.transaction_db, &TransactionStore::HEAD_KEY),
            None => ReadTransaction::new(self.env).get(&self.transaction_db, &TransactionStore::HEAD_KEY)
        }.unwrap_or(Self::HEAD_DEFAULT)
    }

    fn set_head(&self, txn: &mut WriteTransaction, id: c_uint) {
        txn.put(&self.transaction_db, &TransactionStore::HEAD_KEY, &id);
    }

    fn get_id(&self, transaction_hash: &Blake2bHash, txn_option: Option<&Transaction>) -> Option<c_uint> {
        match txn_option {
            Some(txn) => txn.get(&self.transaction_hash_idx, transaction_hash),
            None => ReadTransaction::new(self.env).get(&self.transaction_hash_idx, transaction_hash)
        }
    }

    pub fn get_by_hash(&self, transaction_hash: &Blake2bHash, txn_option: Option<&Transaction>) -> Option<TransactionInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let index = self.get_id(transaction_hash, Some(txn))?;
        txn.get(&self.transaction_db, &index)
    }

    fn get_by_address(&self, database: &Database<'env>, address: &Address, limit: usize, txn: &Transaction) -> Vec<TransactionInfo> {
        let mut transactions = Vec::new();

        // Shortcut for a 0 limit.
        if limit == 0 {
            return transactions;
        }

        // Start collecting transactions.
        let mut cursor = txn.cursor(database);

        // Address not found.
        // Move to last transaction of that address.
        if cursor.seek_key::<Address, c_uint>(address).is_none() {
            return transactions;
        }

        let mut id: Option<c_uint> = cursor.last_duplicate();
        while let Some(index) = id {
            let info = txn.get(&self.transaction_db, &index)
                .expect("Corrupted store: TransactionInfo referenced from index not found");
            transactions.push(info);

            // Stop if we have enough transactions.
            if transactions.len() >= limit {
                break;
            }

            id = cursor.prev_duplicate().map(|(_, value): (Address, c_uint)| value);
        }

        transactions
    }

    pub fn get_by_sender(&self, sender: &Address, limit: usize, txn_option: Option<&Transaction>) -> Vec<TransactionInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        self.get_by_address(&self.sender_idx, sender, limit, txn)
    }

    pub fn get_by_recipient(&self, recipient: &Address, limit: usize, txn_option: Option<&Transaction>) -> Vec<TransactionInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        self.get_by_address(&self.recipient_idx, recipient, limit, txn)
    }

    pub fn put(&self, block: &Block, txn: &mut WriteTransaction<'env>) {
        // Insert all transactions.
        let transactions = TransactionInfo::from_block(block);
        let mut current_id = self.get_head(Some(txn));
        for (tx, info) in transactions.iter() {
            txn.put_reserve(&self.transaction_db, &current_id, info);
            txn.put(&self.transaction_hash_idx, &info.transaction_hash, &current_id);
            txn.put(&self.sender_idx, &tx.sender, &current_id);
            txn.put(&self.recipient_idx, &tx.recipient, &current_id);
            current_id += 1;
        }
        self.set_head(txn, current_id);
    }

    pub fn remove(&self, block: &Block, txn: &mut WriteTransaction<'env>) {
        if let Some(ref body) = block.body {
            // Remove all transactions.
            for tx in body.transactions.iter() {
                let hash = tx.hash();
                // Delete transaction from every store.
                if let Some(id) = self.get_id(&hash, Some(txn)) {
                    txn.remove(&self.transaction_hash_idx, &hash);
                    txn.remove(&self.transaction_db, &id);
                    txn.remove_item(&self.sender_idx, &tx.sender, &id);
                    txn.remove_item(&self.recipient_idx, &tx.recipient, &id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use database::volatile::VolatileEnvironment;

    #[test]
    fn it_can_store_the_head_id() {
        let env = VolatileEnvironment::new(4).unwrap();
        let store = TransactionStore::new(&env);
        assert_eq!(store.get_head(None), TransactionStore::HEAD_DEFAULT);

        let head = 5;
        let mut txn = WriteTransaction::new(&env);
        store.set_head(&mut txn, head);
        txn.commit();

        assert_eq!(store.get_head(None), head);
    }

    #[test]
    fn it_can_get_an_id() {
        let env = VolatileEnvironment::new(4).unwrap();
        let store = TransactionStore::new(&env);

        let hash = Blake2bHash::default();
        let id = 5;
        let mut txn = WriteTransaction::new(&env);
        txn.put(&store.transaction_hash_idx, &hash, &id);
        txn.commit();

        assert_eq!(store.get_id(&hash, None), Some(id));
    }

    #[test]
    fn it_can_get_by_address() {
        let env = VolatileEnvironment::new(4).unwrap();
        let store = TransactionStore::new(&env);

        let id1 = 5;
        let id2 = 8;
        let address = Address::default();
        let mut info = TransactionInfo {
            transaction_hash: Blake2bHash::default(),
            block_hash: Blake2bHash::default(),
            block_height: 1337,
            index: 12
        };

        {
            let mut txn = WriteTransaction::new(&env);
            // Insert tx 1.
            txn.put_reserve(&store.transaction_db, &id1, &info);
            txn.put(&store.sender_idx, &address, &id1);
            // Insert tx 2.
            info.index = 8;
            txn.put_reserve(&store.transaction_db, &id2, &info);
            txn.put(&store.sender_idx, &address, &id2);
            txn.commit();
        }

        let txn = ReadTransaction::new(&env);
        assert_eq!(store.get_by_address(&store.sender_idx, &address, 0, &txn).len(), 0);

        // 1 transaction.
        let txs = store.get_by_address(&store.sender_idx, &address, 1, &txn);
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].index, 8);

        // 2 transaction.
        let txs = store.get_by_address(&store.sender_idx, &address, 3, &txn);
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].index, 8);
        assert_eq!(txs[1].index, 12);
    }
}