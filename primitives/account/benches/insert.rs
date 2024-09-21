use std::collections::HashSet;

use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use nimiq_account::{Accounts, BlockLogger, BlockState};
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
};
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction::{inherent::Inherent, Transaction};
use nimiq_trie::WriteTransactionProxy;
use pprof::criterion::{Output, PProfProfiler};
use rand::{seq::SliceRandom, thread_rng, Rng};

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = insert_accounts
}
criterion_main!(benches);

/// It benchmarks the insertion of transactions.
/// We preload an accounts trie of significant size and measure the time it takes to insert transactions.
pub fn insert_accounts(c: &mut Criterion) {
    let mut group = c.benchmark_group("Transaction Insertion");

    group.sample_size(10);

    for preload_size in [10_000, 100_000, 1_000_000, 10_000_000] {
        for input_size in [100, 1_000] {
            measure_table_insertion(&mut group, preload_size, input_size);
        }
    }
}

fn measure_table_insertion(
    group: &mut BenchmarkGroup<'_, WallTime>,
    preload_size: usize,
    input_size: usize,
) {
    let populate_db = || {
        // `preload` is to be inserted into the database during the setup phase.
        let preload = generate_preload(preload_size);

        // Reset DB.
        let db = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let accounts = Accounts::new(db.clone());

        // Add all accounts as inherents.
        let mut txn = db.write_transaction();
        let mut trie_txn: WriteTransactionProxy = (&mut txn).into();
        accounts
            .commit(
                &mut trie_txn,
                &[],
                &preload,
                &BlockState::new(0, 0),
                &mut BlockLogger::empty(),
            )
            .unwrap();
        txn.commit();

        (preload, db)
    };

    let (preload, db) = populate_db();

    // Setup phase before each benchmark iteration.
    let setup = || {
        let input = generate_input(&preload, input_size);
        let accounts = Accounts::new(db.clone());

        (input, db.clone(), accounts)
    };

    // Iteration to be benchmarked
    let execution = |(input, db, accounts): (Vec<Transaction>, MdbxDatabase, Accounts)| {
        let mut txn = db.write_transaction();
        let mut trie_txn: WriteTransactionProxy = (&mut txn).into();
        accounts
            .commit(
                &mut trie_txn,
                &input,
                &[],
                &BlockState::new(0, 0),
                &mut BlockLogger::empty(),
            )
            .unwrap();
        txn.commit();
    };

    group.bench_function(
        format!("preload: {} | writing: {} ", preload_size, input_size),
        |b| {
            b.iter_with_setup(setup, execution);
        },
    );
}

/// Generates the initial trie.
fn generate_preload(preload_size: usize) -> Vec<Inherent> {
    let mut preload = Vec::with_capacity(preload_size);
    let mut rng = thread_rng();

    for _ in 0..preload_size {
        let address = Address(rng.gen());
        let inherent = Inherent::Reward {
            validator_address: address.clone(),
            target: address,
            value: Coin::from_u64_unchecked(rng.gen::<u64>() % Coin::MAX_SAFE_VALUE),
        };
        preload.push(inherent);
    }

    let mut unique_addresses = HashSet::new();
    preload.retain(|inherent| unique_addresses.insert(inherent.target().clone()));

    preload
}

/// Generate the transactions to be inserted.
fn generate_input(preload: &Vec<Inherent>, input_size: usize) -> Vec<Transaction> {
    let mut input = Vec::with_capacity(input_size);
    let mut rng = thread_rng();

    for _ in 0..input_size {
        // Create a transaction from a random account into a new account.
        let value = Coin::from_u64_unchecked(rng.gen::<u64>() % Coin::MAX_SAFE_VALUE);
        let sender = preload
            .choose(&mut rng)
            .map(|inherent| inherent.target().clone())
            .unwrap();
        let recipient = Address(rng.gen());
        let tx = Transaction::new_basic(
            sender,
            recipient,
            value,
            Coin::ZERO,
            0,
            NetworkId::UnitAlbatross,
        );
        input.push(tx);
    }

    input
}
