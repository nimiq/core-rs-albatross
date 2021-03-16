use anyhow::{bail, Error};
use futures::stream::StreamExt;
use structopt::StructOpt;

use nimiq_jsonrpc_core::Credentials;
use nimiq_keys::Address;
use nimiq_primitives::account::ValidatorId;
use nimiq_primitives::coin::Coin;
use nimiq_rpc_client::Client;
use nimiq_rpc_interface::{
    blockchain::BlockchainInterface,
    consensus::ConsensusInterface,
    types::{BlockNumberOrHash, ValidityStartHeight},
    wallet::WalletInterface,
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
    /// Sends a simple transaction from the wallet `wallet` to a basic `recipient`.
    Basic {
        /// Transaction will be sent from this address. An wallet with this address must be unlocked.
        wallet: Address,

        /// Recipient for this transaction. This must be a basic account.
        recipient: Address,

        /// The amount of NIM to send to the recipient.
        value: Coin,

        #[structopt(short, long, default_value = "0")]
        fee: Coin,

        #[structopt(short, long, default_value)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[structopt(long = "dry")]
        dry: bool,
    },

    /// Sends a staking transaction from the address of a given `key_pair` to a specified `validator_key`.
    Stake {
        /// The stake will be sent from this wallet.
        wallet: Address,

        /// The id of the validator to stake for.
        validator_id: ValidatorId,

        /// The amount of NIM to stake.
        value: Coin,

        #[structopt(short, long, default_value = "0")]
        fee: Coin,

        #[structopt(short, long, default_value)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[structopt(long = "dry")]
        dry: bool,
    },

    /// Retires the stake from the address of a given `key_pair` and a specified `validator_key`.
    Retire {
        /// The stake will be sent from this wallet.
        wallet: Address,

        validator_id: ValidatorId,

        value: Coin,

        #[structopt(short, long, default_value = "0")]
        fee: Coin,

        #[structopt(short, long, default_value)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[structopt(long = "dry")]
        dry: bool,
    },

    Reactivate {
        /// The stake will be sent from this wallet.
        wallet: Address,

        validator_id: ValidatorId,

        value: Coin,

        #[structopt(short, long, default_value = "0")]
        fee: Coin,

        #[structopt(short, long, default_value)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[structopt(long = "dry")]
        dry: bool,
    },

    Unstake {
        /// The stake will be sent from this wallet.
        wallet: Address,

        /// The recipients of the previously staked coins.
        recipient: Address,

        value: Coin,

        #[structopt(short, long, default_value = "0")]
        fee: Coin,

        #[structopt(short, long, default_value)]
        validity_start_height: ValidityStartHeight,

        /// Don't actually send the transaction, but output the transaction as hex string.
        #[structopt(long = "dry")]
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
                            .get_block_by_hash(hash, include_transactions)
                            .await
                    }
                    Some(BlockNumberOrHash::Number(number)) => {
                        client
                            .blockchain
                            .get_block_by_number(number, include_transactions)
                            .await
                    }
                    None => {
                        client
                            .blockchain
                            .get_latest_block(include_transactions)
                            .await
                    }
                }?;

                println!("{:#?}", block)
            }

            Command::Stakes {} => {
                let stakes = client.blockchain.list_stakes().await?;
                println!("{:#?}", stakes);
            }

            Command::Follow { block: show_block } => {
                let mut stream = client.blockchain.head_subscribe().await?;

                while let Some(block_hash) = stream.next().await {
                    if show_block {
                        let block = client.blockchain.get_block_by_hash(block_hash, false).await;
                        println!("{:#?}", block);
                    } else {
                        println!("{}", block_hash);
                    }
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
                                let account =
                                    client.blockchain.get_account(address.clone()).await?;
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
                        let account = client.blockchain.get_account(address).await?;
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
                    validator_id,
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
                                validator_id,
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
                                validator_id,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", txid);
                    }
                }

                TransactionCommand::Retire {
                    wallet,
                    validator_id,
                    value,
                    fee,
                    validity_start_height,
                    dry,
                } => {
                    if dry {
                        let tx = client
                            .consensus
                            .create_retire_transaction(
                                wallet,
                                validator_id,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", tx);
                    } else {
                        let txid = client
                            .consensus
                            .send_retire_transaction(
                                wallet,
                                validator_id,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", txid);
                    }
                }

                TransactionCommand::Reactivate {
                    wallet,
                    validator_id,
                    value,
                    fee,
                    validity_start_height,
                    dry,
                } => {
                    if dry {
                        let tx = client
                            .consensus
                            .create_reactivate_transaction(
                                wallet,
                                validator_id,
                                value,
                                fee,
                                validity_start_height,
                            )
                            .await?;
                        println!("{}", tx);
                    } else {
                        let txid = client
                            .consensus
                            .send_reactivate_transaction(
                                wallet,
                                validator_id,
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
    dotenv::dotenv().ok();
    pretty_env_logger::init();

    let opt = Opt::from_args();
    if let Err(e) = run_app(opt).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
