use beserial::{Deserialize, Serialize};
use keys::Address;

use crate::coin::Coin;
#[cfg(feature = "transaction")]
use crate::transaction::{SignatureProof, Transaction, TransactionError, TransactionFlags};

use super::{Account, AccountError, AccountType};
#[cfg(feature = "transaction")]
use crate::account::AccountTransactionInteraction;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct VestingContract {
    pub balance: Coin,
    pub owner: Address,
    pub vesting_start: u32,
    pub vesting_step_blocks: u32,
    pub vesting_step_amount: Coin,
    pub vesting_total_amount: Coin,
}

impl VestingContract {
    pub fn new(balance: Coin, owner: Address, vesting_start: u32, vesting_step_blocks: u32, vesting_step_amount: Coin, vesting_total_amount: Coin) -> Self {
        return VestingContract { balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount };
    }

    fn with_balance(&self, balance: Coin) -> Self {
        return VestingContract {
            balance,
            owner: self.owner.clone(),
            vesting_start: self.vesting_start,
            vesting_step_blocks: self.vesting_step_blocks,
            vesting_step_amount: self.vesting_step_amount,
            vesting_total_amount: self.vesting_total_amount,
        };
    }

    pub fn min_cap(&self, block_height: u32) -> Coin {
        return if self.vesting_step_blocks > 0 && self.vesting_step_amount > Coin::ZERO {
            let steps = ((block_height - self.vesting_start) as f64 / self.vesting_step_blocks as f64).floor();
            let min_cap = u64::from(self.vesting_total_amount) as f64 - steps * u64::from(self.vesting_step_amount) as f64;
            Coin::from(min_cap.max(0f64) as u64)
        } else {
            Coin::ZERO
        };
    }
}

#[cfg(feature = "transaction")]
impl VestingContract {
    fn parse_and_verify_creation_transaction(transaction: &Transaction) -> Result<(Address, u32, u32, Coin, Coin), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Vesting);

        if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            warn!("Only contract creation is allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        // The contract creation transaction is the only valid incoming transaction.
        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Only contract creation is allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        // Check that data field has the correct length.
        let allowed_sizes = [Address::SIZE + 4, Address::SIZE + 16, Address::SIZE + 24];
        if !allowed_sizes.contains(&transaction.data.len()) {
            warn!("Invalid creation data: invalid length");
            return Err(TransactionError::InvalidData);
        }

        Ok(VestingContract::parse_creation_transaction(transaction)?)
    }

    fn parse_creation_transaction(transaction: &Transaction) -> Result<(Address, u32, u32, Coin, Coin), TransactionError> {
        let reader = &mut &transaction.data[..];
        let owner = Deserialize::deserialize(reader)?;

        if transaction.data.len() == Address::SIZE + 4 {
            // Only block number: vest full amount at that block
            let vesting_step_blocks = Deserialize::deserialize(reader)?;
            return Ok((owner, 0, vesting_step_blocks, transaction.value, transaction.value));
        } else if transaction.data.len() == Address::SIZE + 16 {
            let vesting_start = Deserialize::deserialize(reader)?;
            let vesting_step_blocks = Deserialize::deserialize(reader)?;
            let vesting_step_amount = Deserialize::deserialize(reader)?;
            return Ok((owner, vesting_start, vesting_step_blocks, vesting_step_amount, transaction.value));
        } else if transaction.data.len() == Address::SIZE + 24 {
            // Create a vesting account with some instantly vested funds or additional funds considered.
            let vesting_start = Deserialize::deserialize(reader)?;
            let vesting_step_blocks = Deserialize::deserialize(reader)?;
            let vesting_step_amount = Deserialize::deserialize(reader)?;
            let vesting_total_amount = Deserialize::deserialize(reader)?;
            return Ok((owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount));
        } else {
            unreachable!();
        }
    }
}

#[cfg(feature = "transaction")]
impl AccountTransactionInteraction for VestingContract {
    fn create(balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let (owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount) = VestingContract::parse_and_verify_creation_transaction(transaction)?;
        return Ok(VestingContract::new(balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount));
    }

    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        VestingContract::parse_and_verify_creation_transaction(transaction)?;
        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }

    fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::InvalidForRecipient);
    }

    fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::InvalidForRecipient);
    }

    fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;

        // Check vesting min cap.
        if balance < self.min_cap(block_height) {
            return Err(AccountError::InsufficientFunds);
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;
        if !signature_proof.is_signed_by(&self.owner) {
            return Err(AccountError::InvalidSignature);
        }

        return Ok(self.with_balance(balance));
    }

    fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(self.with_balance(balance));
    }
}
