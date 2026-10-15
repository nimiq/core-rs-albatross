use nimiq_keys::Address;
use nimiq_transaction::account::{
    htlc_contract::HashType, oracle_contract::CreationTransactionData as OracleCreationData,
};
use thiserror::Error;

use crate::recipient::Recipient;

/// Building an oracle recipient can fail if mandatory fields are not set.
#[derive(Debug, Error)]
pub enum OracleRecipientBuilderError {
    #[error("The oracle owner address is missing.")]
    NoOwner,
    #[error("The hash count is missing.")]
    NoHashCount,
    #[error("The hash count must be greater than zero.")]
    InvalidHashCount,
    #[error("The hash algorithm is missing.")]
    NoHashAlgorithm,
}

/// An `OracleRecipientBuilder` can be used to create new oracle contracts.
/// An oracle contract is essentially a hash storage with a fixed-size ring buffer.
#[derive(Default)]
pub struct OracleRecipientBuilder {
    owner: Option<Address>,
    hash_count: Option<u16>,
    hash_algorithm: Option<HashType>,
}

impl OracleRecipientBuilder {
    /// Creates a new oracle contract builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Creates a new oracle contract builder with the owner set.
    pub fn with_owner_init(owner: Address) -> Self {
        let mut builder = Self::new();
        builder.with_owner(owner);
        builder
    }

    /// Sets the `owner` of the oracle contract.
    pub fn with_owner(&mut self, owner: Address) -> &mut Self {
        self.owner = Some(owner);
        self
    }

    /// Sets the `hash_count` (ring buffer size) for the oracle contract.
    pub fn with_hash_count(&mut self, hash_count: u16) -> &mut Self {
        self.hash_count = Some(hash_count);
        self
    }

    /// Sets the `hash_algorithm` of every value the oracle contract will store.
    pub fn with_hash_algorithm(&mut self, hash_algorithm: HashType) -> &mut Self {
        self.hash_algorithm = Some(hash_algorithm);
        self
    }

    /// Generates the oracle contract recipient.
    pub fn generate(self) -> Result<Recipient, OracleRecipientBuilderError> {
        let hash_count = self
            .hash_count
            .ok_or(OracleRecipientBuilderError::NoHashCount)?;

        if hash_count == 0 {
            return Err(OracleRecipientBuilderError::InvalidHashCount);
        }

        Ok(Recipient::OracleCreation {
            data: OracleCreationData {
                owner: self.owner.ok_or(OracleRecipientBuilderError::NoOwner)?,
                hash_count,
                hash_algorithm: self
                    .hash_algorithm
                    .ok_or(OracleRecipientBuilderError::NoHashAlgorithm)?,
            },
        })
    }
}
