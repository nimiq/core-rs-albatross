use crate::{
    mdbx, traits::Database, volatile, TableProxy, TransactionProxy, WriteTransactionProxy,
};

#[derive(Clone, Debug)]
pub enum DatabaseProxy {
    Volatile(volatile::VolatileDatabase),
    Persistent(mdbx::MdbxDatabase),
}

impl Database for DatabaseProxy {
    type Table = TableProxy;

    type ReadTransaction<'db> = TransactionProxy<'db>
    where
        Self: 'db;

    type WriteTransaction<'db> = WriteTransactionProxy<'db>
    where
        Self: 'db;

    fn open_table(&self, name: String) -> Self::Table {
        match self {
            DatabaseProxy::Volatile(ref db) => db.open_table(name),
            DatabaseProxy::Persistent(ref db) => db.open_table(name),
        }
    }

    fn open_table_with_flags(&self, name: String, flags: crate::TableFlags) -> Self::Table {
        match self {
            DatabaseProxy::Volatile(ref db) => db.open_table_with_flags(name, flags),
            DatabaseProxy::Persistent(ref db) => db.open_table_with_flags(name, flags),
        }
    }

    fn read_transaction<'db>(&'db self) -> Self::ReadTransaction<'db> {
        match self {
            DatabaseProxy::Volatile(ref db) => {
                TransactionProxy::ReadTransaction(db.read_transaction())
            }
            DatabaseProxy::Persistent(ref db) => {
                TransactionProxy::ReadTransaction(db.read_transaction())
            }
        }
    }

    fn write_transaction<'db>(&'db self) -> Self::WriteTransaction<'db> {
        match self {
            DatabaseProxy::Volatile(ref db) => WriteTransactionProxy::new(db.write_transaction()),
            DatabaseProxy::Persistent(ref db) => WriteTransactionProxy::new(db.write_transaction()),
        }
    }
}
