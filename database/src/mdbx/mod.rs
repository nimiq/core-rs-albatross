mod cursor;
mod database;
mod iterators;
mod metrics_handler;
mod transaction;

pub use self::{cursor::*, database::*, iterators::*, metrics_handler::*, transaction::*};

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;
    use tempfile::tempdir;

    use super::*;
    use crate::{
        traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
        TableFlags,
    };

    #[test]
    fn it_can_save_basic_objects() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(tempdir.path().join("test"), 0, 1).unwrap();
            let table = db.open_table("test".to_string());

            // Read non-existent value.
            {
                let tx = db.read_transaction();
                assert!(tx.get::<str, String>(&table, "test").is_none());
            }

            // Read non-existent value.
            let mut tx = db.write_transaction();
            assert!(tx.get::<str, String>(&table, "test").is_none());

            // Write and read value.
            tx.put_reserve(&table, "test", "one");
            assert_eq!(
                tx.get::<str, String>(&table, "test"),
                Some("one".to_string())
            );
            // Overwrite and read value.
            tx.put_reserve(&table, "test", "two");
            assert_eq!(
                tx.get::<str, String>(&table, "test"),
                Some("two".to_string())
            );
            tx.commit();

            // Read value.
            let tx = db.read_transaction();
            assert_eq!(
                tx.get::<str, String>(&table, "test"),
                Some("two".to_string())
            );
            tx.close();

            // Remove value.
            let mut tx = db.write_transaction();
            tx.remove(&table, "test");
            assert!(tx.get::<str, String>(&table, "test").is_none());
            tx.commit();

            // Check removal.
            {
                let tx = db.read_transaction();
                assert!(tx.get::<str, String>(&table, "test").is_none());
            }

            // Write and abort.
            let mut tx = db.write_transaction();
            tx.put_reserve(&table, "test", "one");
            tx.abort();

            // Check aborted transaction.
            let tx = db.read_transaction();
            assert!(tx.get::<str, String>(&table, "test").is_none());
        }
    }

    #[test]
    fn isolation_test() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(tempdir.path().join("test2"), 0, 1).unwrap();
            let table = db.open_table("test".to_string());

            // Read non-existent value.
            let tx = db.read_transaction();
            assert!(tx.get::<str, String>(&table, "test").is_none());

            // WriteTransaction.
            let mut txw = db.write_transaction();
            assert!(txw.get::<str, String>(&table, "test").is_none());
            txw.put_reserve(&table, "test", "one");
            assert_eq!(
                txw.get::<str, String>(&table, "test"),
                Some("one".to_string())
            );

            // ReadTransaction should still have the old state.
            assert!(tx.get::<str, String>(&table, "test").is_none());

            // Commit WriteTransaction.
            txw.commit();

            // ReadTransaction should still have the old state.
            assert!(tx.get::<str, String>(&table, "test").is_none());

            // Have a new ReadTransaction read the new state.
            let tx2 = db.read_transaction();
            assert_eq!(
                tx2.get::<str, String>(&table, "test"),
                Some("one".to_string())
            );
        }
        tempdir.close().unwrap();
    }

    #[test]
    fn duplicates_test() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(tempdir.path().join("test3"), 0, 1).unwrap();
            let table = db.open_table_with_flags("test".to_string(), TableFlags::DUPLICATE_KEYS);

            // Write one value.
            let mut txw = db.write_transaction();
            assert!(txw.get::<str, u32>(&table, "test").is_none());
            txw.put::<str, u32>(&table, "test", &125);
            assert_eq!(txw.get::<str, u32>(&table, "test"), Some(125));
            txw.commit();

            // Have a new ReadTransaction read the new state.
            {
                let tx = db.read_transaction();
                assert_eq!(tx.get::<str, u32>(&table, "test"), Some(125));
            }

            // Write a second smaller value.
            let mut txw = db.write_transaction();
            assert_eq!(txw.get::<str, u32>(&table, "test"), Some(125));
            txw.put::<str, u32>(&table, "test", &12);
            assert_eq!(txw.get::<str, u32>(&table, "test"), Some(12));
            txw.commit();

            // Have a new ReadTransaction read the smaller value.
            {
                let tx = db.read_transaction();
                assert_eq!(tx.get::<str, u32>(&table, "test"), Some(12));
            }

            // Remove smaller value and write larger value.
            let mut txw = db.write_transaction();
            assert_eq!(txw.get::<str, u32>(&table, "test"), Some(12));
            txw.remove_item::<str, u32>(&table, "test", &12);
            txw.put::<str, u32>(&table, "test", &5783);
            assert_eq!(txw.get::<str, u32>(&table, "test"), Some(125));
            txw.commit();

            // Have a new ReadTransaction read the smaller value.
            {
                let tx = db.read_transaction();
                assert_eq!(tx.get::<str, u32>(&table, "test"), Some(125));
            }

            // Remove everything.
            let mut txw = db.write_transaction();
            assert_eq!(txw.get::<str, u32>(&table, "test"), Some(125));
            txw.remove::<str>(&table, "test");
            assert!(txw.get::<str, u32>(&table, "test").is_none());
            txw.commit();

            // Have a new ReadTransaction read the new state.
            {
                let tx = db.read_transaction();
                assert!(tx.get::<str, u32>(&table, "test").is_none());
            }
        }
        tempdir.close().unwrap();
    }

    #[test]
    fn cursor_test() {
        let tempdir = tempdir().unwrap();
        {
            let db = MdbxDatabase::new(tempdir.path().join("test4"), 0, 1).unwrap();
            let table = db.open_table_with_flags("test".to_string(), TableFlags::DUPLICATE_KEYS);

            let test1: String = "test1".to_string();
            let test2: String = "test2".to_string();

            // Write some values.
            let mut txw = db.write_transaction();
            assert!(txw.get::<str, u32>(&table, "test").is_none());
            txw.put::<str, u32>(&table, "test1", &125);
            txw.put::<str, u32>(&table, "test1", &12);
            txw.put::<str, u32>(&table, "test1", &5783);
            txw.put::<str, u32>(&table, "test2", &5783);
            txw.commit();

            // Have a new ReadTransaction read the new state.
            let tx = db.read_transaction();
            let mut cursor = tx.cursor(&table);
            assert_eq!(cursor.first::<String, u32>(), Some((test1.clone(), 12)));
            assert_eq!(cursor.last::<String, u32>(), Some((test2.clone(), 5783)));
            assert_eq!(cursor.prev::<String, u32>(), Some((test1.clone(), 5783)));
            assert_eq!(cursor.first_duplicate::<u32>(), Some(12));
            assert_eq!(
                cursor.next_duplicate::<String, u32>(),
                Some((test1.clone(), 125))
            );
            assert_eq!(
                cursor.prev_duplicate::<String, u32>(),
                Some((test1.clone(), 12))
            );
            assert_eq!(
                cursor.next_no_duplicate::<String, u32>(),
                Some((test2.clone(), 5783))
            );
            assert!(cursor.seek_key::<str, u32>("test").is_none());
            assert_eq!(cursor.seek_key::<str, u32>("test1"), Some(12));
            assert_eq!(cursor.count_duplicates(), 3);
            assert_eq!(cursor.last_duplicate::<u32>(), Some(5783));

            assert_eq!(
                cursor.get_current::<String, u32>(),
                Some((test1.clone(), 5783))
            );

            assert_eq!(cursor.get_current::<String, u32>(), Some((test1, 5783)));
            assert!(cursor.prev_no_duplicate::<String, u32>().is_none());
            assert_eq!(cursor.next::<String, u32>(), Some((test2, 5783)));
        }
        tempdir.close().unwrap();
    }
}
