use crate::{AsDatabaseBytes, FromDatabaseValue};

pub(crate) trait RawReadCursor {
    fn first<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn first_duplicate<V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<V>
    where
        V: FromDatabaseValue;

    fn last<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn last_duplicate<V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<V>
    where
        V: FromDatabaseValue;

    fn seek_key_value<K, V>(&mut self, key: &K, value: &V) -> bool
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    fn seek_key_nearest_value<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor, key: &K, value: &V) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + FromDatabaseValue;

    fn get_current<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next_duplicate<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next_no_duplicate<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev_duplicate<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev_no_duplicate<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn seek_key<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue;

    fn seek_key_both<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;

    fn seek_range_key<K, V>(&mut self, accessor: &lmdb_zero::ConstAccessor, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;

    fn count_duplicates(&mut self) -> usize;
}

pub trait ReadCursor {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue;

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue;

    fn seek_key_value<K, V>(&mut self, key: &K, value: &V) -> bool
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized;

    fn seek_key_nearest_value<K, V>(&mut self, key: &K, value: &V) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + FromDatabaseValue;

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue;

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue;

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue;

    fn count_duplicates(&mut self) -> usize;
}

macro_rules! impl_read_cursor_from_raw {
    ($t: ty, $raw: ident, $txn: ident) => {
        impl<'txn, 'db> ReadCursor for $t {
            fn first<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.first(&access)
            }

            fn first_duplicate<V>(&mut self) -> Option<V>
            where
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.first_duplicate(&access)
            }

            fn last<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.last(&access)
            }

            fn last_duplicate<V>(&mut self) -> Option<V>
            where
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.last_duplicate(&access)
            }

            fn seek_key_value<K, V>(&mut self, key: &K, value: &V) -> bool
            where
                K: AsDatabaseBytes + ?Sized,
                V: AsDatabaseBytes + ?Sized,
            {
                self.$raw.seek_key_value(key, value)
            }

            fn seek_key_nearest_value<K, V>(&mut self, key: &K, value: &V) -> Option<V>
            where
                K: AsDatabaseBytes + ?Sized,
                V: AsDatabaseBytes + FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.seek_key_nearest_value(&access, key, value)
            }

            fn get_current<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.get_current(&access)
            }

            fn next<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.next(&access)
            }

            fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.next_duplicate(&access)
            }

            fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.next_no_duplicate(&access)
            }

            fn prev<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.prev(&access)
            }

            fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.prev_duplicate(&access)
            }

            fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.prev_no_duplicate(&access)
            }

            fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
            where
                K: AsDatabaseBytes + ?Sized,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.seek_key(&access, key)
            }

            fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
            where
                K: AsDatabaseBytes + FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.seek_key_both(&access, key)
            }

            fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
            where
                K: AsDatabaseBytes + FromDatabaseValue,
                V: FromDatabaseValue,
            {
                let access = self.$txn.access();
                self.$raw.seek_range_key(&access, key)
            }

            fn count_duplicates(&mut self) -> usize {
                self.$raw.count_duplicates()
            }
        }
    };
}

pub trait WriteCursor: ReadCursor {
    fn remove(&mut self);
}
