use beserial::Deserialize;
use keys::Address;
use primitives::account::AccountType;
use primitives::coin::Coin;

use crate::{Transaction, TransactionError, TransactionFlags};
use crate::account::AccountTransactionVerification;
use crate::SignatureProof;

pub struct VestingContractVerifier {}

impl AccountTransactionVerification for VestingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Vesting);

        if transaction.sender == transaction.recipient {
            return Err(TransactionError::SenderEqualsRecipient);
        }

        // Check that value > 0.
        if transaction.value == Coin::ZERO {
            return Err(TransactionError::ZeroValue);
        }

        if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            warn!("Only contract creation is allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Recipient address must match contract creation address");
            return Err(TransactionError::InvalidForRecipient);
        }

        let allowed_sizes = [Address::SIZE + 4, Address::SIZE + 16, Address::SIZE + 24];
        if !allowed_sizes.contains(&transaction.data.len()) {
            warn!("Invalid creation data: invalid length");
            return Err(TransactionError::InvalidData);
        }

        CreationTransactionData::parse(transaction).map(|_| ())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Vesting);

        // Verify signature.
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;
        if !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CreationTransactionData {
    pub owner: Address,
    pub start: u32,
    pub step_blocks: u32,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl CreationTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.data[..];
        let owner = Deserialize::deserialize(reader)?;

        if transaction.data.len() == Address::SIZE + 4 {
            // Only block number: vest full amount at that block
            let step_blocks = Deserialize::deserialize(reader)?;
            Ok(CreationTransactionData {
                owner,
                start: 0,
                step_blocks,
                step_amount: transaction.value,
                total_amount: transaction.value,
            })
        } else if transaction.data.len() == Address::SIZE + 16 {
            let start = Deserialize::deserialize(reader)?;
            let step_blocks = Deserialize::deserialize(reader)?;
            let step_amount = Deserialize::deserialize(reader)?;
            Ok(CreationTransactionData {
                owner,
                start,
                step_blocks,
                step_amount,
                total_amount: transaction.value,
            })
        } else if transaction.data.len() == Address::SIZE + 24 {
            // Create a vesting account with some instantly vested funds or additional funds considered.
            let start = Deserialize::deserialize(reader)?;
            let step_blocks = Deserialize::deserialize(reader)?;
            let step_amount = Deserialize::deserialize(reader)?;
            let total_amount = Deserialize::deserialize(reader)?;
            Ok(CreationTransactionData {
                owner,
                start,
                step_blocks,
                step_amount,
                total_amount,
            })
        } else {
            Err(TransactionError::InvalidData)
        }
    }
}
