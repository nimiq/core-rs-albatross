use beserial::{Serialize, Deserialize};
use crate::consensus::base::account::{Account, AccountError, AccountType};
use crate::consensus::base::transaction::{Transaction, TransactionError, TransactionFlags};
use crate::consensus::base::transaction::SignatureProof;
use crate::consensus::base::primitive::{Address, Coin};
use crate::consensus::base::primitive::hash::{Hasher, Blake2bHasher, Sha256Hasher};
use hex::FromHex;

create_typed_array!(AnyHash, u8, 32);
add_hex_io_fns_typed_arr!(AnyHash, AnyHash::SIZE);

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct HashedTimeLockedContract {
    pub balance: Coin,
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u32,
    pub total_amount: Coin
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum HashAlgorithm {
    Blake2b = 1,
    Sha256 = 3
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProofType {
    RegularTransfer = 1,
    EarlyResolve = 2,
    TimeoutResolve = 3
}

impl HashedTimeLockedContract {
    pub fn create(balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let (sender, recipient, hash_algorithm, hash_root, hash_count, timeout) = HashedTimeLockedContract::parse_and_verify_creation_transaction(transaction)?;
        return Ok(HashedTimeLockedContract::new(balance, sender, recipient, hash_algorithm, hash_root, hash_count, timeout, transaction.value));
    }

    fn new(balance: Coin, sender: Address, recipient: Address, hash_algorithm: HashAlgorithm, hash_root: AnyHash, hash_count: u8, timeout: u32, total_amount: Coin) -> Self {
        return HashedTimeLockedContract { balance, sender, recipient, hash_algorithm, hash_root, hash_count, timeout, total_amount };
    }

    pub fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        HashedTimeLockedContract::parse_and_verify_creation_transaction(transaction)?;
        Ok(())
    }

    fn parse_and_verify_creation_transaction(transaction: &Transaction) -> Result<(Address, Address, HashAlgorithm, AnyHash, u8, u32), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::HTLC);

        if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            warn!("Only contract creation is allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Recipient address must match contract creation address");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.data.len() != (20 * 2 + 1 + 32 + 1 + 4) {
            warn!("Invalid creation data: invalid length");
            return Err(TransactionError::InvalidData);
        }

        let (sender, recipient, hash_algorithm, hash_root, hash_count, timeout) = HashedTimeLockedContract::parse_creation_transaction(transaction)?;

        if hash_count == 0 {
            warn!("Invalid creation data: hash_count may not be zero");
            return Err(TransactionError::InvalidData);
        }

        Ok((sender, recipient, hash_algorithm, hash_root, hash_count, timeout))
    }

    fn parse_creation_transaction(transaction: &Transaction) -> Result<(Address, Address, HashAlgorithm, AnyHash, u8, u32), TransactionError> {
        let reader = &mut &transaction.data[..];

        let sender: Address = Deserialize::deserialize(reader)?;
        let recipient: Address = Deserialize::deserialize(reader)?;
        let hash_algorithm: HashAlgorithm = Deserialize::deserialize(reader)?;
        let hash_root = Deserialize::deserialize(reader)?;
        let hash_count = Deserialize::deserialize(reader)?;
        let timeout = Deserialize::deserialize(reader)?;

        return Ok((sender, recipient, hash_algorithm, hash_root, hash_count, timeout));
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        let tx_content = transaction.serialize_content();
        let tx_buf = tx_content.as_slice();

        let proof_buf = &mut &transaction.proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;
        match proof_type {
            ProofType::RegularTransfer => {
                let hash_algorithm: HashAlgorithm = Deserialize::deserialize(proof_buf)?;
                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;
                let hash_root: [u8; 32] = AnyHash::deserialize(proof_buf)?.into();
                let mut pre_image: [u8; 32] = AnyHash::deserialize(proof_buf)?.into();
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if proof_buf.len() != 0 {
                    warn!("Over-long proof");
                    return Err(TransactionError::InvalidProof);
                }

                for i in 0..hash_depth {
                    match hash_algorithm {
                        HashAlgorithm::Blake2b => {
                            pre_image = Blake2bHasher::default().digest(&pre_image[..]).into();
                        },
                        HashAlgorithm::Sha256 => {
                            pre_image = Sha256Hasher::default().digest(&pre_image[..]).into();
                        }
                    }
                }

                if hash_root != pre_image {
                    warn!("Hash mismatch");
                    return Err(TransactionError::InvalidProof);
                }

                if !signature_proof.verify(tx_buf) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            },
            ProofType::EarlyResolve => {
                let signature_proof_recipient: SignatureProof = Deserialize::deserialize(proof_buf)?;
                let signature_proof_sender: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if proof_buf.len() != 0 {
                    warn!("Over-long proof");
                    return Err(TransactionError::InvalidProof)
                }

                if !signature_proof_recipient.verify(tx_buf) || !signature_proof_sender.verify(tx_buf) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof)
                }
            },
            ProofType::TimeoutResolve => {
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if proof_buf.len() != 0 {
                    warn!("Over-long proof");
                    return Err(TransactionError::InvalidProof)
                }

                if !signature_proof.verify(tx_buf) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof)
                }
            }
        }
        Ok(())
    }

    fn with_balance(&self, balance: Coin) -> Self {
        return HashedTimeLockedContract {
            balance,
            sender: self.sender.clone(),
            recipient: self.recipient.clone(),
            hash_algorithm: self.hash_algorithm,
            hash_root: self.hash_root.clone(),
            hash_count: self.hash_count,
            timeout: self.timeout,
            total_amount: self.total_amount,
        };
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::InvalidForRecipient);
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::InvalidForRecipient);
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        let proof_buf = &mut &transaction.proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;
        match proof_type {
            ProofType::RegularTransfer => {
                // Check that the contract has not expired yet.
                if self.timeout < block_height {
                    warn!("HTLC expired: {} < {}", self.timeout, block_height);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the provided hash_root is correct.
                let hash_algorithm: HashAlgorithm = Deserialize::deserialize(proof_buf)?;
                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;
                let hash_root: AnyHash = Deserialize::deserialize(proof_buf)?;
                if hash_algorithm != self.hash_algorithm || hash_root != self.hash_root {
                    warn!("HTLC hash mismatch");
                    return Err(AccountError::InvalidForSender);
                }

                // Ignore pre_image.
                let pre_image: AnyHash = Deserialize::deserialize(proof_buf)?;

                // Check that the transaction is signed by the authorized recipient.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                if !signature_proof.is_signed_by(&self.recipient) {
                    return Err(AccountError::InvalidSignature);
                }

                // Check min cap.
                let cap_ratio = 1f64 - (hash_depth as f64 / self.hash_count as f64);
                let min_cap = (cap_ratio * u64::from(self.total_amount) as f64).floor().max(0f64) as u64;
                if balance < Coin::from(min_cap) {
                    return Err(AccountError::InsufficientFunds);
                }
            },
            ProofType::EarlyResolve => {
                // Check that the transaction is signed by both parties.
                let signature_proof_recipient: SignatureProof = Deserialize::deserialize(proof_buf)?;
                let signature_proof_sender: SignatureProof = Deserialize::deserialize(proof_buf)?;
                if !signature_proof_recipient.is_signed_by(&self.recipient)
                        || !signature_proof_sender.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            },
            ProofType::TimeoutResolve => {
                // Check that the contract has expired.
                if self.timeout >= block_height {
                    warn!("HTLC not yet expired: {} >= {}", self.timeout, block_height);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the original sender.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                if !signature_proof.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            }
        }
        Ok(self.with_balance(balance))
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(self.with_balance(balance));
    }
}
