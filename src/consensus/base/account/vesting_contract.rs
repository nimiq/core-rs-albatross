use beserial::{Serialize, Deserialize};
use crate::consensus::base::primitive::{Address, Coin};
use crate::consensus::base::transaction::{Transaction, TransactionError, SignatureProof};
use super::{Account, AccountError};
use std::io;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct VestingContract {
    pub balance: Coin,
    pub owner: Address,
    pub vesting_start: u32,
    pub vesting_step_blocks: u32,
    pub vesting_step_amount: Coin,
    pub vesting_total_amount: Coin
}

impl VestingContract {
    pub fn create(balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return VestingContract::create_from_transaction(balance, transaction)
            .map_err(|_| AccountError::Any("Failed to create vesting contract".to_string()));
    }

    fn create_from_transaction(balance: Coin, transaction: &Transaction) -> io::Result<Self> {
        let reader = &mut &transaction.data[..];
        let owner = Deserialize::deserialize(reader)?;

        if transaction.data.len() == Address::SIZE + 4 {
            // Only block number: vest full amount at that block
            let vesting_step_blocks = Deserialize::deserialize(reader)?;
            return Ok(VestingContract::new(balance, owner, 0, vesting_step_blocks, transaction.value, transaction.value));
        }
        else if transaction.data.len() == Address::SIZE + 16 {
            let vesting_start = Deserialize::deserialize(reader)?;
            let vesting_step_blocks = Deserialize::deserialize(reader)?;
            let vesting_step_amount = Deserialize::deserialize(reader)?;
            return Ok(VestingContract::new(balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, transaction.value));
        }
        else if transaction.data.len() == Address::SIZE + 24 {
            // Create a vesting account with some instantly vested funds or additional funds considered.
            let vesting_start = Deserialize::deserialize(reader)?;
            let vesting_step_blocks = Deserialize::deserialize(reader)?;
            let vesting_step_amount = Deserialize::deserialize(reader)?;
            let vesting_total_amount = Deserialize::deserialize(reader)?;
            return Ok(VestingContract::new(balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount));
        }
        else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid transaction data"));
        }
    }

    fn new(balance: Coin, owner: Address, vesting_start: u32, vesting_step_blocks: u32, vesting_step_amount: Coin, vesting_total_amount: Coin) -> Self {
        return VestingContract { balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount };
    }

    pub fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
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

        Ok(())
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }

    fn with_balance(&self, balance: Coin) -> Self {
        return VestingContract {
            balance,
            owner: self.owner.clone(),
            vesting_start: self.vesting_start,
            vesting_step_blocks: self.vesting_step_blocks,
            vesting_step_amount: self.vesting_step_amount,
            vesting_total_amount: self.vesting_total_amount
        };
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::Any("Illegal incoming transaction".to_string()));
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::Any("Illegal incoming transaction".to_string()));
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;

        // Check vesting min cap.
        if balance < self.min_cap(block_height) {
            return Err(AccountError::Any("Insufficient funds available".to_string()));
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof = match Deserialize::deserialize(&mut &transaction.proof[..]) {
            Ok(v) => v,
            Err(e) => return Err(AccountError::Any("Invalid proof".to_string()))
        };
        if !signature_proof.is_signed_by(&self.owner) {
            return Err(AccountError::Any("Invalid signer".to_string()));
        }

        return Ok(self.with_balance(balance));
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(self.with_balance(balance));
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
