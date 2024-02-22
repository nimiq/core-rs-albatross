use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;
use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature};
use nimiq_rpc_interface::{blockchain::BlockchainInterface, wallet::WalletInterface};

use crate::Client;

#[async_trait]
pub trait HandleSubcommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<Client, Error>;
}

#[derive(Debug, Parser)]
pub enum AccountCommand {
    /// Lists all the currently unlocked accounts.
    List {
        /// Lists only the addresses of the accounts.
        #[clap(short, long)]
        short: bool,
    },

    /// Creates a new account. This doesn't unlock the account automatically.
    New {
        /// Encryption password.
        #[clap(short = 'P', long)]
        password: Option<String>,
    },

    /// Imports an existing account. The account remains locked after this operation.
    Import {
        #[clap(short = 'P', long)]
        password: Option<String>,
        /// The private key of the account to be unlocked.
        key_data: String,
    },

    /// Checks if account is imported.
    IsImported {
        /// The account's address.
        address: Address,
    },

    /// Locks a currently unlocked account.
    Lock {
        /// The account's address.
        address: Address,
    },

    /// Unlocks an account.
    Unlock {
        #[clap(short = 'P', long)]
        password: Option<String>,

        /// The account's address.
        address: Address,
    },

    /// Checks if account is unlocked.
    IsUnlocked {
        /// The account's address.
        address: Address,
    },

    /// Signs a message using the specified account. The account must already be unlocked.
    Sign {
        /// The message to be signed.
        message: String,

        /// The address to sign the message.
        address: Address,

        /// Specifies if the message is in hexadecimal.
        #[clap(long)]
        is_hex: bool,
    },

    /// Verifies if the message was signed by specified account. The account must already be unlocked.
    VerifySignature {
        /// The signed message to be verified.
        message: String,

        /// The public key returned upon signing the message.
        public_key: Ed25519PublicKey,

        /// The signature returned upon signing the message. The r and s bytes should be all concatenated
        /// into one continuous input.
        signature: Ed25519Signature,

        /// Specifies if the message is in hexadecimal.
        #[clap(long)]
        is_hex: bool,
    },

    /// Queries all accounts in the accounts tree
    GetAll {},

    /// Queries the account state (e.g. account balance for basic accounts).
    Get {
        /// The account's address.
        address: Address,
    },
}

#[async_trait]
impl HandleSubcommand for AccountCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<Client, Error> {
        match self {
            AccountCommand::List { short } => {
                let accounts = client.wallet.list_accounts().await?.data;
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
                println!("{:#?}", client.wallet.create_account(password).await?);
            }
            AccountCommand::Import { password, key_data } => {
                let address = client.wallet.import_raw_key(key_data, password).await?;
                println!("{address:#?}");
            }
            AccountCommand::IsImported { address } => {
                println!("{:#?}", client.wallet.is_account_imported(address).await?);
            }
            AccountCommand::Lock { address } => {
                client.wallet.lock_account(address).await?;
            }
            AccountCommand::Unlock {
                address, password, ..
            } => {
                // TODO: Duration
                println!(
                    "{:#?}",
                    client
                        .wallet
                        .unlock_account(address, password, None)
                        .await?
                );
            }
            AccountCommand::IsUnlocked { address } => {
                println!("{:#?}", client.wallet.is_account_unlocked(address).await?);
            }
            AccountCommand::Sign {
                message,
                address,
                is_hex,
            } => {
                println!(
                    "{:#?}",
                    client.wallet.sign(message, address, None, is_hex).await?
                );
            }
            AccountCommand::VerifySignature {
                message,
                public_key,
                signature,
                is_hex,
            } => {
                println!(
                    "{:#?}",
                    client
                        .wallet
                        .verify_signature(message, public_key, signature, is_hex)
                        .await?
                );
            }
            AccountCommand::Get { address } => {
                println!(
                    "{:#?}",
                    client.blockchain.get_account_by_address(address).await?
                );
            }

            AccountCommand::GetAll {} => {
                println!("{:#?}", client.blockchain.get_accounts().await?);
            }
        }

        Ok(client)
    }
}
