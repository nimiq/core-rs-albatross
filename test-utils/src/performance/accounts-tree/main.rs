use std::{fs, path::PathBuf, process::exit, time::Instant};

use clap::Parser;
use nimiq_account::{Accounts, BlockLogger, BlockState};
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
    DatabaseProxy,
};
use nimiq_genesis::NetworkId;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, Ed25519PublicKey, KeyPair, SecureGenerate};
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::{
    test_rng::test_rng,
    test_transaction::{generate_accounts, generate_transactions, TestAccount, TestTransaction},
};
use rand::{CryptoRng, Rng};
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
    /// Loops: repeat the whole operation multiple times.
    #[arg(short, long)]
    loops: Option<u32>,
}

fn init_accounts<R: Rng + CryptoRng>(
    genesis_builder: &mut GenesisBuilder,
    sender_balances: Vec<u64>,
    rng: &mut R,
) -> Vec<TestAccount> {
    generate_accounts(sender_balances, genesis_builder, true, rng)
}

fn accounts_tree_populate(
    env: &DatabaseProxy,
    tpb: u32,
    batches: u32,
    loops: u32,
    _db_file: PathBuf,
) {
    let num_txns = tpb;

    let number_of_blocks = Policy::blocks_per_batch() * batches;
    let total_txns = number_of_blocks * num_txns;

    let mut txns_per_block = Vec::new();

    let sender_balances = vec![100_000_000_000; 1];

    let mut genesis_builder = GenesisBuilder::default();
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    let rewards = vec![];

    // Generate and sign transaction from an address
    let mut rng = test_rng(true);

    let sender_accounts = init_accounts(&mut genesis_builder, sender_balances.clone(), &mut rng);

    // Add validator to genesis
    genesis_builder.with_genesis_validator(
        Address::from(&KeyPair::generate(&mut rng)),
        Ed25519PublicKey::from([0u8; 32]),
        BLSKeyPair::generate(&mut rng).public_key,
        Address::default(),
        None,
        None,
        false,
    );

    let genesis_info = genesis_builder.generate(env.clone()).unwrap();
    let length = genesis_info.accounts.len();
    let accounts = Accounts::new(env.clone());
    let mut txn = env.write_transaction();

    let start = Instant::now();
    accounts.init(&mut (&mut txn).into(), genesis_info.accounts);
    let duration = start.elapsed();
    println!(
        "Time elapsed after account init: {} ms, Accounts per second {}",
        duration.as_millis(),
        length as f64 / (duration.as_millis() as f64 / 1000_f64),
    );
    let start = Instant::now();
    txn.commit();
    let duration = start.elapsed();
    println!(
        "Time elapsed after account init's txn commit: {} ms, Accounts per second {}",
        duration.as_millis(),
        num_txns as f64 / (duration.as_millis() as f64 / 1000_f64),
    );

    println!("Done adding accounts to genesis");

    for loop_number in 1..loops + 1 {
        println!("Starting loop: {}", loop_number);
        println!("-----------------");
        println!("Generating transactions...");
        // Generate recipients accounts
        let recipient_balances = vec![0; total_txns as usize];
        let recipient_accounts =
            generate_accounts(recipient_balances, &mut genesis_builder, false, &mut rng);

        let mut txn_index = 0;

        for _block in 0..number_of_blocks {
            // Generate a new set of txns for each block
            let mut mempool_transactions = vec![];
            for _i in 0..num_txns {
                let mempool_transaction = TestTransaction {
                    fee: 0_u64,
                    value: 1,
                    recipient: recipient_accounts[txn_index as usize].clone(),
                    sender: sender_accounts[0].clone(),
                };
                mempool_transactions.push(mempool_transaction);
                txn_index += 1;
            }
            let (txns, _) = generate_transactions(mempool_transactions.clone(), false);
            txns_per_block.push(txns);
        }

        println!("Done generating {} transactions", txn_index);

        let mut block_index = 0;

        let loop_start = Instant::now();

        for batch in 0..batches {
            let mut txn = env.write_transaction();

            let batch_start = Instant::now();

            for _blocks in 0..Policy::blocks_per_batch() {
                //let block_start = Instant::now();

                let block_state = BlockState::new(block_index, 1);
                let result = accounts.commit(
                    &mut (&mut txn).into(),
                    &txns_per_block[block_index as usize],
                    &rewards[..],
                    &block_state,
                    &mut BlockLogger::empty(),
                );
                match result {
                    Ok(_) => assert!(true),
                    Err(err) => assert!(false, "Received {}", err),
                };

                //let block_duration = block_start.elapsed();
                //println!(
                //    "Processed block {}, duration {}ms",
                //    block_index,
                //    block_duration.as_millis()
                //);

                block_index += 1;
            }

            txn.commit();

            let batch_duration = batch_start.elapsed();

            println!(
                "---  Processed batch {}, duration {}ms  ---",
                batch,
                batch_duration.as_millis()
            );
        }

        let loop_duration = loop_start.elapsed();
        println!(
            "Processed loop {}, duration {:.2}s",
            loop_number,
            loop_duration.as_millis() as f64 / 1000_f64
        );
    }
}

fn main() {
    let args = Args::parse();

    let policy_config = Policy::default();

    let _ = Policy::get_or_init(policy_config);

    let temp_dir = tempdir().expect("Could not create temporal directory");
    let tmp_dir = temp_dir.path().to_str().unwrap();
    let db_file = temp_dir.path().join("mdbx.dat");
    log::debug!("Creating a non volatile environment in {}", tmp_dir);
    let env = MdbxDatabase::new(tmp_dir, 1024 * 1024 * 1024 * 1024, 21).unwrap();

    let loops = args.loops.unwrap_or(1);

    accounts_tree_populate(&env, args.tpb, args.batches, loops, db_file.clone());

    let _ = fs::remove_dir_all(temp_dir);

    exit(0);
}
