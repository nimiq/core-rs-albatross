use super::HistoryTreeHash;
use database::cursor::ReadCursor;
use database::{Database, Transaction, WriteTransaction};
use mmr::store::Store;
use std::cmp;
use std::convert::TryInto;

#[derive(Debug)]
enum Tx<'a, 'env> {
    Write(&'a mut WriteTransaction<'env>),
    Read(&'a Transaction<'env>),
}

/// A store implementation for MMRs based on a single database of LMDB.
/// The database contains multiple MMRs and one entry per node.
/// The values stored are `HistoryTreeHash`es and the keys are constructed as follows:
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
    last_index: usize,
}

impl<'a, 'env> MMRStore<'a, 'env> {
    // Create a read-only store.
    pub fn with_read_transaction(
        hist_tree_db: &'a Database,
        tx: &'a Transaction<'env>,
        epoch_number: u32,
    ) -> Self {
        let last_index = Self::get_last_index(hist_tree_db, tx, epoch_number).unwrap_or(0);
        MMRStore {
            hist_tree_db,
            tx: Tx::Read(tx),
            epoch_number,
            last_index,
        }
    }

    // Create a writable store.
    pub fn with_write_transaction(
        hist_tree_db: &'a Database,
        tx: &'a mut WriteTransaction<'env>,
        epoch_number: u32,
    ) -> Self {
        let last_index = Self::get_last_index(hist_tree_db, &tx, epoch_number).unwrap_or(0);
        MMRStore {
            hist_tree_db,
            tx: Tx::Write(tx),
            epoch_number,
            last_index,
        }
    }

    fn get_last_index(
        hist_tree_db: &Database,
        tx: &Transaction,
        epoch_number: u32,
    ) -> Option<usize> {
        let mut next_epoch = (epoch_number + 1).to_be_bytes().to_vec();
        next_epoch.extend_from_slice(&0usize.to_be_bytes());

        let mut cursor = tx.cursor(hist_tree_db);
        cursor.seek_range_key::<_, HistoryTreeHash>(&next_epoch);

        // Try inferring last index.
        let (last_key, _) = cursor.prev::<Vec<u8>, HistoryTreeHash>()?;
        let (epoch, index) = key_to_index(last_key)?;
        if epoch == epoch_number {
            Some(index)
        } else {
            None
        }
    }
}

fn index_to_key(epoch_number: u32, index: usize) -> Vec<u8> {
    let mut bytes = epoch_number.to_be_bytes().to_vec();
    bytes.extend_from_slice(&index.to_be_bytes());
    bytes
}

fn key_to_index(key: Vec<u8>) -> Option<(u32, usize)> {
    let (epoch_number, index) = key.split_at(4);
    let epoch_number = u32::from_be_bytes(epoch_number.try_into().ok()?);
    let index = usize::from_be_bytes(index.try_into().ok()?);
    Some((epoch_number, index))
}

impl<'a, 'env> Store<HistoryTreeHash> for MMRStore<'a, 'env> {
    fn push(&mut self, elem: HistoryTreeHash) {
        if let Tx::Write(ref mut tx) = self.tx {
            self.last_index += 1;
            let key = index_to_key(self.epoch_number, self.last_index);
            tx.put(&self.hist_tree_db, &key, &elem);
        }
    }

    fn remove_back(&mut self, num_elems: usize) {
        if let Tx::Write(ref mut tx) = self.tx {
            for _ in 0..cmp::min(num_elems, self.last_index + 1) {
                let key = index_to_key(self.epoch_number, self.last_index);
                tx.remove(&self.hist_tree_db, &key);
                self.last_index -= 1;
            }
        }
    }

    fn get(&self, pos: usize) -> Option<HistoryTreeHash> {
        let key = index_to_key(self.epoch_number, pos);
        match self.tx {
            Tx::Read(ref tx) => tx.get(&self.hist_tree_db, &key),
            Tx::Write(ref tx) => tx.get(&self.hist_tree_db, &key),
        }
    }

    fn len(&self) -> usize {
        self.last_index + 1
    }
}
