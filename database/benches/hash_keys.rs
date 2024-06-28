use std::{borrow::Cow, collections::HashSet, hash::Hash, marker::PhantomData};

use criterion::{
    black_box, criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use nimiq_database::{
    mdbx::{DatabaseConfig, MdbxDatabase},
    traits::{Database, Key, RegularTable, Table, WriteCursor, WriteTransaction},
};
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};
use pprof::criterion::{Output, PProfProfiler};
use rand::{
    distributions::{Distribution, Standard},
    thread_rng, Rng,
};

const TABLE: &'static str = "bench";

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = hash_keys
}
criterion_main!(benches);

struct DbTable<K: Key> {
    _key: PhantomData<K>,
}

impl<K: Key> DbTable<K> {
    fn new() -> Self {
        Self { _key: PhantomData }
    }
}

impl<K: Key> Table for DbTable<K> {
    type Key = K;
    type Value = Vec<u8>;

    const NAME: &'static str = TABLE;
}
impl<K: Key> RegularTable for DbTable<K> {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Blake2bHash([u8; 32]);
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Address([u8; 20]);

impl Distribution<Blake2bHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Blake2bHash {
        Blake2bHash(rng.gen())
    }
}

impl Distribution<Address> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Address {
        Address(rng.gen())
    }
}

impl AsDatabaseBytes for Blake2bHash {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self.0)
    }
}

impl AsDatabaseBytes for Address {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self.0)
    }
}

impl FromDatabaseBytes for Blake2bHash {
    fn from_key_bytes(bytes: &[u8]) -> Self {
        let mut hash = [0u8; 32];
        hash.copy_from_slice(bytes);
        Blake2bHash(hash)
    }
}

impl FromDatabaseBytes for Address {
    fn from_key_bytes(bytes: &[u8]) -> Self {
        let mut hash = [0u8; 20];
        hash.copy_from_slice(bytes);
        Address(hash)
    }
}

/// It benchmarks the insertion of rows into a table where `Keys` are hashes.
/// * `append`: Table is empty. Sorts during benchmark.
/// * `insert_sorted`: Table is preloaded with rows (same as batch size). Sorts during benchmark.
/// * `insert_unsorted`: Table is preloaded with rows (same as batch size).
/// * `put_sorted`: Table is preloaded with rows (same as batch size). Sorts during benchmark.
/// * `put_unsorted`: Table is preloaded with rows (same as batch size).
///
/// It does the above steps with different batches of rows. `10_000`, `100_000`, `1_000_000`. In the
/// end, the table statistics are shown (eg. number of pages, table size...)
pub fn hash_keys(c: &mut Criterion) {
    let mut group = c.benchmark_group("Hash-Keys Table Insertion");

    group.sample_size(10);

    for size in [10_000, 100_000, 1_000_000] {
        measure_table_insertion::<u32>(&mut group, size, "u32");
        measure_table_insertion::<Address>(&mut group, size, "Address");
        measure_table_insertion::<Blake2bHash>(&mut group, size, "Blake2bHash");
    }
}

fn measure_table_insertion<K: Eq + PartialEq + PartialOrd + Ord + Hash + Clone + Key>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    size: usize,
    ty: &'static str,
) where
    Standard: Distribution<K>,
{
    let scenarios: Vec<(fn(_, _) -> _, &str)> = vec![
        (append, "append_all"),
        (append, "append_input"),
        (insert, "insert_unsorted"),
        (insert, "insert_sorted"),
        (put, "put_unsorted"),
        (put, "put_sorted"),
    ];

    // `preload` is to be inserted into the database during the setup phase in all scenarios but
    // `append`.
    let (preload, unsorted_input) = generate_batches::<K>(size);

    for (scenario, scenario_str) in scenarios {
        // Append does not preload the table
        let mut preload_size = size;
        let mut input_size = size;
        if scenario_str.contains("append") {
            if scenario_str == "append_all" {
                input_size = size * 2;
            }
            preload_size = 0;
        }

        // Setup phase before each benchmark iteration
        let setup = || {
            // Reset DB
            let db = MdbxDatabase::new_volatile(DatabaseConfig {
                max_tables: Some(2),
                ..Default::default()
            })
            .unwrap();
            let table = DbTable::<K>::new();
            db.create_regular_table(&table);

            let mut unsorted_input = unsorted_input.clone();
            if scenario_str == "append_all" {
                unsorted_input.extend_from_slice(&preload);
            }

            if preload_size > 0 {
                let mut txn = db.write_transaction();

                for (key, value) in &preload {
                    let _ = txn.put(&table, key, value);
                }

                txn.commit();
            }

            (unsorted_input, db)
        };

        // Iteration to be benchmarked
        let execution = |(input, db)| {
            let mut input: Vec<(K, Vec<u8>)> = input;
            if scenario_str.contains("_sorted") || scenario_str.contains("append") {
                input.sort_by(|a, b| {
                    AsDatabaseBytes::as_key_bytes(&a.0).cmp(&AsDatabaseBytes::as_key_bytes(&b.0))
                });
            }
            scenario(db, input)
        };

        group.bench_function(
            format!(
                "{} |  {ty} | {scenario_str} | preload: {} | writing: {} ",
                TABLE, preload_size, input_size
            ),
            |b| {
                b.iter_with_setup(setup, execution);
            },
        );
    }
}

/// Generates two batches. The first is to be inserted into the database before running the
/// benchmark. The second is to be benchmarked with.
fn generate_batches<K: Eq + Hash + Clone>(size: usize) -> (Vec<(K, Vec<u8>)>, Vec<(K, Vec<u8>)>)
where
    Standard: Distribution<K>,
{
    let mut preload = Vec::with_capacity(size);
    let mut input = Vec::with_capacity(size);
    for _ in 0..size {
        let mut rng = thread_rng();
        let key1: K = rng.gen();
        let key2: K = rng.gen();

        let value1: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();
        let value2: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();

        preload.push((key1, value1));
        input.push((key2, value2));
    }

    let mut unique_keys = HashSet::new();
    preload.retain(|(k, _)| unique_keys.insert(k.clone()));
    input.retain(|(k, _)| unique_keys.insert(k.clone()));

    (preload, input)
}

fn append<K: Key>(db: MdbxDatabase, input: Vec<(K, Vec<u8>)>) -> MdbxDatabase {
    {
        let table = DbTable::<K>::new();
        let txn = db.write_transaction();
        let mut cursor = txn.cursor(&table);
        black_box({
            for (k, v) in input {
                cursor.append(&k, &v);
            }

            txn.commit();
        });
    }
    db
}

fn insert<K: Key>(db: MdbxDatabase, input: Vec<(K, Vec<u8>)>) -> MdbxDatabase {
    {
        let table = DbTable::<K>::new();
        let txn = db.write_transaction();
        let mut cursor = txn.cursor(&table);
        black_box({
            for (k, v) in input {
                cursor.put(&k, &v);
            }

            txn.commit();
        });
    }
    db
}

fn put<K: Key>(db: MdbxDatabase, input: Vec<(K, Vec<u8>)>) -> MdbxDatabase {
    {
        let table = DbTable::<K>::new();
        let mut txn = db.write_transaction();
        black_box({
            for (k, v) in input {
                txn.put(&table, &k, &v);
            }

            txn.commit();
        });
    }
    db
}
