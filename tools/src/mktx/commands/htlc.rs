use std::str::FromStr;

use clap::{Args, Subcommand, ValueEnum};
use nimiq_keys::Address;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_transaction::account::htlc_contract::{AnyHash, AnyHash32, AnyHash64, PreImage};
use nimiq_transaction_builder::TransactionBuilder;

use super::{hex_secret_key_to_pair, hex_to_signature_proof, CommandError, TransactionOrProof};

#[derive(Clone, Debug, ValueEnum)]
pub enum HashType {
    Blake2b,
    Sha256,
    Sha512,
}

#[derive(Clone, Debug, Args)]
pub struct HtlcRequiredArgs {
    /// NQ address that receives the funds once the HTLC unlocks
    recipient: Address,
    /// Amount to transfer in Lunas; ensure balance covers value + fee
    value: Coin,
}

#[derive(Debug, Subcommand)]
pub enum HtlcCommands {
    /// Deploys a new HTLC
    Create {
        #[clap(flatten)]
        required_args: HtlcRequiredArgs,
        /// Hex-encoded sender private key; signs the contract creation transaction
        hex_secret_key: String,
        /// NQ address that funds the HTLC and can redeem it
        htlc_sender: Address,
        /// Hash algorithm used
        hash_type: HashType,
        /// Hex-encoded hash root that must be matched to redeem the HTLC
        hash_root: String,
        /// Number of times the hash function must be applied to the pre-image to match the hash root
        hash_count: u8,
        /// Timeout in number of blocks after which the sender can redeem the HTLC
        timeout: u64,
    },
    /// Redeems an HTLC using the pre-image
    RedeemRegular {
        #[clap(flatten)]
        required_args: HtlcRequiredArgs,
        /// Hex-encoded recipient private key signing the redemption transaction
        hex_secret_key: String,
        /// Address of the HTLC contract to redeem
        contract_address: Address,
        /// Hex-encoded pre-image that hashes to the hash root
        pre_image: String,
        /// Hash algorithm used
        hash_type: HashType,
        /// Hex-encoded hash root that must be matched to redeem the HTLC
        hash_root: String,
        /// Hash counter that matches the contract’s expected value. Must be <= the contract’s hash count
        hash_count: u8,
    },
    /// Redeems an HTLC after timeout
    RedeemTimeout {
        #[clap(flatten)]
        required_args: HtlcRequiredArgs,
        /// Hex-encoded sender private key authorizing the timeout redemption
        hex_secret_key: String,
        /// Address of the HTLC contract to redeem
        contract_address: Address,
    },
    /// Redeems an HTLC early using signatures from both sender and recipient
    RedeemEarly {
        #[clap(flatten)]
        required_args: HtlcRequiredArgs,
        /// Address of the HTLC contract to redeem
        contract_address: Address,
        /// Hex-encoded signature proof produced by `htlc sign-early` using the sender key
        htlc_sender_signature: String,
        /// Hex-encoded signature proof produced by `htlc sign-early` using the recipient key
        htlc_recipient_signature: String,
    },
    /// Produces a signature proof for an early redeem transaction; either party signs this transaction and it emits a SignatureProof hex blob for the counterparty. Sign first, then use `htlc redeem-early`
    SignEarly {
        #[clap(flatten)]
        required_args: HtlcRequiredArgs,
        /// Hex-encoded private key of the sender or recipient
        hex_secret_key: String,
        /// Address of the HTLC contract to redeem
        contract_address: Address,
    },
}

impl HtlcCommands {
    fn required_args(&self) -> &HtlcRequiredArgs {
        match self {
            HtlcCommands::Create { required_args, .. } => required_args,
            HtlcCommands::RedeemRegular { required_args, .. } => required_args,
            HtlcCommands::RedeemTimeout { required_args, .. } => required_args,
            HtlcCommands::RedeemEarly { required_args, .. } => required_args,
            HtlcCommands::SignEarly { required_args, .. } => required_args,
        }
    }
}

pub fn get_tx(
    subcommand: HtlcCommands,
    fee: Coin,
    validity_start_height: u32,
    network_id: NetworkId,
) -> Result<TransactionOrProof, CommandError> {
    let HtlcRequiredArgs { recipient, value } = subcommand.required_args().clone();

    match subcommand {
        HtlcCommands::Create {
            hex_secret_key,
            htlc_sender,
            hash_type,
            hash_root,
            hash_count,
            timeout,
            ..
        } => {
            let hash_root = match hash_type {
                HashType::Blake2b => AnyHash::Blake2b(AnyHash32::from_str(&hash_root)?),
                HashType::Sha256 => AnyHash::Sha256(AnyHash32::from_str(&hash_root)?),
                HashType::Sha512 => AnyHash::Sha512(AnyHash64::from_str(&hash_root)?),
            };

            Ok(TransactionOrProof::Transaction(
                TransactionBuilder::new_create_htlc(
                    &hex_secret_key_to_pair(hex_secret_key)?,
                    htlc_sender,
                    recipient,
                    hash_root,
                    hash_count,
                    timeout,
                    value,
                    fee,
                    validity_start_height,
                    network_id,
                )?,
            ))
        }
        HtlcCommands::RedeemRegular {
            hex_secret_key,
            contract_address,
            pre_image,
            hash_type,
            hash_root,
            hash_count,
            ..
        } => {
            let hash_root = match hash_type {
                HashType::Blake2b => AnyHash::Blake2b(AnyHash32::from_str(&hash_root)?),
                HashType::Sha256 => AnyHash::Sha256(AnyHash32::from_str(&hash_root)?),
                HashType::Sha512 => AnyHash::Sha512(AnyHash64::from_str(&hash_root)?),
            };

            Ok(TransactionOrProof::Transaction(
                TransactionBuilder::new_redeem_htlc_regular(
                    &hex_secret_key_to_pair(hex_secret_key)?,
                    contract_address,
                    recipient,
                    PreImage::from_str(&pre_image)?,
                    hash_root,
                    hash_count,
                    value,
                    fee,
                    validity_start_height,
                    network_id,
                )?,
            ))
        }
        HtlcCommands::RedeemTimeout {
            hex_secret_key,
            contract_address,
            ..
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_redeem_htlc_timeout(
                &hex_secret_key_to_pair(hex_secret_key)?,
                contract_address,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        HtlcCommands::RedeemEarly {
            contract_address,
            htlc_sender_signature,
            htlc_recipient_signature,
            ..
        } => Ok(TransactionOrProof::Transaction(
            TransactionBuilder::new_redeem_htlc_early(
                contract_address,
                recipient,
                hex_to_signature_proof(htlc_sender_signature)?,
                hex_to_signature_proof(htlc_recipient_signature)?,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
        HtlcCommands::SignEarly {
            hex_secret_key,
            contract_address,
            ..
        } => Ok(TransactionOrProof::Proof(
            TransactionBuilder::sign_htlc_early(
                &hex_secret_key_to_pair(hex_secret_key)?,
                contract_address,
                recipient,
                value,
                fee,
                validity_start_height,
                network_id,
            )?,
        )),
    }
}
