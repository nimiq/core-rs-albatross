use std::{process::exit, time::Instant};

use clap::Parser;
use nimiq_blockchain::{
    interface::HistoryInterface, light_history_store::LightHistoryStore, HistoryStore,
};
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
    DatabaseProxy,
};
use nimiq_genesis::NetworkId;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::{Policy, TEST_POLICY},
};
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
    /// Use the light history store
    #[arg(short, long, action = clap::ArgAction::Count)]
    light: u8,
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

fn history_store_performance(
    history_store: Box<dyn HistoryInterface + Sync + Send>,
    env: DatabaseProxy,
    tpb: u32,
    number_of_batches: u32,
) {
    let num_txns = tpb;

    let number_of_blocks = Policy::blocks_per_batch() * number_of_batches;
    let mut txns_per_block = Vec::new();

    println!("Generating txns.. ");
    for block_number in 0..number_of_blocks {
        txns_per_block.push(gen_hist_txs_block(1 + block_number, num_txns));
    }
    println!("Done generating txns. ");

    let mut txn = env.write_transaction();

    println!("Adding txns to history store");
    let start = Instant::now();

    for block_number in 0..number_of_blocks {
        history_store.add_to_history(
            &mut txn,
            1 + block_number,
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

    let mut txn = env.write_transaction();
    println!("Pruning the history store..");
    let start = Instant::now();

    history_store.remove_history(&mut txn, Policy::epoch_at(1));

    let duration = start.elapsed();

    println!(
        "It took: {:.2}s, to prune the history store",
        duration.as_millis() as f64 / 1000_f64,
    );
}

fn main() {
    let args = Args::parse();

    let policy_config = TEST_POLICY;

    let _ = Policy::get_or_init(policy_config);

    let tmp_dir = tempdir().expect("Could not create temporal directory");
    let tmp_dir = tmp_dir.path().to_str().unwrap();
    log::debug!("Creating a non volatile environment in {}", tmp_dir);
    let env = MdbxDatabase::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();

    let history_store = if args.light > 0 {
        println!("Exercising the light history store");
        Box::new(LightHistoryStore::new(
            env.clone(),
            NetworkId::UnitAlbatross,
        )) as Box<dyn HistoryInterface + Sync + Send>
    } else {
        println!("Exercising the history store");
        Box::new(HistoryStore::new(env.clone(), NetworkId::UnitAlbatross))
            as Box<dyn HistoryInterface + Sync + Send>
    };

    history_store_performance(history_store, env, args.tpb, args.batches);

    exit(0);
}
