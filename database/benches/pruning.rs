use std::ops::Range;

use criterion::{
    black_box, criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use nimiq_database::{
    mdbx::{DatabaseConfig, MdbxDatabase},
    traits::{Database, DupTable, ReadCursor, RegularTable, Table, WriteCursor, WriteTransaction},
};
use pprof::criterion::{Output, PProfProfiler};

const TABLE: &'static str = "bench";

criterion_group! {
    name = pruning_benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = pruning
}
criterion_main!(pruning_benches);

struct DbTable;
impl Table for DbTable {
    type Key = u32;
    type Value = u32;

    const NAME: &'static str = TABLE;
}
impl RegularTable for DbTable {}
impl DupTable for DbTable {}

/// Benchmarks pruning via delete, cursor and dup tables.
pub fn pruning(c: &mut Criterion) {
    let mut group = c.benchmark_group("Pruning");

    group.sample_size(10);

    for size in [10_000, 100_000, 1_000_000] {
        measure_table_pruning(&mut group, size);
    }
}

fn measure_table_pruning(group: &mut BenchmarkGroup<'_, WallTime>, size: usize) {
    let scenarios: Vec<(fn(_, _) -> _, &str)> = vec![
        (delete, "delete"),
        (cursor, "cursor"),
        (delete, "delete_dup"),
    ];

    // `preload` is to be inserted into the database during the setup phase.
    for (scenario, scenario_str) in scenarios {
        let (preload, delete_range) = generate_batches(size, scenario_str.contains("dup"));

        // Setup phase before each benchmark iteration
        let setup = || {
            // Reset DB
            let db = MdbxDatabase::new_volatile(DatabaseConfig {
                max_tables: Some(2),
                ..Default::default()
            })
            .unwrap();
            let table = DbTable;
            if scenario_str.contains("dup") {
                db.create_dup_table(&table);
            } else {
                db.create_regular_table(&table);
            }

            let mut txn = db.write_transaction();

            for (key, value) in &preload {
                let _ = txn.put(&table, key, value);
            }

            txn.commit();

            (db, delete_range.clone())
        };

        // Iteration to be benchmarked
        let execution = |(db, delete_range)| scenario(db, delete_range);

        group.bench_function(
            format!("{} | {scenario_str} | preload: {} ", TABLE, preload.len()),
            |b| {
                b.iter_with_setup(setup, execution);
            },
        );
    }
}

/// Generate a mapping to delete a range from.
/// If `dup` is true, we create 10 keys with equally distributed duplicate keys.
fn generate_batches(size: usize, dup: bool) -> (Vec<(u32, u32)>, Range<u32>) {
    let mut input = Vec::with_capacity(size);
    let mut range = 0..size as u32;

    // For `dup`, we create 10 keys with equally distributed duplicate keys.
    if dup {
        for key in range.clone() {
            input.push((key % 10, key / 10));
        }
        let mid = (size as u32 % 10) / 2;
        range = mid..mid + 1;
    } else {
        // Generate a range of keys to delete.
        for key in range.clone() {
            input.push((key, key));
        }
        let mid = size as u32 / 2;
        let delete_size = size as u32 / 20;
        range = mid - delete_size..mid + delete_size;
    }

    (input, range)
}

fn delete(db: MdbxDatabase, input: Range<u32>) -> MdbxDatabase {
    {
        let table = DbTable;
        let mut txn = db.write_transaction();
        black_box({
            for key in input {
                txn.remove(&table, &key);
            }

            txn.commit();
        });
    }
    db
}

fn cursor(db: MdbxDatabase, input: Range<u32>) -> MdbxDatabase {
    {
        let table = DbTable;
        let txn = db.write_transaction();
        let mut cursor = txn.cursor(&table);
        black_box({
            let first_key = input.end - 1;
            cursor.set_key(&first_key).unwrap();
            for _ in input {
                cursor.remove();
                cursor.prev();
            }
            txn.commit();
        });
    }
    db
}
