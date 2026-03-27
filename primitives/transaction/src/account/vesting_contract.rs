use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_serde::{Deserialize, Serialize, SerializedSize};

use crate::{
    account::AccountTransactionVerification, PoWSignatureProof, SignatureProof, Transaction,
    TransactionError, TransactionFlags,
};

/// The verifier trait for a basic account. This only uses data available in the transaction.
pub struct VestingContractVerifier;

impl AccountTransactionVerification for VestingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Vesting);

        if !transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            warn!(
                "Only contract creation is allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.flags.contains(TransactionFlags::SIGNALING) {
            warn!(
                "Signaling not allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Recipient address must match contract creation address for this transaction:\n{:?}",
                transaction);
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.network_id.is_albatross() {
            CreationTransactionData::parse(transaction).map(|_| ())
        } else {
            PoWCreationTransactionData::parse(transaction).map(|_| ())
        }
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Vesting);

        if !transaction.sender_data.is_empty() {
            warn!(
                "The following transaction can't have sender data:\n{:?}",
                transaction
            );
            return Err(TransactionError::Overflow);
        }

        // Verify signature.
        let signature_proof = if transaction.network_id.is_albatross() {
            SignatureProof::deserialize_all(&transaction.proof)?
        } else {
            PoWSignatureProof::deserialize_all(&transaction.proof)?.into_pos()
        };

        if !signature_proof.verify(&transaction.serialize_content()) {
            warn!("Invalid signature for this transaction:\n{:?}", transaction);
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}

/// Data used to create vesting contracts.
///
/// Used in [`Transaction::recipient_data`].
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreationTransactionData {
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,
    /// The timestamp at which the release schedule starts.
    pub start_time: u64,
    /// The frequency at which funds are released.
    pub time_step: u64,
    /// The amount released at each [`time_step`](Self::time_step).
    pub step_amount: Coin,
    /// Initially locked balance.
    pub total_amount: Coin,
}

#[derive(Deserialize, Serialize, SerializedSize)]
struct CreationTransactionData8 {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub time_step: u64,
}
#[derive(Deserialize, Serialize, SerializedSize)]
struct CreationTransactionData24 {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub start_time: u64,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub time_step: u64,
    pub step_amount: Coin,
}
#[derive(Deserialize, Serialize, SerializedSize)]
struct CreationTransactionData32 {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub start_time: u64,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl CreationTransactionData {
    pub fn parse_data(data: &[u8], tx_value: Coin) -> Result<Self, TransactionError> {
        Ok(match data.len() {
            CreationTransactionData8::SIZE => {
                // Only step length: vest full amount at that time
                let CreationTransactionData8 { owner, time_step } =
                    CreationTransactionData8::deserialize_all(data)?;
                CreationTransactionData {
                    owner,
                    start_time: 0,
                    time_step,
                    step_amount: tx_value,
                    total_amount: tx_value,
                }
            }
            CreationTransactionData24::SIZE => {
                let CreationTransactionData24 {
                    owner,
                    start_time,
                    time_step,
                    step_amount,
                } = CreationTransactionData24::deserialize_all(data)?;
                CreationTransactionData {
                    owner,
                    start_time,
                    time_step,
                    step_amount,
                    total_amount: tx_value,
                }
            }
            CreationTransactionData32::SIZE => {
                let CreationTransactionData32 {
                    owner,
                    start_time,
                    time_step,
                    step_amount,
                    total_amount,
                } = CreationTransactionData32::deserialize_all(data)?;
                if total_amount > tx_value {
                    return Err(TransactionError::InvalidData);
                }
                if step_amount > total_amount {
                    return Err(TransactionError::InvalidData);
                }
                CreationTransactionData {
                    owner,
                    start_time,
                    time_step,
                    step_amount,
                    total_amount,
                }
            }
            _ => return Err(TransactionError::InvalidData),
        })
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        CreationTransactionData::parse_data(&transaction.recipient_data, transaction.value)
    }

