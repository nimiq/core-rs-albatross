use std::{cmp, convert::TryInto};

use nimiq_database::{
    traits::{ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
    TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::store::{LightStore, Store};

#[derive(Debug)]
enum Tx<'a, 'env> {
    Write(&'a mut WriteTransactionProxy<'env>),
    Read(&'a TransactionProxy<'env>),
}

/// A store implementation for MMRs based on a single database of LMDB.
/// The database contains multiple MMRs and one entry per node.
/// The values stored are `Blake2bHash`es and the keys are constructed as follows:
/// The big-endian byte representation of the epoch number concatenated with the big-endian byte
/// representation of the node index.
///
/// This way, we can efficiently retrieve individual nodes in an MMR and efficiently calculate
/// an MMRs size.
///
/// To calculate the size, we retrieve the last node index of an MMR.
/// To this end, we place a database cursor at the beginning of the next epoch `key = epoch + 1 || 0`
/// and move the cursor back by one entry (thus being the last node of the previous epoch, if the
/// epoch has any nodes).
#[derive(Debug)]
pub struct MMRStore<'a, 'env> {
    hist_tree_table: &'a TableProxy,
    tx: Tx<'a, 'env>,
    epoch_number: u32,
    size: usize,
}

impl<'a, 'env> MMRStore<'a, 'env> {
    /// Create a read-only store.
    pub fn with_read_transaction(
        hist_tree_table: &'a TableProxy,
        tx: &'a TransactionProxy<'env>,
        epoch_number: u32,
    ) -> Self {
        let size = get_size(hist_tree_table, tx, epoch_number);
        MMRStore {
            hist_tree_table,
            tx: Tx::Read(tx),
            epoch_number,
            size,
        }
    }

    /// Create a writable store.
    pub fn with_write_transaction(
        hist_tree_table: &'a TableProxy,
        tx: &'a mut WriteTransactionProxy<'env>,
        epoch_number: u32,
    ) -> Self {
        let size = get_size(hist_tree_table, tx, epoch_number);
        MMRStore {
            hist_tree_table,
            tx: Tx::Write(tx),
            epoch_number,
            size,
        }
    }
}

/// Calculates the size of MMR at a given epoch.
fn get_size(hist_tree_table: &TableProxy, tx: &TransactionProxy, epoch_number: u32) -> usize {
    // Calculate the key for the beginning of the next epoch, `epoch_number + 1 || 0`.
    let mut next_epoch = (epoch_number + 1).to_be_bytes().to_vec();
    next_epoch.extend_from_slice(&0usize.to_be_bytes());

    // Initialize the cursor for the database.
    let mut cursor = tx.cursor(hist_tree_table);

    // Try to get the cursor on the key `epoch_number + 1 || 0`. If that key doesn't exist, then
    // the cursor will continue until it finds the next key, which we know will be of the form
    // `n || 0`. If it reaches the end of the database without finding a key, then it will be on
    // a special key that indicates the end of the database.
    cursor.seek_range_key::<_, Blake2bHash>(&next_epoch);

    // Move the cursor back until it finds a key. By definition that key will either be the
    // last index of some epoch or the beginning of the database.
    // If we reach the beginning of the file, then we know that the epoch is empty and we return
    // 0 as the size.
    let (last_key, _) = match cursor.prev::<Vec<u8>, Blake2bHash>() {
        Some(v) => v,
        None => return 0,
    };

    // Deconstruct the key into an epoch number and a node index.
    let (epoch, index) = key_to_index(last_key).unwrap();

    // If the epoch number we got is equal to the desired epoch number, then we know that we are
    // at the last node index of the epoch. The size then is simply the index + 1.
    // Otherwise, then we did not find any key for the epoch that we wanted, consequently the
    // epoch must be empty and we return a size of 0.
    if epoch == epoch_number {
        index + 1
    } else {
        0
    }
}

/// Transforms an epoch number and a node index into the corresponding database key.
fn index_to_key(epoch_number: u32, index: usize) -> Vec<u8> {
    let mut bytes = epoch_number.to_be_bytes().to_vec();
    bytes.extend_from_slice(&index.to_be_bytes());
    bytes
}

/// Transforms a database key into the corresponding epoch number and node index. Returns None if it
/// fails.
fn key_to_index(key: Vec<u8>) -> Option<(u32, usize)> {
    let (epoch_number, index) = key.split_at(4);
    let epoch_number = u32::from_be_bytes(epoch_number.try_into().ok()?);
    let index = usize::from_be_bytes(index.try_into().ok()?);
    Some((epoch_number, index))
}

impl<'a, 'env> Store<Blake2bHash> for MMRStore<'a, 'env> {
    fn push(&mut self, elem: Blake2bHash) {
        if let Tx::Write(ref mut tx) = self.tx {
            let key = index_to_key(self.epoch_number, self.size);
            tx.put(self.hist_tree_table, &key, &elem);
            self.size += 1;
        }
    }

    fn remove_back(&mut self, num_elems: usize) {
        if let Tx::Write(ref mut tx) = self.tx {
            for _ in 0..cmp::min(num_elems, self.size) {
                let key = index_to_key(self.epoch_number, self.size - 1);
                tx.remove(self.hist_tree_table, &key);
                self.size -= 1;
            }
        }
    }

    fn get(&self, pos: usize) -> Option<Blake2bHash> {
        let key = index_to_key(self.epoch_number, pos);
        match self.tx {
            Tx::Read(tx) => tx.get(self.hist_tree_table, &key),
            Tx::Write(ref tx) => tx.get(self.hist_tree_table, &key),
        }
    }

    fn len(&self) -> usize {
        self.size
    }

    fn is_empty(&self) -> bool {
        self.size == 0
    }
}

/// A store implementation for LightMMRs based on a single database of LMDB.
/// The implementation is equivalent to the MMRStore, the only difference resides
/// in the insert and remove functions
#[derive(Debug)]
pub struct LightMMRStore<'a, 'env> {
    hist_tree_table: &'a TableProxy,
    tx: Tx<'a, 'env>,
    block_number: u32,
    size: usize,
}

impl<'a, 'env> LightMMRStore<'a, 'env> {
    /// Create a read-only store.
    pub fn with_read_transaction(
        hist_tree_table: &'a TableProxy,
        tx: &'a TransactionProxy<'env>,
        block_number: u32,
    ) -> Self {
        let size = get_size(hist_tree_table, tx, block_number);
        LightMMRStore {
            hist_tree_table,
            tx: Tx::Read(tx),
            block_number,
            size,
        }
    }

    /// Create a writable store.
    pub fn with_write_transaction(
        hist_tree_table: &'a TableProxy,
        tx: &'a mut WriteTransactionProxy<'env>,
        block_number: u32,
    ) -> Self {
        let size = get_size(hist_tree_table, tx, block_number);
        LightMMRStore {
            hist_tree_table,
            tx: Tx::Write(tx),
            block_number,
            size,
        }
    }
}

/// Removes all txns corresponding to the given block number
pub fn remove_block_from_store(
    hist_tree_table: &TableProxy,
    txn: &mut WriteTransactionProxy,
    block_number: u32,
) {
    // Calculate the key for the beginning of the block `block_number || 0`.
    let block_start = index_to_key(block_number, 0);

    // Initialize the cursor for the database.
    let mut cursor = WriteTransaction::cursor(txn, hist_tree_table);

    if cursor.seek_key::<_, Blake2bHash>(&block_start).is_none() {
        return;
    }

    let mut current_block_number = block_number;

    while current_block_number == block_number {
        cursor.remove();

        let (next_key, _) = match cursor.next::<Vec<u8>, Blake2bHash>() {
            Some(v) => v,
            None => return,
        };

        // Deconstruct the key into a block number and a node index.
        let (new_block_number, _) = key_to_index(next_key).unwrap();

        current_block_number = new_block_number;
    }
}

impl<'a, 'env> LightStore<Blake2bHash> for LightMMRStore<'a, 'env> {
    fn insert(&mut self, elem: Blake2bHash, pos: usize) {
        if let Tx::Write(ref mut tx) = self.tx {
            let key = index_to_key(self.block_number, pos);
            tx.put(self.hist_tree_table, &key, &elem);
        }
    }

    fn remove(&mut self, pos: usize) {
        if let Tx::Write(ref mut tx) = self.tx {
            let key = index_to_key(self.block_number, pos);
            tx.remove(self.hist_tree_table, &key);
        }
    }

    fn get(&self, pos: usize) -> Option<Blake2bHash> {
        let key = index_to_key(self.block_number, pos);
        match self.tx {
            Tx::Read(tx) => tx.get(self.hist_tree_table, &key),
            Tx::Write(ref tx) => tx.get(self.hist_tree_table, &key),
        }
    }

    fn len(&self) -> usize {
        self.size
    }

    fn is_empty(&self) -> bool {
        self.size == 0
    }
}
