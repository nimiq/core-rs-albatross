use structopt::StructOpt;
use anyhow::{Error, bail};

use nimiq_jsonrpc_core::Credentials;
use nimiq_rpc_client::Client;
use nimiq_rpc_interface::{
    types::{BlockNumberOrHash, OrLatest},
    blockchain::BlockchainInterface,
};


#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(short)]
    url: Option<String>,

    #[structopt(short = "U")]
    username: Option<String>,

    #[structopt(short = "P")]
    password: Option<String>,

    #[structopt(subcommand)]
    command: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    GetBlock {
        hash_or_number: Option<BlockNumberOrHash>,

        #[structopt(short = "t")]
        include_transactions: bool,
    }
}

impl Command {
    async fn run(self, mut client: Client) -> Result<(), Error> {
        match self {
            Command::GetBlock { hash_or_number, include_transactions } => {
                let block = match hash_or_number {
                    Some(BlockNumberOrHash::Hash(hash)) => client.blockchain.block_by_hash(hash, include_transactions).await,
                    Some(BlockNumberOrHash::Number(number)) =>  client.blockchain.block_by_number(OrLatest::Value(number), include_transactions).await,
                    None => client.blockchain.block_by_number(OrLatest::Latest, include_transactions).await,
                }?;

                println!("{:#?}", block)
            }
        }

        Ok(())
    }
}


async fn run_app(opt: Opt) -> Result<(), Error> {
    let url = opt.url
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or_else(|| "ws://127.0.0.1:8648/ws")
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
    dotenv::dotenv().ok();
    pretty_env_logger::init();

    let opt = Opt::from_args();
    if let Err(e) = run_app(opt).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
