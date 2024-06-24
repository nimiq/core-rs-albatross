use log::error;
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_serde::{Deserialize, Serialize};

use crate::{
    account::AccountTransactionVerification, SignatureProof, Transaction, TransactionError,
    TransactionFlags,
};

/// The verifier trait for a basic account. This only uses data available in the transaction.
pub struct VestingContractVerifier {}

impl AccountTransactionVerification for VestingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Vesting);

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

        if transaction.flags.contains(TransactionFlags::SIGNALING) {
            error!(
                "Signaling not allowed for this transaction:\n{:?}",
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
        if !allowed_sizes.contains(&transaction.recipient_data.len()) {
            warn!(
                len = transaction.recipient_data.len(),
                ?transaction,
                "Invalid data length for this transaction",
            );
            return Err(TransactionError::InvalidData);
        }

        CreationTransactionData::parse(transaction).map(|_| ())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Vesting);

        // Verify signature.
        let signature_proof = SignatureProof::deserialize_all(&transaction.proof)?;

        if !signature_proof.verify(&transaction.serialize_content()) {
            warn!("Invalid signature for this transaction:\n{:?}", transaction);
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct CreationTransactionData {
    pub owner: Address,
    #[serde(with = "nimiq_serde::fixint::be")]
    pub start_time: u64,
    #[serde(with = "nimiq_serde::fixint::be")]
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl CreationTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.recipient_data[..];
        let (owner, left_over) = Deserialize::deserialize_take(reader)?;

        if transaction.recipient_data.len() == Address::SIZE + 8 {
            // Only timestamp: vest full amount at that time
            let (time_step, _) = <[u8; 8]>::deserialize_take(left_over)?;
            Ok(CreationTransactionData {
                owner,
                start_time: 0,
                time_step: u64::from_be_bytes(time_step),
                step_amount: transaction.value,
                total_amount: transaction.value,
            })
        } else if transaction.recipient_data.len() == Address::SIZE + 24 {
            let (start_time, left_over) = <[u8; 8]>::deserialize_take(left_over)?;
            let (time_step, left_over) = <[u8; 8]>::deserialize_take(left_over)?;
            let (step_amount, _) = Deserialize::deserialize_take(left_over)?;
            Ok(CreationTransactionData {
                owner,
                start_time: u64::from_be_bytes(start_time),
                time_step: u64::from_be_bytes(time_step),
                step_amount,
                total_amount: transaction.value,
            })
        } else if transaction.recipient_data.len() == Address::SIZE + 32 {
            // Create a vesting account with some instantly vested funds or additional funds considered.
            let (start_time, left_over) = <[u8; 8]>::deserialize_take(left_over)?;
            let (time_step, left_over) = <[u8; 8]>::deserialize_take(left_over)?;
            let (step_amount, left_over) = Deserialize::deserialize_take(left_over)?;
            let (total_amount, _) = Deserialize::deserialize_take(left_over)?;
            Ok(CreationTransactionData {
                owner,
                start_time: u64::from_be_bytes(start_time),
                time_step: u64::from_be_bytes(time_step),
                step_amount,
                total_amount,
            })
        } else {
            Err(TransactionError::InvalidData)
        }
    }

    pub fn to_tx_data(&self) -> Vec<u8> {
        let mut data = self.owner.serialize_to_vec();

        if self.step_amount == self.total_amount {
            if self.start_time == 0 {
                data.append(&mut self.time_step.to_be_bytes().serialize_to_vec());
            } else {
                data.append(&mut self.start_time.to_be_bytes().serialize_to_vec());
                data.append(&mut self.time_step.to_be_bytes().serialize_to_vec());
                data.append(&mut self.step_amount.serialize_to_vec());
            }
        } else {
            data.append(&mut self.start_time.to_be_bytes().serialize_to_vec());
            data.append(&mut self.time_step.to_be_bytes().serialize_to_vec());
            data.append(&mut self.step_amount.serialize_to_vec());
            data.append(&mut self.total_amount.serialize_to_vec());
        }

        data
    }
}
