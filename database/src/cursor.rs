use crate::{AsDatabaseBytes, FromDatabaseValue};

pub(crate) trait RawReadCursor {
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
    ($t: ty, $raw: ident) => {
        impl<'txn, 'db> ReadCursor for $t {
            fn first<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.first()
            }

            fn first_duplicate<V>(&mut self) -> Option<V>
            where
                V: FromDatabaseValue,
            {
                self.$raw.first_duplicate()
            }

            fn last<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.last()
            }

            fn last_duplicate<V>(&mut self) -> Option<V>
            where
                V: FromDatabaseValue,
            {
                self.$raw.last_duplicate()
            }

            fn get_current<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.get_current()
            }

            fn next<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.next()
            }

            fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.next_duplicate()
            }

            fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.next_no_duplicate()
            }

            fn prev<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.prev()
            }

            fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.prev_duplicate()
            }

            fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
            where
                K: FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.prev_no_duplicate()
            }

            fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
            where
                K: AsDatabaseBytes + ?Sized,
                V: FromDatabaseValue,
            {
                self.$raw.seek_key(key)
            }

            fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
            where
                K: AsDatabaseBytes + FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.seek_key_both(key)
            }

            fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
            where
                K: AsDatabaseBytes + FromDatabaseValue,
                V: FromDatabaseValue,
            {
                self.$raw.seek_range_key(key)
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
