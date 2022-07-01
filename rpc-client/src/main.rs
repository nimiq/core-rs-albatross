use anyhow::{bail, Error};
use clap::Parser;
use futures::StreamExt;
use url::Url;

use nimiq_jsonrpc_client::{websocket::WebsocketClient, ArcClient};
use nimiq_jsonrpc_core::Credentials;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_rpc_interface::{
    blockchain::{BlockchainInterface, BlockchainProxy},
    consensus::{ConsensusInterface, ConsensusProxy},
    mempool::MempoolProxy,
    types::{BlockNumberOrHash, LogType, ValidityStartHeight},
    wallet::{WalletInterface, WalletProxy},
};

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
    /// Query a block from the blockchain.
    Block {
        /// Either a block hash or number. If omitted, the last block is queried.
        hash_or_number: Option<BlockNumberOrHash>,

        /// Include transactions
        #[clap(short = 't')]
        include_transactions: bool,
    },

    /// Lists the current stakes from the staking contract.
    Stakes {},

    /// Follow the head of the blockchain.
    FollowHead {
        /// Show the full block instead of only the hash.
        #[clap(short)]
        block: bool,
    },
    /// Follow a validator state upon election blocks.
    FollowValidator { address: Address },

    /// Follow the logs associated with the specified addresses and of any of the log types given.
    /// If no addresses or no logtypes are provided it fetches all logs.
    FollowLogsOfAddressesAndTypes {
        #[clap(short = 'a', long, multiple_values = true)]
        addresses: Vec<Address>,

        /// Possible values are:
        #[clap(short = 'l', long, arg_enum, multiple_values = true)]
        log_types: Vec<LogType>,
    },

    /// Show wallet accounts and their balances.
    #[clap(flatten)]
    Account(AccountCommand),

    /// Create, sign and send transactions.
    #[clap(name = "tx", flatten)]
    Transaction(TransactionCommand),
}

#[derive(Debug, Parser)]
enum AccountCommand {
    List {
        #[clap(short, long)]
        short: bool,
    },

    New {
        #[clap(short = 'P', long)]
        password: Option<String>,
    },

    Import {
        #[clap(short = 'P', long)]
        password: Option<String>,

        key_data: String,
    },

    Lock {
        address: Address,
    },

    Unlock {
        #[clap(short = 'P', long)]
        password: Option<String>,

        address: Address,
    },
    /// Queries the account state (e.g. account balance for basic accounts).
    Get {
        address: Address,
    },
}

#[derive(Debug, Parser)]
enum TransactionCommand {
    /// Sends a simple transaction from the wallet `wallet` to a basic `recipient`.
    Basic {
        /// Transaction will be sent from this address. An wallet with this address must be unlocked.
        wallet: Address,

        /// Recipient for this transaction. This must be a basic account.
        recipient: Address,

        /// The amount of NIM to send to the recipient.
        value: Coin,

        /// The associated transaction fee to be payed by the sender. If absent it defaults to 0 NIM.
        #[clap(short, long, default_value = "0")]
        fee: Coin,

        /// The block height for the unstake to be applied. If absent it defaults to the current block height at time of processing.
        #[clap(short, long, default_value_t)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[clap(long = "dry")]
        dry: bool,
    },

    /// Sends a staking transaction from the address of a given `key_pair` to a given `staker_address`.
    Stake {
        /// The stake will be sent from this wallet.
        wallet: Address,

        /// Destination address for the stake.
        staker_address: Address,

        /// The amount of NIM to stake.
        value: Coin,

        /// The associated transaction fee to be payed by the sender wallet. If absent it defaults to 0 NIM.
        #[clap(short, long, default_value = "0")]
        fee: Coin,

        /// The block height from which on the unstake could be applied. The maximum amount of blocks the transaction is valid for
        /// is specified in `TRANSACTION_VALIDITY_WINDOW`.
        /// If absent it defaults to the current block height at time of processing.
        #[clap(short, long, default_value_t)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[clap(long = "dry")]
        dry: bool,
    },

    Unstake {
        /// The stake will be sent from this wallet.
        wallet: Address,

        /// The recipients of the previously staked coins.
        recipient: Address,

        /// The amount of NIM to unstake.
        value: Coin,

        /// The associated transaction fee to be payed from the funds being unstaked. If absent it defaults to 0 NIM.
        #[clap(short, long, default_value = "0")]
        fee: Coin,

        /// The block height from which on the unstake could be applied. The maximum amount of blocks the transaction is valid for is specified in `TRANSACTION_VALIDITY_WINDOW`.
        /// If absent it defaults to the current block height at time of processing.
        #[clap(short, long, default_value_t)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[clap(long = "dry")]
        dry: bool,
    },
}

