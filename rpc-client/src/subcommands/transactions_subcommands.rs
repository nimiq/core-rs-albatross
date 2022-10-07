use clap::{Args, Parser};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_rpc_interface::{
    consensus::ConsensusInterface,
    types::{HashAlgorithm, ValidityStartHeight},
};
use nimiq_transaction::account::htlc_contract::AnyHash;

use anyhow::Error;
use async_trait::async_trait;

use crate::Client;

use super::accounts_subcommands::HandleSubcommand;

#[derive(Debug, Args)]
pub struct TxCommon {
    /// The associated transaction fee to be payed. If absent it defaults to 0 NIM.
    #[clap(short, long, default_value = "0")]
    pub fee: Coin,

    /// The block height from which on the transaction could be applied. The maximum amount of blocks the transaction is valid for
    /// is specified in `TRANSACTION_VALIDITY_WINDOW`.
    /// If absent it defaults to the current block height at time of processing.
    #[clap(short, long, default_value_t)]
    pub validity_start_height: ValidityStartHeight,

    /// Don't actually send the transaction, but output the transaction as hex string.
    #[clap(long)]
    pub dry: bool,
}

#[derive(Debug, Args)]
pub struct TxCommonWithValue {
    /// The amount of NIM to be used by the transaction.
    #[clap(long)]
    pub value: Coin,

    #[clap(flatten)]
    pub common_tx_fields: TxCommon,
}

