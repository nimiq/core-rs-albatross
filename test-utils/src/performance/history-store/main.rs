use std::{process::exit, time::Instant};

use clap::Parser;
use nimiq_blockchain::{interface::HistoryInterface, HistoryStore};
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
};
use nimiq_genesis::NetworkId;
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, HistoricTransactionData},
    ExecutedTransaction, Transaction as BlockchainTransaction,
};
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
}

fn create_transaction(block: u32, value: u64) -> HistoricTransaction {
    HistoricTransaction {
        network_id: NetworkId::UnitAlbatross,
        block_number: block,
        block_time: 0,
        data: HistoricTransactionData::Basic(ExecutedTransaction::Ok(
            BlockchainTransaction::new_basic(
                Address::from_user_friendly_address("NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF")
                    .unwrap(),
                Address::burn_address(),
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

fn history_store_performance(tpb: u32, number_of_batches: u32) {
    let tmp_dir = tempdir().expect("Could not create temporal directory");
    let tmp_dir = tmp_dir.path().to_str().unwrap();
    log::debug!("Creating a non volatile environment in {}", tmp_dir);
    let env = MdbxDatabase::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();
    let num_txns = tpb;

    let history_store = HistoryStore::new(env.clone(), NetworkId::UnitAlbatross);

    let number_of_blocks = Policy::blocks_per_batch() * number_of_batches;
    let mut txns_per_block = Vec::new();

    println!("Generating txns.. ");
    for block_number in 0..number_of_blocks {
        //println!("Generating {} txns for block {} ", num_txns, block_number);
        txns_per_block.push(gen_hist_txs_block(block_number, num_txns));
    }
    println!("Done generating txns. ");

    let mut txn = env.write_transaction();

    println!("Adding txns to history store");
    let start = Instant::now();

    for block_number in 0..number_of_blocks {
        history_store.add_to_history(
            &mut txn,
            block_number,
            &txns_per_block[block_number as usize],
        );
    }
    txn.commit();

    let duration = start.elapsed();

    println!(
        "It took: {:.2}s, to add {} batches, {} tpb to the history store, total_txns {}",
        duration.as_millis() as f64 / 1000_f64,
        number_of_batches,
        num_txns,
        num_txns * number_of_blocks,
    );
}

fn main() {
    let args = Args::parse();

    history_store_performance(args.tpb, args.batches);

    exit(0);
}
