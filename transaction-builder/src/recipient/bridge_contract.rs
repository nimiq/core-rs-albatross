use nimiq_keys::Address;
use nimiq_transaction::account::bridge_contract::{
    ChainConfig, CreationTransactionData as BridgeCreationData,
};
use thiserror::Error;

use crate::recipient::Recipient;

/// Building a bridge recipient can fail if mandatory fields are not set.
#[derive(Debug, Error)]
pub enum BridgeRecipientBuilderError {
    #[error("The bridge owner address is missing.")]
    NoOwner,
    #[error("The oracle contract address is missing.")]
    NoOracleAddress,
    #[error("The source chain ID is missing.")]
    NoSourceChainId,
    #[error("The chain configuration is missing.")]
    NoChainConfig,
}

/// A `BridgeRecipientBuilder` can be used to create new bridge contracts.
/// A bridge contract manages cross-chain asset transfers by validating Merkle proofs
/// against oracle-verified state hashes.
#[derive(Default)]
pub struct BridgeRecipientBuilder {
    owner: Option<Address>,
    oracle_address: Option<Address>,
    source_chain_id: Option<u32>,
    chain_config: Option<ChainConfig>,
}

impl BridgeRecipientBuilder {
    /// Creates a new bridge contract builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the `owner` of the bridge contract.
    pub fn with_owner(&mut self, owner: Address) -> &mut Self {
        self.owner = Some(owner);
        self
    }

    /// Sets the `oracle_address` for state verification.
    pub fn with_oracle_address(&mut self, oracle_address: Address) -> &mut Self {
        self.oracle_address = Some(oracle_address);
        self
    }

    /// Sets the `source_chain_id` this bridge instance supports.
    pub fn with_source_chain_id(&mut self, source_chain_id: u32) -> &mut Self {
        self.source_chain_id = Some(source_chain_id);
        self
    }

    /// Sets the `chain_config` for the source chain.
    pub fn with_chain_config(&mut self, chain_config: ChainConfig) -> &mut Self {
        self.chain_config = Some(chain_config);
        self
    }

    /// Generates the bridge contract recipient.
    pub fn generate(self) -> Result<Recipient, BridgeRecipientBuilderError> {
        Ok(Recipient::BridgeCreation {
            data: BridgeCreationData {
                owner: self.owner.ok_or(BridgeRecipientBuilderError::NoOwner)?,
                oracle_address: self
                    .oracle_address
                    .ok_or(BridgeRecipientBuilderError::NoOracleAddress)?,
                source_chain_id: self
                    .source_chain_id
                    .ok_or(BridgeRecipientBuilderError::NoSourceChainId)?,
                chain_config: self
                    .chain_config
                    .ok_or(BridgeRecipientBuilderError::NoChainConfig)?,
            },
        })
    }
}
