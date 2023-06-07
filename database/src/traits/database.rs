use super::{ReadTransaction, WriteTransaction};
use crate::TableFlags;

/// A database handle that can hold multiple tables.
pub trait Database: Sized {
    type Table;
    type ReadTransaction<'db>: ReadTransaction<'db, Table = Self::Table>
    where
        Self: 'db;
    type WriteTransaction<'db>: WriteTransaction<'db, Table = Self::Table>
    where
        Self: 'db;

    fn open_table(&self, name: String) -> Self::Table;
    fn open_table_with_flags(&self, name: String, flags: TableFlags) -> Self::Table;
    fn read_transaction(&self) -> Self::ReadTransaction<'_>;
    fn write_transaction(&self) -> Self::WriteTransaction<'_>;
    fn close(self) {}
}
