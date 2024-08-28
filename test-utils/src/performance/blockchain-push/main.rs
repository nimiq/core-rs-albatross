use std::{fs, process::exit, sync::Arc, time::Instant};

use clap::Parser;
use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_log::TargetsExt;
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::blockchain::{
    fill_micro_blocks_with_txns, sign_macro_block, signing_key, voting_key,
};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;
use tempfile::tempdir;
use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
};

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

fn main() {
    let args = Args::parse();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_test_writer())
        .with(
            Targets::new()
                .with_default(LevelFilter::INFO)
                .with_nimiq_targets(LevelFilter::INFO)
                .with_env(),
        )
        .init();

    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();

    // Run tests with different policy values:
    let policy_config = Policy {
        genesis_block_number: genesis_block.block_number(),
        ..Default::default()
    };

    let _ = Policy::get_or_init(policy_config);

    let time = Arc::new(OffsetTime::new());
    let temp_dir = tempdir().expect("Could not create temporal directory");
    let tmp_dir = temp_dir.path().to_str().unwrap();
    let db_file = temp_dir.path().join("mdbx.dat");
    log::info!("Creating a non volatile environment in {}", tmp_dir);
    let env = MdbxDatabase::new(tmp_dir, Default::default()).unwrap();

    let blockchain = Arc::new(RwLock::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));
    let producer = BlockProducer::new(signing_key(), voting_key());

    let batches = args.batches;
    let tpb = args.tpb;

    for batch in 0..batches {
        let batch_start = Instant::now();
        fill_micro_blocks_with_txns(&producer, &blockchain, tpb as usize, 0);

        let blockchain = blockchain.upgradable_read();

        let macro_block_proposal = producer
            .next_macro_block_proposal(
                &blockchain,
                blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
                0u32,
                vec![],
            )
            .unwrap();

        let block = sign_macro_block(
            &producer.voting_key,
            macro_block_proposal.header,
            macro_block_proposal.body,
        );

        assert_eq!(
            Blockchain::push(blockchain, Block::Macro(block)),
            Ok(PushResult::Extended)
        );

        let batch_duration = batch_start.elapsed();
        let db_file_size = fs::metadata(db_file.to_str().unwrap()).unwrap().len();

        log::info!(
            " ----- {:.2}s to process batch {}, DB size: {:.2}Mb -----",
            batch_duration.as_secs(),
            batch + 1,
            db_file_size as f64 / 1000000_f64,
        );
    }

    let _ = fs::remove_dir_all(temp_dir);

    exit(0);
}
