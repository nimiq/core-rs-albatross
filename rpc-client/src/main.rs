use anyhow::{bail, Error};
use clap::Parser;
use nimiq_jsonrpc_client::Credentials;
use nimiq_rpc_client::Client;

pub mod subcommands;

use crate::subcommands::*;

#[derive(Debug, Parser)]
#[command(name = "nimiq-rpc", version = nimiq_utils::CARGO_VERSION)]
struct Opt {
    #[clap(short)]
    url: Option<String>,

    #[clap(short = 'U')]
    username: Option<String>,

    #[clap(short = 'P')]
    password: Option<String>,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Parser)]
enum Command {
    /// Shows policy information.
    #[clap(flatten)]
    Policy(PolicyCommand),

    /// Show and subscribe to the blockchain's current state.
    #[clap(flatten)]
    Blockchain(BlockchainCommand),

    /// Show wallet accounts and their balances.
    #[clap(flatten)]
    Account(AccountCommand),

    /// Create, sign and send transactions.
    #[clap(name = "tx", flatten)]
    Transaction(TransactionCommand),

    /// Shows local mempool information and push transactions to the mempool.
    #[clap(flatten)]
    Mempool(MempoolCommand),

    /// Shows local network peer information.
    #[clap(flatten)]
    Network(NetworkCommand),

    /// Shows and modifies validator information.
    /// Create, signs and send transactions referring to the local validator.
    /// These operations only work if the current client is configured as a validator.
    #[clap(flatten)]
    Validator(ValidatorCommand),

    /// Shows the zkp information.
    #[clap(flatten)]
    Zkp(ZKPComponentCommand),
}

impl Command {
    async fn run(self, client: Client) -> Result<Client, Error> {
        match self {
            Command::Policy(command) => command.handle_subcommand(client).await,
            Command::Blockchain(command) => command.handle_subcommand(client).await,
            Command::Account(command) => command.handle_subcommand(client).await,
            Command::Transaction(command) => command.handle_subcommand(client).await,
            Command::Network(command) => command.handle_subcommand(client).await,
            Command::Mempool(command) => command.handle_subcommand(client).await,
            Command::Validator(command) => command.handle_subcommand(client).await,
            Command::Zkp(command) => command.handle_subcommand(client).await,
        }
    }
}

async fn run_app(opt: Opt) -> Result<(), Error> {
    let url = opt
        .url
        .as_deref()
        .unwrap_or("ws://127.0.0.1:8648/ws")
        .parse()?;

    let credentials = match (&opt.username, &opt.password) {
        (Some(username), Some(password)) => Some(Credentials::new(username, password)),
        (None, None) => None,
        _ => bail!("Both username and password needs to be specified."),
    };

    let client = Client::new(url, credentials).await?;

    let mut client = opt.command.run(client).await?;
    client.close().await;
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = dotenvy::dotenv()
        && !e.not_found()
    {
        panic!("could not read .env file: {e}");
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    if let Err(e) = run_app(Opt::parse()).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
