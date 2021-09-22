use std::cmp;
use std::convert::TryInto;

use nimiq_database::cursor::ReadCursor;
use nimiq_database::{Database, Transaction, WriteTransaction};
use nimiq_hash::Blake2bHash;
use nimiq_mmr::store::Store;

#[derive(Debug)]
enum Tx<'a, 'env> {
    Write(&'a mut WriteTransaction<'env>),
    Read(&'a Transaction<'env>),
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
    hist_tree_db: &'a Database,
    tx: Tx<'a, 'env>,
    epoch_number: u32,
    size: usize,
}

impl<'a, 'env> MMRStore<'a, 'env> {
    /// Create a read-only store.
    pub fn with_read_transaction(
        hist_tree_db: &'a Database,
        tx: &'a Transaction<'env>,
        epoch_number: u32,
    ) -> Self {
        let size = Self::get_size(hist_tree_db, tx, epoch_number);
        MMRStore {
            hist_tree_db,
            tx: Tx::Read(tx),
            epoch_number,
            size,
        }
    }

    /// Create a writable store.
    pub fn with_write_transaction(
        hist_tree_db: &'a Database,
        tx: &'a mut WriteTransaction<'env>,
        epoch_number: u32,
    ) -> Self {
        let size = Self::get_size(hist_tree_db, tx, epoch_number);
        MMRStore {
            hist_tree_db,
            tx: Tx::Write(tx),
            epoch_number,
            size,
        }
    }

    /// Calculates the size of MMR at a given epoch.
    fn get_size(hist_tree_db: &Database, tx: &Transaction, epoch_number: u32) -> usize {
        // Calculate the key for the beginning of the next epoch, `epoch_number + 1 || 0`.
        let mut next_epoch = (epoch_number + 1).to_be_bytes().to_vec();
        next_epoch.extend_from_slice(&0usize.to_be_bytes());

        // Initialize the cursor for the database.
        let mut cursor = tx.cursor(hist_tree_db);

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
            tx.put(self.hist_tree_db, &key, &elem);
            self.size += 1;
        }
    }

    fn remove_back(&mut self, num_elems: usize) {
        if let Tx::Write(ref mut tx) = self.tx {
            for _ in 0..cmp::min(num_elems, self.size) {
                let key = index_to_key(self.epoch_number, self.size - 1);
                tx.remove(self.hist_tree_db, &key);
                self.size -= 1;
            }
        }
    }

    fn get(&self, pos: usize) -> Option<Blake2bHash> {
        let key = index_to_key(self.epoch_number, pos);
        match self.tx {
            Tx::Read(tx) => tx.get(self.hist_tree_db, &key),
            Tx::Write(ref tx) => tx.get(self.hist_tree_db, &key),
        }
    }

    fn len(&self) -> usize {
        self.size
    }

    fn is_empty(&self) -> bool {
        self.size == 0
    }
}
