use log::error;

use beserial::{Deserialize, Serialize, SerializingError, WriteBytesExt};
use keys::Address;
use primitives::account::AccountType;
use primitives::coin::Coin;

use crate::account::AccountTransactionVerification;
use crate::SignatureProof;
use crate::{Transaction, TransactionError, TransactionFlags};

/// The verifier trait for a basic account. This only uses data available in the transaction.
pub struct VestingContractVerifier {}

impl AccountTransactionVerification for VestingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Vesting);

        if transaction.sender == transaction.recipient {
            error!(
                "The following transaction can't have the same sender and recipient:\n{:?}",
                transaction
            );
            return Err(TransactionError::SenderEqualsRecipient);
        }

        if !transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            error!(
                "Only contract creation is allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.flags.contains(TransactionFlags::SIGNALLING) {
            error!(
                "Signalling not allowed for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            error!("Recipient address must match contract creation address for this transaction:\n{:?}",
                transaction);
            return Err(TransactionError::InvalidForRecipient);
        }

        let allowed_sizes = [Address::SIZE + 8, Address::SIZE + 24, Address::SIZE + 32];
        if !allowed_sizes.contains(&transaction.data.len()) {
            warn!(
                "Invalid data length for this transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidData);
        }

        CreationTransactionData::parse(transaction).map(|_| ())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Vesting);

        // Verify signature.
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature for this transaction:\n{:?}", transaction);
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}

#[derive(Default, Clone, Debug)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct CreationTransactionData {
    pub owner: Address,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl CreationTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.data[..];
        let owner = Deserialize::deserialize(reader)?;

        if transaction.data.len() == Address::SIZE + 8 {
            // Only timestamp: vest full amount at that time
            let time_step = Deserialize::deserialize(reader)?;
            Ok(CreationTransactionData {
                owner,
                start_time: 0,
                time_step,
                step_amount: transaction.value,
                total_amount: transaction.value,
            })
        } else if transaction.data.len() == Address::SIZE + 24 {
            let start_time = Deserialize::deserialize(reader)?;
            let time_step = Deserialize::deserialize(reader)?;
            let step_amount = Deserialize::deserialize(reader)?;
            Ok(CreationTransactionData {
                owner,
                start_time,
                time_step,
                step_amount,
                total_amount: transaction.value,
            })
        } else if transaction.data.len() == Address::SIZE + 32 {
            // Create a vesting account with some instantly vested funds or additional funds considered.
            let start_time = Deserialize::deserialize(reader)?;
            let time_step = Deserialize::deserialize(reader)?;
            let step_amount = Deserialize::deserialize(reader)?;
            let total_amount = Deserialize::deserialize(reader)?;
            Ok(CreationTransactionData {
                owner,
                start_time,
                time_step,
                step_amount,
                total_amount,
            })
        } else {
            Err(TransactionError::InvalidData)
        }
    }
}

impl Serialize for CreationTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.owner.serialize(writer)?;

        if self.step_amount == self.total_amount {
            if self.start_time == 0 {
                size += self.time_step.serialize(writer)?;
            } else {
                size += self.start_time.serialize(writer)?;
                size += self.time_step.serialize(writer)?;
                size += self.step_amount.serialize(writer)?;
            }
        } else {
            size += self.start_time.serialize(writer)?;
            size += self.time_step.serialize(writer)?;
            size += self.step_amount.serialize(writer)?;
            size += self.total_amount.serialize(writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        if self.step_amount == self.total_amount {
            if self.start_time == 0 {
                Address::SIZE + 8
            } else {
                Address::SIZE + 24
            }
        } else {
            Address::SIZE + 32
        }
    }
}
