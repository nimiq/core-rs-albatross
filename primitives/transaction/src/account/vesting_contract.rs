use nimiq_keys::Address;
use nimiq_primitives::{
    account::AccountType,
    coin::{Coin, CoinBe},
};
use nimiq_serde::{Deserialize, Serialize, SerializedSize};

use crate::{
    account::AccountTransactionVerification, SignatureProof, Transaction, TransactionError,
    TransactionFlags,
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

        CreationTransactionData::parse(transaction).map(|_| ())
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
        let signature_proof = SignatureProof::deserialize_all(&transaction.proof)?;

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
#[derive(Clone, Debug, Default)]
pub struct CreationTransactionData {
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,
    /// The block height at which the release schedule starts.
    pub start_time: u64,
    /// The frequency at which funds are released.
    pub time_step: u64,
    /// The amount released at each [`time_step`](Self::time_step).
    pub step_amount: CoinBe,
    /// Initially locked balance.
    pub total_amount: CoinBe,
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
    pub step_amount: CoinBe,
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
    pub step_amount: CoinBe,
    pub total_amount: CoinBe,
}

impl CreationTransactionData {
    fn parse_impl(data: &[u8], tx_value: Coin) -> Result<Self, TransactionError> {
        Ok(match data.len() {
            CreationTransactionData8::SIZE => {
                // Only timestamp: vest full amount at that time
                let CreationTransactionData8 { owner, time_step } =
                    CreationTransactionData8::deserialize_all(data)?;
                CreationTransactionData {
                    owner,
                    start_time: 0,
                    time_step,
                    step_amount: tx_value.into(),
                    total_amount: tx_value.into(),
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
                    total_amount: tx_value.into(),
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
    pub fn parse(tx: &Transaction) -> Result<Self, TransactionError> {
        CreationTransactionData::parse_impl(&tx.recipient_data, tx.value)
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
