use std::{cmp, fmt};

use nimiq_database::{
    traits::{ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
    TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::store::Store;

use super::utils::IndexedHash;

type WriteCursorProxy<'env> =
    <WriteTransactionProxy<'env> as WriteTransaction<'env>>::WriteCursor<'env>;

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
pub struct MMRStore<'a, 'env> {
    hist_tree_table: &'a TableProxy,
    tx: &'a TransactionProxy<'env>,
    cursor: Option<WriteCursorProxy<'env>>,
    epoch_number: u32,
    size: usize,
}

impl fmt::Debug for MMRStore<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MMRStore")
            .field("hist_tree_table", &self.hist_tree_table)
            .field("tx", &self.tx)
            .field("epoch_number", &self.epoch_number)
            .field("size", &self.size)
            .finish()
    }
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
            tx,
            epoch_number,
            size,
            cursor: None,
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
            tx,
            epoch_number,
            size,
            cursor: Some(WriteTransaction::cursor(tx, hist_tree_table)),
        }
    }
}

/// Calculates the size of MMR at a given epoch.
fn get_size(hist_tree_table: &TableProxy, tx: &TransactionProxy, epoch_number: u32) -> usize {
    // Calculate the key for the beginning of the next epoch, `epoch_number + 1 || 0`.
    let next_epoch = epoch_number + 1;

    // Initialize the cursor for the database.
    let mut cursor = tx.cursor(hist_tree_table);

    // Try to get the cursor on the key `epoch_number + 1`. If that key doesn't exist, then
    // the cursor will continue until it finds the next key, which we know will be of the form
    // `n`. If it reaches the end of the database without finding a key, then it will be on
    // a special key that indicates the end of the database.
    cursor.seek_range_key::<_, IndexedHash>(&next_epoch);

    // Move the cursor back until it finds a key. By definition that key will either be the
    // last index of some epoch or the beginning of the database.
    // If we reach the beginning of the file, then we know that the epoch is empty and we return
    // 0 as the size.
    let Some((epoch, last_value)) = cursor.prev_no_duplicate::<u32, IndexedHash>() else {
        return 0;
    };

    // If the epoch number we got is equal to the desired epoch number, then we know that we are
    // at the last node index of the epoch. The size then is simply the index + 1.
    // Otherwise, then we did not find any key for the epoch that we wanted, consequently the
    // epoch must be empty and we return a size of 0.
    if epoch == epoch_number {
        last_value.index as usize + 1
    } else {
        0
    }
}

impl<'a, 'env> Store<Blake2bHash> for MMRStore<'a, 'env> {
    fn push(&mut self, elem: Blake2bHash) {
        // This function assumes that there is no higher epoch.
        // Otherwise the append method will fail.
        if let Some(ref mut cursor) = self.cursor {
            let value = IndexedHash {
                index: self.size as u32,
                value: elem,
            };
            cursor.append_dup(&self.epoch_number, &value);
            self.size += 1;
        } else {
            panic!("Cannot push to a read-only store");
        }
    }

    fn remove_back(&mut self, num_elems: usize) {
        if self.size == 0 {
            return;
        }

        if let Some(ref mut cursor) = self.cursor {
            // Minimal seeking.
            // We cannot just remove from the back of the database,
            // because this might be in a previous epoch.
            let key = IndexedHash {
                index: self.size as u32 - 1,
                value: Blake2bHash::default(),
            };
            let (_, epoch, value) = cursor.seek_range_subkey(&self.epoch_number, &key).unwrap();
            assert_eq!(epoch, self.epoch_number, "Received invalid epoch number");
            assert_eq!(value.index, self.size as u32 - 1, "Received invalid index");

            for _ in 0..cmp::min(num_elems, self.size) {
                cursor.remove();
                cursor.prev_duplicate::<u32, IndexedHash>();
                self.size -= 1;
            }
        } else {
            panic!("Cannot push to a read-only store");
        }
    }

    fn get(&self, pos: usize) -> Option<Blake2bHash> {
        let mut cursor = self.tx.cursor(self.hist_tree_table);
        let key = IndexedHash {
            index: pos as u32,
            value: Blake2bHash::default(),
        };
        let (_, epoch, value) = cursor.seek_range_subkey(&self.epoch_number, &key)?;
        if epoch != self.epoch_number || value.index != pos as u32 {
            return None;
        }
        Some(value.value)
    }

    fn len(&self) -> usize {
        self.size
    }

    fn is_empty(&self) -> bool {
        self.size == 0
    }
}
