use structopt::StructOpt;
use anyhow::{Error, bail};
use futures::stream::StreamExt;

use nimiq_jsonrpc_core::Credentials;
use nimiq_rpc_client::Client;
use nimiq_rpc_interface::{
    types::{BlockNumberOrHash, OrLatest},
    blockchain::BlockchainInterface,
    consensus::ConsensusInterface,
    wallet::WalletInterface,
};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_rpc_interface::types::TransactionParameters;
use nimiq_transaction::TransactionFlags;
use nimiq_account::AccountType;


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
    /// Query a block from the blockchain.
    Block {
        /// Either a block hash or number. If omitted, the last block is queried.
        hash_or_number: Option<BlockNumberOrHash>,

        /// Include transactions
        #[structopt(short = "t")]
        include_transactions: bool,
    },

    /// Lists the current stakes from the staking contract.
    Stakes {},

    /// Follow the head of the blockchain.
    Follow {
        /// Show the full block instead of only the hash.
        #[structopt(short)]
        block: bool,
    },

    /// Show wallet accounts and their balances.
    Account(AccountCommand),

    /// Create, sign and send transactions.
    #[structopt(name = "tx")]
    Transaction(TransactionCommand),
}

#[derive(Debug, StructOpt)]
enum AccountCommand {
    List {
        #[structopt(short, long)]
        short: bool,
    },
    New {
        #[structopt(short = "P", long)]
        password: Option<String>,
    },
    Import {
        #[structopt(short = "P", long)]
        password: Option<String>,

        key_data: String,
    },
    Lock {
        address: Address,
    },
    Unlock {
        #[structopt(short = "P", long)]
        password: Option<String>,

        address: Address,
    },
    /// Queries the account state (e.g. account balance for basic accounts).
    Get {
        address: Address,
    },
}

#[derive(Debug, StructOpt)]
enum TransactionCommand {
    Send {
        from: Address,
        to: Address,
        value: Coin,
        fee: Coin,
    },
    Stake {

    },
    Unstake {

    },
}

impl Command {
    async fn run(self, mut client: Client) -> Result<(), Error> {
        match self {
            Command::Block { hash_or_number, include_transactions } => {
                let block = match hash_or_number {
                    Some(BlockNumberOrHash::Hash(hash)) => client.blockchain.block_by_hash(hash, include_transactions).await,
                    Some(BlockNumberOrHash::Number(number)) =>  client.blockchain.block_by_number(OrLatest::Value(number), include_transactions).await,
                    None => client.blockchain.block_by_number(OrLatest::Latest, include_transactions).await,
                }?;

                println!("{:#?}", block)
            },

            Command::Stakes {} => {
                let stakes = client.blockchain.list_stakes().await?;
                println!("{:#?}", stakes);
            },

            Command::Follow { block: show_block} => {
                let mut stream = client.blockchain.head_subscribe().await?;

                while let Some(block_hash) = stream.next().await {
                    if show_block {
                        let block = client.blockchain.block_by_hash(block_hash, false).await;
                        println!("{:#?}", block);
                    }
                    else {
                        println!("{}", block_hash);
                    }
                }
            },

            Command::Account(command) => {
                match command {
                    AccountCommand::List { short } => {
                        let accounts = client.wallet.list_accounts().await?;
                        for address in &accounts {
                            if short {
                                println!("{}", address.to_user_friendly_address());
                            }
                            else {
                                let account = client.blockchain.get_account(address.clone()).await?;
                                println!("{}: {:#?}", address.to_user_friendly_address(), account);
                            }
                        }
                    },

                    AccountCommand::New { password } => {
                        let account = client.wallet.create_account(password).await?;
                        println!("{:#?}", account);
                    },

                    AccountCommand::Import { password, key_data, } => {
                        let address = client.wallet.import_raw_key(key_data, password).await?;
                        println!("{}", address);
                    },

                    AccountCommand::Lock { address } => {
                        client.wallet.lock_account(address).await?;
                    },

                    AccountCommand::Unlock { address, password, .. } => {
                        // TODO: Duration
                        client.wallet.unlock_account(address, password, None).await?;
                    },

                    AccountCommand::Get { address } => {
                        let account = client.blockchain.get_account(address).await?;
                        println!("{:#?}", account);
                    },

                }
            },

            Command::Transaction(command) => {
                match command {
                    TransactionCommand::Send { from, to, value, fee } => {
                        let txid = client.consensus.send_transaction(TransactionParameters {
                            from,
                            from_type: AccountType::Basic,
                            to: Some(to),
                            to_type: AccountType::Basic,
                            value,
                            fee,
                            flags: TransactionFlags::empty(),
                            data: vec![],
                            validity_start_height: None
                        }).await?;
                        println!("{}", txid);
                    },
                    TransactionCommand::Stake {} => todo!(),
                    TransactionCommand::Unstake {} => todo!(),
                }
            }
        }

        Ok(())
    }
}


async fn run_app(opt: Opt) -> Result<(), Error> {
    let url = opt.url.as_deref().unwrap_or("ws://127.0.0.1:8648/ws")
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