    pub fn to_tx_data(&self) -> Vec<u8> {
        let CreationTransactionData {
            owner,
            start_time,
            time_step,
            step_amount,
            total_amount,
        } = self.clone();
        if step_amount == total_amount {
            if start_time == 0 {
                CreationTransactionData8 { owner, time_step }.serialize_to_vec()
            } else {
                CreationTransactionData24 {
                    owner,
                    start_time,
                    time_step,
                    step_amount,
                }
                .serialize_to_vec()
            }
        } else {
            CreationTransactionData32 {
                owner,
                start_time,
                time_step,
                step_amount,
                total_amount,
            }
            .serialize_to_vec()
        }
    }
}

/// This struct represents vesting contract creation data in the Proof-of-Work chain. The differences
/// to the data in the Albatross chain are that the vesting period start and step length were u32 block numbers
/// in PoW instead of a u64 timestamps, making this serialization shorter.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PoWCreationTransactionData {
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,
    /// The block height at which the release schedule starts.
    pub start_block: u32,
    /// The frequency at which funds are released.
    pub step_blocks: u32,
    /// The amount released at each [`step_blocks`](Self::step_blocks).
    pub step_amount: Coin,
    /// Initially locked balance.
    pub total_amount: Coin,
}

#[derive(Deserialize, Serialize, SerializedSize)]
struct PoWCreationTransactionData4 {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub step_blocks: u32,
}
#[derive(Deserialize, Serialize, SerializedSize)]
struct PoWCreationTransactionData16 {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub start_block: u32,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub step_blocks: u32,
    pub step_amount: Coin,
}
#[derive(Deserialize, Serialize, SerializedSize)]
struct PoWCreationTransactionData24 {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub start_block: u32,
    #[serde(with = "nimiq_serde::fixint::be")]
    #[serialize_size(fixed_size)]
    pub step_blocks: u32,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl PoWCreationTransactionData {
    pub fn parse_data(data: &[u8], tx_value: Coin) -> Result<Self, TransactionError> {
        Ok(match data.len() {
            PoWCreationTransactionData4::SIZE => {
                // Only step length: vest full amount at that block
                let PoWCreationTransactionData4 { owner, step_blocks } =
                    PoWCreationTransactionData4::deserialize_all(data)?;
                PoWCreationTransactionData {
                    owner,
                    start_block: 1, // PoW genesis block number
                    step_blocks,
                    step_amount: tx_value,
                    total_amount: tx_value,
                }
            }
            PoWCreationTransactionData16::SIZE => {
                let PoWCreationTransactionData16 {
                    owner,
                    start_block,
                    step_blocks,
                    step_amount,
                } = PoWCreationTransactionData16::deserialize_all(data)?;
                PoWCreationTransactionData {
                    owner,
                    start_block,
                    step_blocks,
                    step_amount,
                    total_amount: tx_value,
                }
            }
            PoWCreationTransactionData24::SIZE => {
                let PoWCreationTransactionData24 {
                    owner,
                    start_block,
                    step_blocks,
                    step_amount,
                    total_amount,
                } = PoWCreationTransactionData24::deserialize_all(data)?;
                PoWCreationTransactionData {
                    owner,
                    start_block,
                    step_blocks,
                    step_amount,
                    total_amount,
                }
            }
            _ => return Err(TransactionError::InvalidData),
        })
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        PoWCreationTransactionData::parse_data(&transaction.recipient_data, transaction.value)
    }

    pub fn into_pos(
        self,
        genesis_block_number: u32,
        genesis_timestamp: u64,
    ) -> CreationTransactionData {
        let start_time = if self.start_block <= genesis_block_number {
            genesis_timestamp - (genesis_block_number - self.start_block) as u64 * 60_000
        } else {
            genesis_timestamp + (self.start_block - genesis_block_number) as u64 * 60_000
        };
        let time_step = self.step_blocks as u64 * 60_000;

        CreationTransactionData {
            owner: self.owner,
            start_time,
            time_step,
            step_amount: self.step_amount,
            total_amount: self.total_amount,
        }
    }
}