impl Command {
    async fn run(self, mut client: Client) -> Result<(), Error> {
        match self {
            Command::Block {
                hash_or_number,
                include_transactions,
            } => {
                let block = match hash_or_number {
                    Some(BlockNumberOrHash::Hash(hash)) => {
                        client
                            .blockchain
                            .get_block_by_hash(hash, Some(include_transactions))
                            .await
                    }
                    Some(BlockNumberOrHash::Number(number)) => {
                        client
                            .blockchain
                            .get_block_by_number(number, Some(include_transactions))
                            .await
                    }
                    None => {
                        client
                            .blockchain
                            .get_latest_block(Some(include_transactions))
                            .await
                    }
                }?;

                println!("{:#?}", block)
            }

            Command::Stakes {} => {
                let stakes = client.blockchain.get_active_validators().await?;
                println!("{:#?}", stakes);
            }

            Command::FollowHead { block: show_block } => {
                if show_block {
                    let mut stream = client
                        .blockchain
                        .subscribe_for_head_block(Some(false))
                        .await?;

                    while let Some(block) = stream.next().await {
                        println!("{:#?}", block);
                    }
                } else {
                    let mut stream = client.blockchain.subscribe_for_head_block_hash().await?;

                    while let Some(block_hash) = stream.next().await {
                        println!("{}", block_hash);
                    }
                }
            }

            Command::FollowValidator { address } => {
                let mut stream = client
                    .blockchain
                    .subscribe_for_validator_election_by_address(address)
                    .await?;
                while let Some(validator) = stream.next().await {
                    println!("{:#?}", validator);
                }
            }

            Command::FollowLogsOfAddressesAndTypes {
                addresses,
                log_types,
            } => {
                let mut stream = client
                    .blockchain
                    .subscribe_for_logs_by_addresses_and_types(addresses, log_types)
                    .await?;

                while let Some(blocklog) = stream.next().await {
                    println!("{:#?}", blocklog);
                }
            }

            Command::Account(command) => {
                match command {
                    AccountCommand::List { short } => {
                        let accounts = client.wallet.list_accounts().await?;
                        for address in &accounts {
                            if short {
                                println!("{}", address.to_user_friendly_address());
                            } else {
                                let account = client
                                    .blockchain
                                    .get_account_by_address(address.clone())
                                    .await?;
                                println!("{}: {:#?}", address.to_user_friendly_address(), account);
                            }
                        }
                    }

                    AccountCommand::New { password } => {
                        let account = client.wallet.create_account(password).await?;
                        println!("{:#?}", account);
                    }

                    AccountCommand::Import { password, key_data } => {
                        let address = client.wallet.import_raw_key(key_data, password).await?;
                        println!("{}", address);
                    }

                    AccountCommand::Lock { address } => {
                        client.wallet.lock_account(address).await?;
                    }

                    AccountCommand::Unlock {
                        address, password, ..
                    } => {
                        // TODO: Duration
                        client
                            .wallet
                            .unlock_account(address, password, None)
                            .await?;
                    }

                    AccountCommand::Get { address } => {
                        let account = client.blockchain.get_account_by_address(address).await?;
                        println!("{:#?}", account);
                    }
                }
            }

            Command::Transaction(command) => match command {
                TransactionCommand::Basic {
                    wallet,
                    recipient,
                    value,
                    fee,
                    validity_start_height,
                    dry,
                } => {
                    if dry {
                        let tx = client
                            .consensus
                            .create_basic_transaction(
                                wallet,
                                recipient,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", tx);
                    } else {
                        let txid = client
                            .consensus
                            .send_basic_transaction(
                                wallet,
                                recipient,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", txid);
                    }
                }

                TransactionCommand::Stake {
                    wallet,
                    staker_address,
                    value,
                    fee,
                    validity_start_height,
                    dry,
                } => {
                    if dry {
                        let tx = client
                            .consensus
                            .create_stake_transaction(
                                wallet,
                                staker_address,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", tx);
                    } else {
                        let txid = client
                            .consensus
                            .send_stake_transaction(
                                wallet,
                                staker_address,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", txid);
                    }
                }

                TransactionCommand::Unstake {
                    wallet,
                    recipient,
                    value,
                    fee,
                    validity_start_height,
                    dry,
                } => {
                    if dry {
                        let tx = client
                            .consensus
                            .create_unstake_transaction(
                                wallet,
                                recipient,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", tx);
                    } else {
                        let txid = client
                            .consensus
                            .send_unstake_transaction(
                                wallet,
                                recipient,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", txid);
                    }
                }
            },
        }

        Ok(())
    }
}

pub struct Client {
    pub blockchain: BlockchainProxy<ArcClient<WebsocketClient>>,
    pub consensus: ConsensusProxy<ArcClient<WebsocketClient>>,
    pub mempool: MempoolProxy<ArcClient<WebsocketClient>>,
    pub wallet: WalletProxy<ArcClient<WebsocketClient>>,
}

impl Client {
    pub async fn new(url: Url, credentials: Option<Credentials>) -> Result<Self, Error> {
        let client = ArcClient::new(WebsocketClient::new(url, credentials).await?);

        Ok(Self {
            blockchain: BlockchainProxy::new(client.clone()),
            consensus: ConsensusProxy::new(client.clone()),
            mempool: MempoolProxy::new(client.clone()),
            wallet: WalletProxy::new(client),
        })
    }
}

async fn run_app(opt: Opt) -> Result<(), Error> {
    let url = opt
        .url
        .as_deref()
        .unwrap_or("ws://127.0.0.1:8648/ws")
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
