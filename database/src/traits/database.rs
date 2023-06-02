use super::{ReadTransaction, WriteTransaction};
use crate::TableFlags;

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
    fn read_transaction<'db>(&'db self) -> Self::ReadTransaction<'db>;
    fn write_transaction<'db>(&'db self) -> Self::WriteTransaction<'db>;
    fn close(self) {}
}
