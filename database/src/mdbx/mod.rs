mod cursor;
mod database;
mod iterators;
mod transaction;

pub use self::{cursor::*, database::*, iterators::*, transaction::*};
use crate::traits::Database;

/// A helper trait that is implemented on `Option<&T>` with `T: AsRef<MdbxReadTransaction<'db>>`.
/// It allows to use the existing transaction or create a new one.
/// Example: `let txn = opt_txn.or_new(db);`
pub trait OptionalTransaction<'db, 'txn> {
    fn or_new<'inner>(self, db: &'inner MdbxDatabase) -> TransactionProxy<'db, 'txn>
    where
        'db: 'txn,
        'inner: 'db;
}

impl<'db, 'txn, T: AsRef<MdbxReadTransaction<'db>>> OptionalTransaction<'db, 'txn>
    for Option<&'txn T>
{
    fn or_new<'inner>(self, db: &'inner MdbxDatabase) -> TransactionProxy<'db, 'txn>
    where
        'db: 'txn,
        'inner: 'db,
    {
        match self {
            Some(txn) => TransactionProxy::Read(txn.as_ref()),
            None => TransactionProxy::OwnedRead(db.read_transaction()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::{
        declare_table,
        traits::{Database, DupReadCursor, ReadCursor, ReadTransaction, WriteTransaction},
    };

    declare_table!(TestTable, "test", String => String);
    declare_table!(DupTestTable, "dup_test", String => dup(u32));
    declare_table!(U32DupTable, "u32_dup", u32 => dup(u32));
    declare_table!(U32Table, "u32_nodup", u32 => u32);

    #[test]
    fn it_can_save_basic_objects() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(
                tempdir.path().join("test"),
                DatabaseConfig {
                    max_tables: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
            let table = TestTable {};
            db.create_regular_table(&table);

            // Read non-existent value.
            {
                let tx = db.read_transaction();
                assert!(tx.get(&table, &"test".to_string()).is_none());
            }

            // Read non-existent value.
            let mut tx = db.write_transaction();
            assert!(tx.get(&table, &"test".to_string()).is_none());

            // Write and read value.
            tx.put(&table, &"test".to_string(), &"one".to_string());
            assert_eq!(tx.get(&table, &"test".to_string()), Some("one".to_string()));
            // Overwrite and read value.
            tx.put(&table, &"test".to_string(), &"two".to_string());
            assert_eq!(tx.get(&table, &"test".to_string()), Some("two".to_string()));
            tx.commit();

            // Read value.
            let tx = db.read_transaction();
            assert_eq!(tx.get(&table, &"test".to_string()), Some("two".to_string()));
            tx.close();

            // Remove value.
            let mut tx = db.write_transaction();
            tx.remove(&table, &"test".to_string());
            assert!(tx.get(&table, &"test".to_string()).is_none());
            tx.commit();

            // Check removal.
            {
                let tx = db.read_transaction();
                assert!(tx.get(&table, &"test".to_string()).is_none());
            }

            // Write and abort.
            let mut tx = db.write_transaction();
            tx.put(&table, &"test".to_string(), &"one".to_string());
            tx.abort();

            // Check aborted transaction.
            let tx = db.read_transaction();
            assert!(tx.get(&table, &"test".to_string()).is_none());
        }
    }

    #[test]
    fn isolation_test() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(
                tempdir.path().join("test2"),
                DatabaseConfig {
                    max_tables: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
            let table = TestTable {};
            db.create_regular_table(&table);

            // Read non-existent value.
            let tx = db.read_transaction();
            assert!(tx.get(&table, &"test".to_string()).is_none());

            // WriteTransaction.
            let mut txw = db.write_transaction();
            assert!(txw.get(&table, &"test".to_string()).is_none());
            txw.put(&table, &"test".to_string(), &"one".to_string());
            assert_eq!(
                txw.get(&table, &"test".to_string()),
                Some("one".to_string())
            );

            // ReadTransaction should still have the old state.
            assert!(tx.get(&table, &"test".to_string()).is_none());

            // Commit WriteTransaction.
            txw.commit();

            // ReadTransaction should still have the old state.
            assert!(tx.get(&table, &"test".to_string()).is_none());

            // Have a new ReadTransaction read the new state.
            let tx2 = db.read_transaction();
            assert_eq!(
                tx2.get(&table, &"test".to_string()),
                Some("one".to_string())
            );
        }
        tempdir.close().unwrap();
    }

    #[test]
    fn duplicates_test() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(
                tempdir.path().join("test3"),
                DatabaseConfig {
                    max_tables: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
            let table = DupTestTable {};
            db.create_dup_table(&table);

            // Write one value.
            let mut txw = db.write_transaction();
            assert!(txw.get(&table, &"test".to_string()).is_none());
            txw.put(&table, &"test".to_string(), &125);
            assert_eq!(txw.get(&table, &"test".to_string()), Some(125));
            txw.commit();

            // Have a new ReadTransaction read the new state.
            {
                let tx = db.read_transaction();
                assert_eq!(tx.get(&table, &"test".to_string()), Some(125));
            }

            // Write a second smaller value.
            let mut txw = db.write_transaction();
            assert_eq!(txw.get(&table, &"test".to_string()), Some(125));
            txw.put(&table, &"test".to_string(), &12);
            assert_eq!(txw.get(&table, &"test".to_string()), Some(12));
            txw.commit();

            // Have a new ReadTransaction read the smaller value.
            {
                let tx = db.read_transaction();
                assert_eq!(tx.get(&table, &"test".to_string()), Some(12));
            }

            // Remove smaller value and write larger value.
            let mut txw = db.write_transaction();
            assert_eq!(txw.get(&table, &"test".to_string()), Some(12));
            txw.remove_item(&table, &"test".to_string(), &12);
            txw.put(&table, &"test".to_string(), &5783);
            assert_eq!(txw.get(&table, &"test".to_string()), Some(125));
            txw.commit();

            // Have a new ReadTransaction read the smaller value.
            {
                let tx = db.read_transaction();
                assert_eq!(tx.get(&table, &"test".to_string()), Some(125));
            }

            // Remove everything.
            let mut txw = db.write_transaction();
            assert_eq!(txw.get(&table, &"test".to_string()), Some(125));
            txw.remove(&table, &"test".to_string());
            assert!(txw.get(&table, &"test".to_string()).is_none());
            txw.commit();

            // Have a new ReadTransaction read the new state.
            {
                let tx = db.read_transaction();
                assert!(tx.get(&table, &"test".to_string()).is_none());
            }
        }
        tempdir.close().unwrap();
    }

    #[test]
    fn cursor_test() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(
                tempdir.path().join("test4"),
                DatabaseConfig {
                    max_tables: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
            let table = DupTestTable {};
            db.create_dup_table(&table);

            let test1: String = "test1".to_string();
            let test2: String = "test2".to_string();

            // Write some values.
            let mut txw = db.write_transaction();
            assert!(txw.get(&table, &"test".to_string()).is_none());
            txw.put(&table, &"test1".to_string(), &125);
            txw.put(&table, &"test1".to_string(), &12);
            txw.put(&table, &"test1".to_string(), &5783);
            txw.put(&table, &"test2".to_string(), &5783);
            txw.commit();

            // Have a new ReadTransaction read the new state.
            let tx = db.read_transaction();
            let mut cursor = tx.dup_cursor(&table);
            assert_eq!(cursor.first(), Some((test1.clone(), 12)));
            assert_eq!(cursor.last(), Some((test2.clone(), 5783)));
            assert_eq!(cursor.prev(), Some((test1.clone(), 5783)));
            assert_eq!(cursor.first_duplicate(), Some(12));
            assert_eq!(cursor.next_duplicate(), Some((test1.clone(), 125)));
            assert_eq!(cursor.prev_duplicate(), Some((test1.clone(), 12)));
            assert_eq!(cursor.next_no_duplicate(), Some((test2.clone(), 5783)));
            assert!(cursor.set_key(&"test".to_string()).is_none());
            assert_eq!(cursor.set_key(&"test1".to_string()), Some(12));
            assert_eq!(cursor.count_duplicates(), 3);
            assert_eq!(cursor.last_duplicate(), Some(5783));

            assert_eq!(cursor.get_current(), Some((test1.clone(), 5783)));

            assert_eq!(cursor.get_current(), Some((test1, 5783)));
            assert!(cursor.prev_no_duplicate().is_none());
            assert_eq!(cursor.next(), Some((test2, 5783)));
        }
        tempdir.close().unwrap();
    }

    #[test]
    fn it_correctly_orders_u32() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(
                tempdir.path().join("test5"),
                DatabaseConfig {
                    max_tables: Some(2),
                    ..Default::default()
                },
            )
            .unwrap();
            let dup_table = U32DupTable {};
            let table = U32Table {};
            db.create_dup_table(&dup_table);
            db.create_regular_table(&table);

            // Write some values.
            let mut txw = db.write_transaction();

            txw.put(&table, &256, &2);
            txw.put(&table, &3, &2);

            txw.put(&dup_table, &256, &3);
            txw.put(&dup_table, &3, &3);
            txw.put(&dup_table, &256, &2);
            txw.put(&dup_table, &3, &2);
            txw.commit();

            // Have a new ReadTransaction read the new state.
            let tx = db.read_transaction();

            let mut cursor = tx.cursor(&table);
            assert_eq!(cursor.first(), Some((3, 2)));
            assert_eq!(cursor.last(), Some((256, 2)));

            let mut cursor = tx.dup_cursor(&dup_table);
            assert_eq!(cursor.first(), Some((3, 2)));
            assert_eq!(cursor.last(), Some((256, 3)));
            assert_eq!(cursor.prev(), Some((256, 2)));
            assert_eq!(cursor.prev(), Some((3, 3)));
            assert_eq!(cursor.first_duplicate(), Some(2));
            assert_eq!(cursor.last_duplicate(), Some(3));
            assert_eq!(cursor.next_duplicate(), None);
            assert_eq!(cursor.next_no_duplicate(), Some((256, 2)));
        }
        tempdir.close().unwrap();
    }
}
