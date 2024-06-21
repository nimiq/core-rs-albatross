use std::{fs::{self, create_dir}, path::{Path, PathBuf}, process::exit, time::Instant};

use clap::Parser;
// use nimiq_blockchain::{
//     interface::HistoryInterface, light_history_store::LightHistoryStore, HistoryStore,
// };
use nimiq_database::sqlite::{SqliteDatabase, SqliteTable};
use nimiq_genesis::NetworkId;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, HistoricTransactionData},
    ExecutedTransaction, Transaction as BlockchainTransaction,
};
use rand::{thread_rng, Rng};
use tempfile::tempdir;

/// Command line arguments for the binary
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of batches to add
    #[arg(short, long)]
    batches: u32,
    /// Transactions per block
    #[arg(short, long)]
    tpb: u32,
    /// Use the light history store
    #[arg(short, long, action = clap::ArgAction::Count)]
    light: u8,
    /// Rounds (Batches operations)
    #[arg(short, long)]
    rounds: Option<u32>,
    /// Loops: repeat the whole operation multiple times.
    #[arg(short, long)]
    loops: Option<u32>,
}

fn create_transaction(block: u32, value: u64) -> HistoricTransaction {
    let mut rng = thread_rng();
    HistoricTransaction {
        network_id: NetworkId::UnitAlbatross,
        block_number: block,
        block_time: 0,
        data: HistoricTransactionData::Basic(ExecutedTransaction::Ok(
            BlockchainTransaction::new_basic(
                Address(rng.gen()),
                Address(rng.gen()),
                Coin::from_u64_unchecked(value),
                Coin::from_u64_unchecked(0),
                0,
                NetworkId::Dummy,
            ),
        )),
    }
}

// Creates num_txns for the given block number, using consecutive values
fn gen_hist_txs_block(block_number: u32, num_txns: u32) -> Vec<HistoricTransaction> {
    let mut txns = Vec::new();

    for i in 0..num_txns {
        txns.push(create_transaction(block_number, i as u64));
    }
    txns
}

fn history_store_populate(
    db: &mut SqliteTable,
    tpb: u32,
    batches: u32,
    rounds: u32,
    loops: u32,
    db_file: PathBuf,
) {
    let num_txns = tpb;
    let loops = loops - 1;

    let number_of_blocks = Policy::blocks_per_batch() * batches;
    let loop_ofset = loops * number_of_blocks;
    let mut txns_per_block = Vec::new();

    println!(" Generating txns.. ");
    for block_number in 0..number_of_blocks {
        txns_per_block.push(gen_hist_txs_block(loop_ofset + 1 + block_number, num_txns));
    }
    println!(" Done generating txns. ");

    println!(" Adding txns to history store");
    let start = Instant::now();

    let batches_per_round = batches / rounds;
    let blocks_per_round = number_of_blocks / rounds;
    println!(
        " Number of rounds: {}, batches per round {} blocks per round {}",
        rounds, batches_per_round, blocks_per_round
    );

    for round in 0..rounds {
        let round_start = Instant::now();
        db.begin_transaction().unwrap(); // TODO: Handle error

        for block_number in 0..blocks_per_round {
            db.insert(&txns_per_block[((round * blocks_per_round) + block_number) as usize])
                .unwrap(); // TODO: Handle error
        }
        let commit_start = Instant::now();
        db.commit().unwrap(); // TODO: Handle error
        let round_duration = round_start.elapsed();
        let commit_duration = commit_start.elapsed();

        println!(
            "...{:.2}s to process round {}, {:.2}s for commit",
            round_duration.as_millis() as f64 / 1000_f64,
            1 + round,
            commit_duration.as_millis() as f64 / 1000_f64,
        );
    }

    let duration = start.elapsed();

    println!(" Creating indices..");
    let indices_start = Instant::now();
    db.create_indexes().unwrap(); // TODO: Handle error
    let indices_duration = indices_start.elapsed();

    let db_file_size = fs::metadata(db_file.to_str().unwrap()).unwrap().len();

    println!("Done adding {} txns to history store.", db.count().unwrap()); // TODO: Handle error

    println!(
        " {:.2}s to add {} batches, {:.2}s to create indices, {} tpb, DB size: {:.2}Mb, rounds: {}, total_txns: {}",
        duration.as_millis() as f64 / 1000_f64,
        batches,
        indices_duration.as_millis() as f64 / 1000_f64,
        num_txns,
        db_file_size as f64 / 1000000_f64,
        rounds,
        num_txns * number_of_blocks
    );
}

// fn history_store_prune(db: &mut SqliteTable) {
//     db.begin_transaction().unwrap(); // TODO: Handle error
//     println!("Pruning the history store..");
//     let start = Instant::now();

//     let removed_rows = db.prune(Policy::epoch_at(1)).unwrap(); // TODO: Handle error
//     db.commit().unwrap(); // TODO: Handle error

//     let duration = start.elapsed();

//     println!(
//         "It took: {:.2}s, to prune the history store, removing {} rows.",
//         duration.as_millis() as f64 / 1000_f64,
//         removed_rows,
//     );
// }

fn main() {
    let args = Args::parse();

    let policy_config = Policy::default();

    let _ = Policy::get_or_init(policy_config);

    let temp_dir = Path::new("./data");
    create_dir(temp_dir).expect("Could not create ./data directory");
    let db_file = temp_dir.join("sqlite.db");
    println!("Creating a non volatile database at {:?}", db_file);
    let database = SqliteDatabase::open(db_file.clone()).unwrap();
    let mut table = database.hist_txs_table().unwrap();

    let rounds = args.rounds.unwrap_or(1);
    let loops = args.loops.unwrap_or(1);

    for loop_number in 1..(1 + loops) {
        println!("Current loop {}", loop_number);

        history_store_populate(
            &mut table,
            args.tpb,
            args.batches,
            rounds,
            loop_number,
            db_file.clone(),
        );
    }

    // history_store_prune(&mut table);

    // let _ = fs::remove_dir_all(temp_dir);

    exit(0);
}