#[derive(Debug, Parser)]
pub enum TransactionCommand {
    /// Sends a simple transaction from the wallet `wallet` to a basic `recipient`.
    Basic {
        /// Transaction will be sent from this address. The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// Recipient for this transaction. This must be a basic account.
        #[clap(short, long)]
        recipient: Address,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /* Staker transactions */
    /// Sends a `new_staker` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    NewStaker {
        /// The stake will be sent from this wallet. The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The staker address. This wallet must be unlocked prior to this action.
        #[clap(long)]
        staker_wallet: Address,

        /// Validator address to delegate stake to. If empty, no delegation will occur.
        #[clap(long)]
        delegation: Option<Address>,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Sends a staking transaction from the address of a given `wallet` to a given `staker_address`.
    Stake {
        /// The stake will be sent from this wallet. The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// Destination address for the stake.
        #[clap(long)]
        staker_address: Address,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Sends a `update_staker` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    UpdateStaker {
        /// The fee will be payed by this wallet if any is provided. In such case the sender wallet must be unlocked prior to this action.
        /// If absent the fee is payed by the stakers account.
        #[clap(long)]
        sender_wallet: Option<Address>,

        /// Destination address for the update. This wallet must be already unlocked.
        #[clap(long)]
        staker_wallet: Address,

        /// The new address for the delegation.
        #[clap(long)]
        new_delegation: Option<Address>,

        #[clap(flatten)]
        tx_commons: TxCommon,
    },

    /// Sends a `unstake` transaction to the network. The transaction fee will be paid from the funds
    /// being unstaked.
    Unstake {
        /// The stake will be sent from this wallet. The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The recipients of the previously staked coins.
        #[clap(long)]
        recipient: Address,

        /// The amount of NIM to unstake.
        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /* Vesting transactions */
    /// Sends a transaction creating a new vesting contract to the network.
    VestingCreate {
        /// The wallet used to sign the transaction. The vesting contract value is sent from the basic account
        /// belonging to this wallet. The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The owner of the vesting contract.
        #[clap(long)]
        owner: Address,

        #[clap(long)]
        start_time: u64,

        #[clap(long)]
        time_step: u64,

        /// Create a release schedule of `num_steps` payouts of value starting at `start_time + time_step`.
        #[clap(long)]
        num_steps: u32,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Sends a transaction redeeming a vesting contract to the network.
    VestingRedeem {
        /// The address to sign the transaction. This address should be the owner of the vesting contract.
        /// The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The vesting contract address.
        #[clap(long)]
        contract_address: Address,

        /// The address of the basic account that will receive the funds.
        #[clap(long)]
        recipient: Address,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /* HTLC transactions */
    /// Sends a transaction creating a new HTLC contract to the network.
    CreateHTLC {
        /// The wallet to sign the transaction. The HTLC contract value is sent from the basic account belonging to this wallet.
        /// The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The address of the sender in the HTLC contract.
        #[clap(long)]
        htlc_sender: Address,

        /// The address of the recipient in the HTLC contract.
        #[clap(long)]
        htlc_recipient: Address,

        /// The result of hashing the pre-image hash `hash_count` times.
        #[clap(short = 'r', long)]
        hash_root: AnyHash,

        /// Number of times the pre-image was hashed.
        #[clap(short = 'c', long)]
        hash_count: u8,

        /// The hashing algorithm used.
        #[clap(short = 'a', long, value_enum)]
        hash_algorithm: HashAlgorithm,

        /// Sets the blockchain height at which the `htlc_sender` automatically gains control over the funds.
        #[clap(long)]
        timeout: u64,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Sends a transaction redeeming a HTLC contract, using the `RegularTransfer` method, to the
    /// network.
    RedeemRegularHTLC {
        /// This address corresponds to the `htlc_recipient` in the HTLC contract.
        /// The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The address of the HTLC contract.
        #[clap(long)]
        contract_address: Address,

        /// The address of the basic account that will receive the funds.
        #[clap(long)]
        htlc_recipient: Address,

        #[clap(long)]
        pre_image: AnyHash,

        /// The result of hashing the pre-image hash `hash_count` times.
        #[clap(short = 'r', long)]
        hash_root: AnyHash,

        /// Number of times the pre-image was hashed.
        #[clap(short = 'c', long)]
        hash_count: u8,

        /// The `hash_root` is the result of hashing the `pre_image` `hash_count` times using `hash_algorithm`.
        #[clap(short = 'a', long, value_enum)]
        hash_algorithm: HashAlgorithm,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Sends a transaction redeeming a HTLC contract, using the `TimeoutResolve` method, to the
    /// network.
    RedeemHTLCTimeout {
        /// This address corresponds to the `htlc_recipient` in the HTLC contract.
        /// The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The address of the HTLC contract.
        #[clap(long)]
        contract_address: Address,

        /// The address of the basic account that will receive the funds.
        #[clap(long)]
        htlc_recipient: Address,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Sends a transaction redeeming a HTLC contract, using the `EarlyResolve` method, to the
    /// network.
    RedeemHTLCEarly {
        /// The address of the HTLC contract.
        #[clap(long)]
        contract_address: Address,

        /// The address of the basic account that will receive the funds.
        #[clap(long)]
        htlc_recipient: Address,

        /// The signature corresponding to the `htlc_sender` in the HTLC contract.
        #[clap(long)]
        htlc_sender_signature: String,

        /// The signature corresponding to the `htlc_recipient` in the HTLC contract.
        #[clap(long)]
        htlc_recipient_signature: String,

        #[clap(flatten)]
        tx_commons: TxCommonWithValue,
    },

    /// Returns a serialized signature that can be used to redeem funds from a HTLC contract using
    /// the `EarlyResolve` method.
    SignRedeemHTLCEarly {
        /// This is the address used to sign the transaction. It corresponds either to the `htlc_sender` or the `htlc_recipient`
        /// in the HTLC contract.
        /// The sender wallet must be unlocked prior to this action.
        #[clap(long)]
        sender_wallet: Address,

        /// The address of the HTLC contract.
        #[clap(long)]
        contract_address: Address,

        /// The address of the basic account that will receive the funds.
        #[clap(long)]
        htlc_recipient: Address,

        /// The amount of NIM to be used by the transaction.
        #[clap(long)]
        value: Coin,

        /// The associated transaction fee to be payed. If absent it defaults to 0 NIM.
        #[clap(short, long, default_value = "0")]
        fee: Coin,

        /// The block height from which on the transaction could be applied. The maximum amount of blocks the transaction is valid for
        /// is specified in `TRANSACTION_VALIDITY_WINDOW`.
        /// If absent it defaults to the current block height at time of processing.
        #[clap(short, long, default_value_t)]
        validity_start_height: ValidityStartHeight,
    },
}

#[async_trait]
impl HandleSubcommand for TransactionCommand {
    async fn handle_subcommand(self, mut client: Client) -> Result<(), Error> {
        match self {
            TransactionCommand::Basic {
                sender_wallet,
                recipient,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_basic_transaction(
                            sender_wallet,
                            recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_basic_transaction(
                            sender_wallet,
                            recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::NewStaker {
                sender_wallet,
                staker_wallet,
                delegation,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_new_staker_transaction(
                            sender_wallet,
                            staker_wallet,
                            delegation,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_new_staker_transaction(
                            sender_wallet,
                            staker_wallet,
                            delegation,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::Stake {
                sender_wallet,
                staker_address,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_stake_transaction(
                            sender_wallet,
                            staker_address,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_stake_transaction(
                            sender_wallet,
                            staker_address,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::UpdateStaker {
                sender_wallet,
                staker_wallet,
                new_delegation,
                tx_commons,
            } => {
                if tx_commons.dry {
                    let tx = client
                        .consensus
                        .create_update_staker_transaction(
                            sender_wallet,
                            staker_wallet,
                            new_delegation,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_update_staker_transaction(
                            sender_wallet,
                            staker_wallet,
                            new_delegation,
                            tx_commons.fee,
                            tx_commons.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::Unstake {
                sender_wallet,
                recipient,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_unstake_transaction(
                            sender_wallet,
                            recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_unstake_transaction(
                            sender_wallet,
                            recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }

            TransactionCommand::VestingCreate {
                sender_wallet,
                owner,
                start_time,
                time_step,
                num_steps,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_new_vesting_transaction(
                            sender_wallet,
                            owner,
                            start_time,
                            time_step,
                            num_steps,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_new_vesting_transaction(
                            sender_wallet,
                            owner,
                            start_time,
                            time_step,
                            num_steps,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::VestingRedeem {
                sender_wallet,
                contract_address,
                recipient,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_redeem_vesting_transaction(
                            sender_wallet,
                            contract_address,
                            recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_redeem_vesting_transaction(
                            sender_wallet,
                            contract_address,
                            recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }

            TransactionCommand::CreateHTLC {
                sender_wallet,
                htlc_sender,
                htlc_recipient,
                hash_root,
                hash_count,
                hash_algorithm,
                timeout,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_new_htlc_transaction(
                            sender_wallet,
                            htlc_sender,
                            htlc_recipient,
                            hash_root,
                            hash_count,
                            hash_algorithm.into(),
                            timeout,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_new_htlc_transaction(
                            sender_wallet,
                            htlc_sender,
                            htlc_recipient,
                            hash_root,
                            hash_count,
                            hash_algorithm.into(),
                            timeout,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::RedeemRegularHTLC {
                sender_wallet,
                contract_address,
                htlc_recipient,
                pre_image,
                hash_root,
                hash_count,
                hash_algorithm,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_redeem_regular_htlc_transaction(
                            sender_wallet,
                            contract_address,
                            htlc_recipient,
                            pre_image,
                            hash_root,
                            hash_count,
                            hash_algorithm.into(),
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_redeem_regular_htlc_transaction(
                            sender_wallet,
                            contract_address,
                            htlc_recipient,
                            pre_image,
                            hash_root,
                            hash_count,
                            hash_algorithm.into(),
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::RedeemHTLCTimeout {
                sender_wallet,
                contract_address,
                htlc_recipient,
                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_redeem_timeout_htlc_transaction(
                            sender_wallet,
                            contract_address,
                            htlc_recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_redeem_timeout_htlc_transaction(
                            sender_wallet,
                            contract_address,
                            htlc_recipient,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::RedeemHTLCEarly {
                contract_address,
                htlc_recipient,
                htlc_sender_signature,
                htlc_recipient_signature,

                tx_commons,
            } => {
                if tx_commons.common_tx_fields.dry {
                    let tx = client
                        .consensus
                        .create_redeem_early_htlc_transaction(
                            contract_address,
                            htlc_recipient,
                            htlc_sender_signature,
                            htlc_recipient_signature,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", tx);
                } else {
                    let txid = client
                        .consensus
                        .send_redeem_early_htlc_transaction(
                            contract_address,
                            htlc_recipient,
                            htlc_sender_signature,
                            htlc_recipient_signature,
                            tx_commons.value,
                            tx_commons.common_tx_fields.fee,
                            tx_commons.common_tx_fields.validity_start_height,
                        )
                        .await?;
                    println!("{}", txid);
                }
            }
            TransactionCommand::SignRedeemHTLCEarly {
                sender_wallet,
                contract_address,
                htlc_recipient,
                value,
                fee,
                validity_start_height,
            } => {
                let tx = client
                    .consensus
                    .sign_redeem_early_htlc_transaction(
                        sender_wallet,
                        contract_address,
                        htlc_recipient,
                        value,
                        fee,
                        validity_start_height,
                    )
                    .await?;
                println!("{}", tx);
            }
        }
        Ok(())
    }
}
