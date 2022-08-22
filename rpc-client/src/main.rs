use anyhow::{bail, Error};
use clap::Parser;
use url::Url;

use nimiq_jsonrpc_client::{websocket::WebsocketClient, ArcClient};
use nimiq_jsonrpc_core::Credentials;
use nimiq_rpc_interface::{
    blockchain::BlockchainProxy, consensus::ConsensusProxy, mempool::MempoolProxy,
    network::NetworkProxy, policy::PolicyProxy, validator::ValidatorProxy, wallet::WalletProxy,
};
pub mod subcommands;

use crate::subcommands::*;

#[derive(Debug, Parser)]
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

    /// Shows local mempool information and pushs tranactions to the mempool.
    #[clap(flatten)]
    Mempool(MempoolCommand),

    /// Shows local network peer information.
    #[clap(flatten)]
    Network(NetworkCommand),

    /// Shows and modifies validator information.
    /// Create, signs and send transactions reffering to the local validator.
    #[clap(flatten)]
    Validator(ValidatorCommand),
}

impl Command {
    async fn run(self, client: Client) -> Result<(), Error> {
        match self {
            Command::Policy(command) => command.handle_subcommand(client).await,
            Command::Blockchain(command) => command.handle_subcommand(client).await,
            Command::Account(command) => command.handle_subcommand(client).await,
            Command::Transaction(command) => command.handle_subcommand(client).await,
            Command::Network(command) => command.handle_subcommand(client).await,
            Command::Mempool(command) => command.handle_subcommand(client).await,
            Command::Validator(command) => command.handle_subcommand(client).await,
        }
    }
}

pub struct Client {
    pub policy: PolicyProxy<ArcClient<WebsocketClient>>,
    pub blockchain: BlockchainProxy<ArcClient<WebsocketClient>>,
    pub consensus: ConsensusProxy<ArcClient<WebsocketClient>>,
    pub mempool: MempoolProxy<ArcClient<WebsocketClient>>,
    pub wallet: WalletProxy<ArcClient<WebsocketClient>>,
    pub validator: ValidatorProxy<ArcClient<WebsocketClient>>,
    pub network: NetworkProxy<ArcClient<WebsocketClient>>,
}

impl Client {
    pub async fn new(url: Url, credentials: Option<Credentials>) -> Result<Self, Error> {
        let client = ArcClient::new(WebsocketClient::new(url, credentials).await?);

        Ok(Self {
            policy: PolicyProxy::new(client.clone()),
            blockchain: BlockchainProxy::new(client.clone()),
            consensus: ConsensusProxy::new(client.clone()),
            mempool: MempoolProxy::new(client.clone()),
            wallet: WalletProxy::new(client.clone()),
            validator: ValidatorProxy::new(client.clone()),
            network: NetworkProxy::new(client),
        })
    }
}

async fn run_app(opt: Opt) -> Result<(), Error> {
    let url = opt
        .url
        .as_deref()
        .unwrap_or("ws://127.0.0.1:8649/ws")
        .parse()?;

    let credentials = match (&opt.username, &opt.password) {
        (Some(username), Some(password)) => Some(Credentials {
            username: username.to_string(),
            password: password.to_string(),
        }),
        (None, None) => None,
        _ => bail!("Both username and password needs to be specified."),
    };

    let client = Client::new(url, credentials).await?;

    opt.command.run(client).await?;

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = dotenv::dotenv() {
        if !e.not_found() {
            panic!("could not read .env file: {}", e);
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    if let Err(e) = run_app(Opt::parse()).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
