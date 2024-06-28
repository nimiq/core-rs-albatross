use super::{DupTable, ReadTransaction, RegularTable, WriteTransaction};

/// A database handle that can hold multiple tables.
pub trait Database: Sized {
    type ReadTransaction<'db>: ReadTransaction<'db>
    where
        Self: 'db;
    type WriteTransaction<'db>: WriteTransaction<'db>
    where
        Self: 'db;

    /// Creates a regular table (no-duplicates).
    fn create_regular_table<T: RegularTable>(&self, table: &T);

    /// Creates a table that can store duplicate keys.
    fn create_dup_table<T: DupTable>(&self, table: &T);

    /// Creates a read transaction.
    fn read_transaction(&self) -> Self::ReadTransaction<'_>;

    /// Creates a read/write transaction.
    fn write_transaction(&self) -> Self::WriteTransaction<'_>;
}
