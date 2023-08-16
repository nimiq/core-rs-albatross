use std::{path::Path, time::Instant};

use clap::Parser;
use log::level_filters::LevelFilter;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_rpc::Client;
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt, Layer};
use url::Url;

use nimiq_history_migration::get_history_root;

/// Command line arguments for the binary
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// RPC connection URL to use
    #[arg(short, long)]
    rpc: String,

    /// TOML output file name
    #[arg(short, long)]
    db_path: String,

    /// Cutting block height to use
    #[arg(short, long)]
    height: u32,

    /// Cutting block hash to use
    #[arg(short, long)]
    hash: String,

    /// Set to true for testnet usage
    #[arg(short, long)]
    testnet: bool,
}

fn initialize_logging() {
    let filter = Targets::new().with_default(LevelFilter::DEBUG);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(true)
                .with_filter(filter),
        )
        .init();
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let url = match Url::parse(&args.rpc) {
        Ok(url) => url,
        Err(error) => {
            log::error!(?error, "Invalid RPC URL");
            std::process::exit(1);
        }
    };
    let client = Client::new(url);

    initialize_logging();

    // Get block according to arguments and check if it exists
    let block = client.get_block_by_hash(&args.hash, false).await.unwrap();
    if block.number != args.height {
        log::error!(
            height = args.height,
            hash = args.hash,
            "Could not find provided block"
        );
        std::process::exit(1);
    }

    // Create DB environment
    let network_id = if args.testnet { "test" } else { "main" };
    let db_name = format!("{network_id}-history-consensus").to_lowercase();
    let db_path = Path::new(&args.db_path).join(db_name);
    let env = match MdbxDatabase::new_with_max_readers(
        db_path.clone(),
        100 * 1024 * 1024 * 1024,
        20,
        600,
    ) {
        Ok(db) => db,
        Err(e) => {
            log::error!(error = ?e, "Failed to create database");
            std::process::exit(1);
        }
    };

    // Build history tree
    log::info!(?db_path, "Building history tree");
    let start = Instant::now();
    match get_history_root(&client, block.number, env).await {
        Ok(history_root) => {
            let duration = start.elapsed();
            log::info!(
                duration = humantime::format_duration(duration).to_string(),
                history_root = history_root.to_hex(),
                "Finished building history tree"
            )
        }
        Err(e) => {
            log::error!(error = ?e, "Failed to build history root");
            std::process::exit(1);
        }
    }
}
