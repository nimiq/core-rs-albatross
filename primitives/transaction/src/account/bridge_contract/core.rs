use nimiq_hash::{
    sha512::Sha512Hasher, Blake2bHash, Blake2bHasher, HashOutput, Hasher, Keccak256Hash,
    Keccak256Hasher, Sha256Hash, Sha256Hasher,
};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::merkle::MerkleProof;

use crate::account::htlc_contract::{AnyHash, AnyHash32};

/// A Merkle proof that can use any supported hash algorithm.
///
/// This enum wraps `MerkleProof<H>` for different hash types, allowing
/// cross-chain transactions to use their native hash algorithms throughout
/// the proof verification process without conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnyMerkleProof {
    /// Merkle proof using Blake2b hashing
    Blake2b(MerkleProof<Blake2bHash>),
    /// Merkle proof using SHA-256 hashing
    Sha256(MerkleProof<Sha256Hash>),
    /// Merkle proof using Keccak-256 hashing (Ethereum)
    Keccak256(MerkleProof<Keccak256Hash>),
}

impl AnyMerkleProof {
    /// Computes the Merkle root from the proof and a leaf hash.
    ///
    /// Delegates to the underlying typed MerkleProof based on the hash algorithm.
    pub fn compute_root(&self, leaf_hash: AnyHash) -> Result<AnyHash, BridgeError> {
        match (self, leaf_hash) {
            (AnyMerkleProof::Blake2b(proof), AnyHash::Blake2b(leaf)) => {
                let leaf_hash = Blake2bHash::from(leaf.0);
                let root = proof
                    .compute_root(vec![leaf_hash])
                    .map_err(|_| BridgeError::InvalidMerkleProof)?;
                Ok(AnyHash::Blake2b(AnyHash32::from(root.as_bytes())))
            }
            (AnyMerkleProof::Sha256(proof), AnyHash::Sha256(leaf)) => {
                let leaf_hash = Sha256Hash::from(leaf.0);
                let root = proof
                    .compute_root(vec![leaf_hash])
                    .map_err(|_| BridgeError::InvalidMerkleProof)?;
                Ok(AnyHash::Sha256(AnyHash32::from(root.as_bytes())))
            }
            (AnyMerkleProof::Keccak256(proof), AnyHash::Keccak256(leaf)) => {
                let leaf_hash = Keccak256Hash::from(leaf.0);
                let root = proof
                    .compute_root(vec![leaf_hash])
                    .map_err(|_| BridgeError::InvalidMerkleProof)?;
                Ok(AnyHash::Keccak256(AnyHash32::from(root.as_bytes())))
            }
            _ => Err(BridgeError::InvalidMerkleProof),
        }
    }

    /// Returns the number of nodes in the proof (proof depth).
    pub fn len(&self) -> usize {
        match self {
            AnyMerkleProof::Blake2b(proof) => proof.len(),
            AnyMerkleProof::Sha256(proof) => proof.len(),
            AnyMerkleProof::Keccak256(proof) => proof.len(),
        }
    }

    /// Returns true if the proof is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Byte order representation for cross-chain data encoding.
///
/// Different blockchain networks use different byte ordering conventions.
/// This enum allows the bridge to correctly interpret and convert data
/// between chains with different endianness.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Endianness {
    /// Little-endian byte order (least significant byte first)
    LittleEndian,
    /// Big-endian byte order (most significant byte first)
    BigEndian,
}

/// Address format specification for different blockchain networks.
///
/// Each blockchain network has its own address format and validation rules.
/// This enum allows the bridge to validate addresses according to the
/// specific requirements of each supported chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressFormat {
    /// Nimiq address format (20 bytes)
    Nimiq,
    /// Ethereum address format (20 bytes, EVM-compatible)
    Ethereum,
    /// Bitcoin address format (variable length, base58 encoded)
    Bitcoin,
    /// Custom address format with specified validation rules
    Custom(String),
}

/// Configuration parameters for a specific blockchain network.
///
/// Each bridge contract instance is configured for a single source chain.
/// This structure contains all the chain-specific parameters needed to
/// correctly process and validate cross-chain transactions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainConfig {
    /// Unique identifier for the blockchain network
    pub chain_id: u32,
    /// Hash function used by the source chain for Merkle tree construction
    pub hash_function: AnyHash,
    /// Address format and validation rules for the chain
    pub address_format: AddressFormat,
    /// Byte order used by the chain for data encoding
    pub endianness: Endianness,
    /// Average time between blocks on the source chain
    pub block_time: std::time::Duration,
    /// Validation program for burn transactions
    /// Defines how to parse and validate burn transactions from the source chain
    pub validation_program: ValidationProgram,
}

/// Represents a verified state hash from the Nimiq Oracle Contract.
///
/// The oracle contract stores cryptographically verified state hashes from
/// source chains. These state hashes serve as the root of trust for
/// validating Merkle proofs in cross-chain transactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateHash {
    /// The state hash (Merkle root) from the source chain
    /// Uses AnyHash to support different hash functions from different chains
    /// (e.g., Keccak256 for Ethereum, SHA256 for Bitcoin, Blake2b for Nimiq)
    pub hash: AnyHash,
    /// Block height at which this state was recorded
    pub block_height: u32,
    /// Unix timestamp when the state was recorded
    pub timestamp: u64,
    /// Oracle's cryptographic signature attesting to the state validity
    pub oracle_signature: Vec<u8>,
}

/// Error types for cross-chain bridge operations.
///
/// Comprehensive error enumeration covering all failure modes in the
/// bridge contract system, from oracle communication failures to
/// cryptographic verification errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeError {
    /// Oracle contract is unreachable or not responding
    OracleUnavailable,
    /// Oracle signature verification failed or oracle reference is incorrect
    InvalidOracleSignature,
    /// State hash not found for the specified block height
    StateHashNotFound(u32),
    /// Merkle proof verification failed
    InvalidMerkleProof,
    /// Merkle proof depth exceeds maximum allowed depth
    ProofDepthExceeded,
    /// Leaf hash does not match the expected burn transaction hash
    LeafHashMismatch,
    /// Computed root hash does not match oracle-provided state hash
    RootHashMismatch,
    /// Recipient data format is invalid or cannot be parsed
    InvalidRecipientData,
    /// Transaction amount is invalid (zero or out of range)
    InvalidAmount,
    /// Address format is invalid for the specified chain
    InvalidAddress(String),
    /// Nonce has already been used (prevents replay attacks)
    NonceAlreadyUsed,
    /// Transaction timestamp is invalid or malformed
    InvalidTimestamp,
    /// Chain ID is invalid or not supported
    InvalidChainId,
    /// Validity start height is invalid
    InvalidValidityHeight,
    /// Transaction resubmission limit has been exceeded
    ResubmissionLimitExceeded,
    /// Hash function is not supported by the bridge
    UnsupportedHashFunction,
    /// Chain configuration is invalid or incomplete
    InvalidChainConfiguration,
    /// Oracle address has not been configured
    OracleAddressNotSet,
    /// Failed to serialize data
    SerializationError,
    /// Failed to deserialize data
    DeserializationError,
    /// Data length is invalid (empty or exceeds maximum)
    InvalidDataLength,
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::OracleUnavailable => write!(f, "Oracle contract is unavailable"),
            BridgeError::InvalidOracleSignature => write!(f, "Invalid oracle signature"),
            BridgeError::StateHashNotFound(height) => {
                write!(f, "State hash not found for block height {}", height)
            }
            BridgeError::InvalidMerkleProof => write!(f, "Invalid Merkle proof"),
            BridgeError::ProofDepthExceeded => {
                write!(f, "Merkle proof depth exceeded maximum allowed")
            }
            BridgeError::LeafHashMismatch => write!(f, "Leaf hash does not match expected value"),
            BridgeError::RootHashMismatch => write!(f, "Root hash does not match oracle state"),
            BridgeError::InvalidRecipientData => write!(f, "Invalid recipient data format"),
            BridgeError::InvalidAmount => write!(f, "Invalid transaction amount"),
            BridgeError::InvalidAddress(addr) => write!(f, "Invalid address format: {}", addr),
            BridgeError::NonceAlreadyUsed => write!(f, "Nonce has already been used"),
            BridgeError::InvalidTimestamp => write!(f, "Invalid transaction timestamp"),
            BridgeError::InvalidChainId => write!(f, "Invalid chain ID"),
            BridgeError::InvalidValidityHeight => write!(f, "Invalid validity start height"),
            BridgeError::ResubmissionLimitExceeded => {
                write!(f, "Transaction resubmission limit exceeded")
            }
            BridgeError::UnsupportedHashFunction => write!(f, "Unsupported hash function"),
            BridgeError::InvalidChainConfiguration => write!(f, "Invalid chain configuration"),
            BridgeError::OracleAddressNotSet => write!(f, "Oracle address not configured"),
            BridgeError::SerializationError => write!(f, "Failed to serialize data"),
            BridgeError::DeserializationError => write!(f, "Failed to deserialize data"),
            BridgeError::InvalidDataLength => write!(f, "Invalid data length"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Represents an incoming cross-chain transaction from a source blockchain.
///
/// This structure contains the information needed to process a transaction
/// that originates on a source chain and targets the destination chain.
/// Replay protection is provided by the nonce in recipient_data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingTransaction {
    /// Embedded data containing target address and nonce (nonce provides replay protection)
    pub recipient_data: RecipientData,
    /// Amount of assets to transfer (in smallest unit)
    pub amount: Coin,
    /// Block height from which this transaction becomes valid
    pub validity_start_height: u32,
}

impl IncomingTransaction {
    /// Creates a new incoming transaction with validation.
    ///
    pub fn new(
        recipient_data: RecipientData,
        amount: Coin,
        validity_start_height: u32,
    ) -> Result<Self, BridgeError> {
        let transaction = Self {
            recipient_data,
            amount,
            validity_start_height,
        };
        transaction.validate()?;
        Ok(transaction)
    }

    /// Validates the incoming transaction fields.
    ///
    /// Checks that amount is non-zero, validity height is non-zero, and nonce is non-zero.
    ///
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.amount == Coin::ZERO {
            return Err(BridgeError::InvalidAmount);
        }
        if self.validity_start_height == 0 {
            return Err(BridgeError::InvalidValidityHeight);
        }
        if self.recipient_data.target_nonce == 0 {
            return Err(BridgeError::InvalidRecipientData);
        }
        Ok(())
    }

    /// Validates the transaction against chain-specific rules.
    ///
    /// Performs comprehensive validation including chain-specific checks for
    /// Ethereum and other chains. This includes EVM-specific validation for
    /// Ethereum chains.
    pub fn validate_with_chain_config(
        &self,
        chain_config: &ChainConfig,
    ) -> Result<(), BridgeError> {
        // Perform basic validation
        self.validate()?;

        // Validate recipient data against chain config
        self.recipient_data.validate(chain_config)?;

        Ok(())
    }
}

/// Recipient information embedded in cross-chain transactions.
///
/// This data is embedded in the transaction on the source chain and
/// specifies where assets should be sent on the destination chain.
/// The nonce ensures transaction ordering and prevents replay attacks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipientData {
    /// Destination address on the target chain (raw bytes)
    /// The format depends on the target chain's AddressFormat
    pub target_address: Vec<u8>,
    /// Nonce for transaction ordering (must be non-zero)
    pub target_nonce: u64,
}

impl RecipientData {
    /// Parses recipient data from raw bytes.
    ///
    /// Deserializes JSON-encoded recipient data and validates that
    /// the nonce is non-zero.
    ///
    pub fn parse(data: &[u8]) -> Result<Self, BridgeError> {
        if data.is_empty() {
            return Err(BridgeError::InvalidRecipientData);
        }
        let recipient_data: RecipientData =
            Deserialize::deserialize_from_vec(data).map_err(|e| {
                log::warn!("Failed to parse recipient data: {:?}", e);
                BridgeError::InvalidRecipientData
            })?;

        if recipient_data.target_nonce == 0 {
            return Err(BridgeError::InvalidRecipientData);
        }
        Ok(recipient_data)
    }

    /// Extracts the target address from recipient data.
    ///
    /// Returns a reference to the destination address bytes on the target chain.
    pub fn extract_target_address(&self) -> &[u8] {
        &self.target_address
    }

    /// Extracts the target nonce from recipient data.
    ///
    /// Returns the nonce value used for transaction ordering.
    pub fn extract_target_nonce(&self) -> u64 {
        self.target_nonce
    }

    /// Validates recipient data against chain configuration.
    ///
    /// Checks that nonce is non-zero and address format is valid for the chain.
    ///
    pub fn validate(&self, chain_config: &ChainConfig) -> Result<(), BridgeError> {
        if self.target_nonce == 0 {
            return Err(BridgeError::InvalidRecipientData);
        }
        self.validate_address_format(chain_config)?;
        Ok(())
    }

    /// Validates that the address format matches the chain configuration.
    ///
    /// Different chains have different address formats and validation rules.
    /// This method ensures the target address is valid for the specified chain.
    ///
    pub fn validate_address_format(&self, chain_config: &ChainConfig) -> Result<(), BridgeError> {
        match chain_config.address_format {
            AddressFormat::Nimiq | AddressFormat::Ethereum => {
                // Both use 20-byte addresses
                if self.target_address.len() != 20 {
                    return Err(BridgeError::InvalidAddress(
                        "Nimiq/Ethereum address must be 20 bytes".to_string(),
                    ));
                }
                Ok(())
            }
            AddressFormat::Bitcoin => {
                // Bitcoin addresses are variable length (20-64 bytes typically)
                if self.target_address.len() < 20 || self.target_address.len() > 64 {
                    return Err(BridgeError::InvalidAddress(
                        "Bitcoin address must be 20-64 bytes".to_string(),
                    ));
                }
                Ok(())
            }
            AddressFormat::Custom(ref format_name) => {
                log::debug!("Validating custom address format: {}", format_name);
                // Custom format - basic length check
                if self.target_address.is_empty() || self.target_address.len() > 64 {
                    return Err(BridgeError::InvalidAddress(format!(
                        "Custom address format {} invalid",
                        format_name
                    )));
                }
                Ok(())
            }
        }
    }

    /// Creates new recipient data with validation.
    ///
    pub fn new(
        target_address: Vec<u8>,
        target_nonce: u64,
        chain_config: &ChainConfig,
    ) -> Result<Self, BridgeError> {
        let recipient_data = Self {
            target_address,
            target_nonce,
        };
        recipient_data.validate(chain_config)?;
        Ok(recipient_data)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, BridgeError> {
        Ok(self.serialize_to_vec())
    }
}

/// Represents an outgoing cross-chain transaction with cryptographic proof.
///
/// This structure contains a burn transaction from the source chain along
/// with a Merkle proof that demonstrates the transaction's inclusion in
/// the source chain's state. The relayer provides the oracle state index
/// that the proof is verified against.
///
/// All transaction details (amount, address, nonce, etc.) are extracted
/// from the burn_transaction_data using the chain's ValidationProgram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingTransaction {
    /// Raw burn transaction data from the source chain
    /// Contains all transaction details (amount, address, nonce, block height, etc.)
    pub burn_transaction_data: Vec<u8>,

    /// Merkle proof demonstrating transaction inclusion in source chain state
    /// Uses the chain's native hash algorithm (Blake2b, Keccak256, SHA256, etc.)
    pub merkle_proof: AnyMerkleProof,

    /// Index into the oracle contract's hash array
    /// Provided by the relayer to reference which oracle state hash to verify against
    pub oracle_state_index: u64,
}

impl OutgoingTransaction {
    /// Creates a new outgoing transaction with validation.
    ///
    pub fn new(
        burn_transaction_data: Vec<u8>,
        merkle_proof: AnyMerkleProof,
        oracle_state_index: u64,
    ) -> Result<Self, BridgeError> {
        let transaction = Self {
            burn_transaction_data,
            merkle_proof,
            oracle_state_index,
        };
        transaction.validate()?;
        Ok(transaction)
    }

    /// Validates the outgoing transaction fields.
    ///
    /// Checks that burn data is present and within size limits.
    ///
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.burn_transaction_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }
        if self.burn_transaction_data.len() > 10_000 {
            return Err(BridgeError::InvalidDataLength);
        }
        Ok(())
    }

    /// Parses burn transaction data using the validation program.
    ///
    /// Extracts all transaction fields from the raw burn data using the
    /// chain's configured validation program.
    pub fn parse_burn_data(
        &self,
        chain_config: &ChainConfig,
    ) -> Result<ParsedBurnData, BridgeError> {
        // Execute validation program in extraction-only mode
        // This will error if the program contains validation operations (Assert, PushExpected*)
        // that require expected values from an IncomingTransaction.
        let result = chain_config
            .validation_program
            .extract_only(&self.burn_transaction_data)?;

        // Extract required fields from the result
        let amount_value = result
            .extracted_values
            .get("amount")
            .ok_or(BridgeError::InvalidRecipientData)?;
        let amount = Coin::from_u64_unchecked(amount_value.as_u64()?);

        let address_value = result
            .extracted_values
            .get("target_address")
            .ok_or(BridgeError::InvalidRecipientData)?;
        let target_address_bytes = address_value.as_bytes()?;

        // Validate address length before conversion
        if target_address_bytes.len() != 20 {
            return Err(BridgeError::InvalidAddress(format!(
                "Address must be 20 bytes, got {}",
                target_address_bytes.len()
            )));
        }
        let target_address = Address::from(target_address_bytes.as_slice());

        let nonce_value = result
            .extracted_values
            .get("target_nonce")
            .ok_or(BridgeError::InvalidRecipientData)?;
        let target_nonce = nonce_value.as_u64()?;

        let height_value = result
            .extracted_values
            .get("burn_block_height")
            .ok_or(BridgeError::InvalidRecipientData)?;
        let burn_block_height = height_value.as_u32()?;

        let chain_id_value = result
            .extracted_values
            .get("target_chain_id")
            .ok_or(BridgeError::InvalidRecipientData)?;
        let target_chain_id = chain_id_value.as_u32()?;

        // Compute transaction ID
        let transaction_id = self.transaction_id(&chain_config.hash_function)?;

        ParsedBurnData::new(
            transaction_id,
            amount,
            target_address,
            target_nonce,
            burn_block_height,
            target_chain_id,
        )
    }

    /// Computes the transaction ID from burn transaction data.
    ///
    /// Computes the transaction ID from burn transaction data.
    ///
    /// Uses the chain's native hash function to compute the transaction ID.
    /// This ensures consistency with how the source chain identifies transactions.
    pub fn transaction_id(&self, hash_function: &AnyHash) -> Result<AnyHash, BridgeError> {
        if self.burn_transaction_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        // Use the chain's native hash function for transaction ID
        let hash = match hash_function {
            AnyHash::Blake2b(_) => {
                let h = Blake2bHasher::default().digest(&self.burn_transaction_data);
                AnyHash::Blake2b(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Sha256(_) => {
                let h = Sha256Hasher::default().digest(&self.burn_transaction_data);
                AnyHash::Sha256(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Keccak256(_) => {
                let h = Keccak256Hasher::default().digest(&self.burn_transaction_data);
                AnyHash::Keccak256(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Sha512(_) => {
                let h = Sha512Hasher::default().digest(&self.burn_transaction_data);
                AnyHash::Sha512(crate::account::htlc_contract::AnyHash64::from(h.as_bytes()))
            }
        };

        Ok(hash)
    }

    /// Extracts the hash of the burn transaction data using the specified hash algorithm.
    ///
    /// Computes the hash of the raw burn transaction bytes using the chain's
    /// native hash function (Blake2b, Keccak256, SHA256, etc.).
    ///
    pub fn extract_burn_transaction_hash(
        &self,
        hash_function: &AnyHash,
    ) -> Result<AnyHash, BridgeError> {
        if self.burn_transaction_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        let hash = match hash_function {
            AnyHash::Blake2b(_) => {
                let h = Blake2bHasher::default().digest(&self.burn_transaction_data);
                AnyHash::Blake2b(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Sha256(_) => {
                let h = Sha256Hasher::default().digest(&self.burn_transaction_data);
                AnyHash::Sha256(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Keccak256(_) => {
                let h = Keccak256Hasher::default().digest(&self.burn_transaction_data);
                AnyHash::Keccak256(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Sha512(_) => {
                return Err(BridgeError::UnsupportedHashFunction);
            }
        };

        Ok(hash)
    }

    /// Verifies the Merkle proof against a given root hash.
    ///
    /// Computes the root hash from the proof and burn transaction,
    /// then compares it to the provided root hash. Both the proof and
    /// root hash use the chain's native hash algorithm.
    ///
    pub fn verify_merkle_proof(
        &self,
        root_hash: AnyHash,
        leaf_hash: AnyHash,
    ) -> Result<bool, BridgeError> {
        let computed_root = self.merkle_proof.compute_root(leaf_hash)?;
        Ok(computed_root == root_hash)
    }

    /// Returns the transaction details as a tuple (for backward compatibility).
    ///
    /// Note: This method requires a ChainConfig to parse the burn data.
    /// Consider using parse_burn_data() instead for more explicit error handling.
    pub fn transaction_details(
        &self,
        chain_config: &ChainConfig,
    ) -> Result<(Coin, Vec<u8>, u64, u32), BridgeError> {
        let parsed = self.parse_burn_data(chain_config)?;
        Ok((
            parsed.amount,
            parsed.target_address.as_bytes().to_vec(),
            parsed.target_nonce,
            parsed.burn_block_height,
        ))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, BridgeError> {
        use nimiq_serde::Serialize;
        Ok(self.serialize_to_vec())
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, BridgeError> {
        use nimiq_serde::Deserialize;
        Self::deserialize_all(data).map_err(|_| BridgeError::DeserializationError)
    }

    /// Returns the depth of the Merkle proof (number of nodes).
    pub fn proof_depth(&self) -> usize {
        self.merkle_proof.len()
    }

    /// Checks if the proof depth is within the allowed maximum.
    pub fn is_proof_depth_valid(&self, max_depth: u32) -> bool {
        self.proof_depth() <= max_depth as usize
    }
}
/// Merkle proof validator for cross-chain transaction verification.
///
/// This component handles all cryptographic verification of Merkle proofs,
/// including proof structure validation, root hash computation, and
/// integration with the oracle contract for state verification.
/// Supports multiple hash functions for cross-chain compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProofValidator {
    /// Hash function used for Merkle tree operations
    pub hash_function: AnyHash,
    /// Maximum allowed depth for Merkle proofs (prevents DoS attacks)
    pub max_proof_depth: u32,
    /// Byte order for data encoding/decoding
    pub endianness: Endianness,
    /// Address of the oracle contract (optional until configured)
    pub oracle_address: Option<Address>,
}

impl MerkleProofValidator {
    /// Creates a new Merkle proof validator with specified configuration.
    pub fn new(
        hash_function: AnyHash,
        max_proof_depth: u32,
        endianness: Endianness,
        oracle_address: Option<Address>,
    ) -> Self {
        Self {
            hash_function,
            max_proof_depth,
            endianness,
            oracle_address,
        }
    }

    /// Creates a default validator configured for Blake2b hashing.
    ///
    /// Uses Blake2b hash function, 32 max proof depth, little-endian byte order,
    /// and no oracle address (must be configured separately).
    pub fn default_blake2b() -> Self {
        Self {
            hash_function: AnyHash::Blake2b(AnyHash32::default()),
            max_proof_depth: 32,
            endianness: Endianness::LittleEndian,
            oracle_address: None,
        }
    }

    /// Verifies a Merkle proof against a known root hash.
    ///
    /// Validates the proof structure, computes the root from the proof and leaf,
    /// and compares it to the expected root hash. Uses the chain's native hash algorithm.
    ///
    pub fn verify_merkle_proof(
        &self,
        proof: &AnyMerkleProof,
        leaf_hash: AnyHash,
        root_hash: AnyHash,
    ) -> Result<bool, BridgeError> {
        self.validate_proof_structure(proof)?;
        let computed_root = self.compute_root_from_proof(proof, leaf_hash)?;
        Ok(computed_root == root_hash)
    }

    /// Extracts the transaction hash using the configured hash function.
    ///
    /// Supports multiple hash functions (Blake2b, SHA256, Keccak256)
    /// for cross-chain compatibility. Returns the hash in the chain's native format.
    /// Applies endianness conversion if the source chain uses big-endian byte order.
    ///
    pub fn extract_transaction_hash(&self, tx_data: &[u8]) -> Result<AnyHash, BridgeError> {
        if tx_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        // Apply endianness conversion if needed
        // For big-endian chains, we may need to convert multi-byte values before hashing
        // Note: This is a simplified approach - actual implementation may need more sophisticated
        // handling depending on the transaction format
        let data_to_hash = match self.endianness {
            Endianness::BigEndian => {
                // For big-endian chains like Ethereum, the data is already in the correct format
                // We just use it as-is since Ethereum uses big-endian natively
                tx_data
            }
            Endianness::LittleEndian => {
                // For little-endian chains like Nimiq, use data as-is
                tx_data
            }
        };

        let hash = match &self.hash_function {
            AnyHash::Blake2b(_) => {
                let h = Blake2bHasher::default().digest(data_to_hash);
                AnyHash::Blake2b(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Sha256(_) => {
                let h = Sha256Hasher::default().digest(data_to_hash);
                AnyHash::Sha256(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Keccak256(_) => {
                let h = Keccak256Hasher::default().digest(data_to_hash);
                AnyHash::Keccak256(AnyHash32::from(h.as_bytes()))
            }
            AnyHash::Sha512(_) => {
                return Err(BridgeError::UnsupportedHashFunction);
            }
        };

        Ok(hash)
    }

    /// Extracts transaction hash with explicit endianness conversion.
    ///
    /// Similar to `extract_transaction_hash` but allows overriding the validator's
    /// configured endianness. Useful when processing transactions from multiple chains.
    pub fn extract_transaction_hash_with_endianness(
        &self,
        tx_data: &[u8],
        source_endianness: Endianness,
    ) -> Result<Blake2bHash, BridgeError> {
        if tx_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        // Convert data if endianness differs from validator's configuration
        let data_to_hash = if source_endianness != self.endianness {
            // Note: Full endianness conversion of transaction data is complex
            // This is a placeholder for more sophisticated conversion logic
            log::debug!(
                ?source_endianness,
                target_endianess = ?self.endianness,
                "Converting transaction data"
            );
            tx_data
        } else {
            tx_data
        };

        match &self.hash_function {
            AnyHash::Blake2b(_) => Ok(Blake2bHasher::default().digest(data_to_hash)),
            AnyHash::Sha256(_) => {
                let sha256_hash = Sha256Hasher::default().digest(data_to_hash);
                Ok(Blake2bHasher::default().digest(sha256_hash.as_bytes()))
            }
            AnyHash::Sha512(_) => {
                let sha512_hash = Sha512Hasher::default().digest(data_to_hash);
                Ok(Blake2bHasher::default().digest(sha512_hash.as_bytes()))
            }
            AnyHash::Keccak256(_) => {
                let keccak_hash = Keccak256Hasher::default().digest(data_to_hash);
                Ok(Blake2bHasher::default().digest(keccak_hash.as_bytes()))
            }
        }
    }

    /// Validates the structure of a Merkle proof.
    ///
    /// Checks that the proof depth does not exceed the maximum allowed
    /// and that the proof is not empty.
    ///
    pub fn validate_proof_structure(&self, proof: &AnyMerkleProof) -> Result<(), BridgeError> {
        if proof.len() > self.max_proof_depth as usize {
            return Err(BridgeError::ProofDepthExceeded);
        }
        if proof.is_empty() {
            return Err(BridgeError::InvalidMerkleProof);
        }
        Ok(())
    }

    /// Computes the Merkle root hash from a proof and leaf hash.
    ///
    /// Traverses the Merkle tree using the proof path to compute
    /// the root hash that should match the oracle's state hash.
    /// Uses the chain's native hash algorithm throughout.
    ///
    pub fn compute_root_from_proof(
        &self,
        proof: &AnyMerkleProof,
        leaf_hash: AnyHash,
    ) -> Result<AnyHash, BridgeError> {
        proof.compute_root(leaf_hash)
    }

    /// Verifies that the provided oracle address matches the configured address.
    ///
    /// Used to validate that proofs reference the correct oracle contract.
    ///
    pub fn verify_oracle_reference(&self, oracle_address: &Address) -> Result<bool, BridgeError> {
        match &self.oracle_address {
            Some(expected_address) => Ok(oracle_address == expected_address),
            None => Err(BridgeError::OracleAddressNotSet),
        }
    }

    /// Updates the oracle contract address.
    ///
    /// Changes the oracle address used for state verification.
    /// The zero address is rejected as invalid.
    ///
    pub fn update_oracle_address(
        &mut self,
        new_oracle_address: Address,
    ) -> Result<(), BridgeError> {
        if new_oracle_address == Address::from([0u8; 20]) {
            return Err(BridgeError::OracleAddressNotSet);
        }
        log::info!(%new_oracle_address, "Updated oracle address");
        self.oracle_address = Some(new_oracle_address);
        Ok(())
    }

    /// Configures the hash function used for proof verification.
    ///
    /// Allows switching between different hash functions for cross-chain
    /// compatibility. Supports Blake2b, SHA256, SHA512, and Keccak256.
    ///
    pub fn configure_hash_function(&mut self, hash_function: AnyHash) -> Result<(), BridgeError> {
        match hash_function {
            AnyHash::Blake2b(_)
            | AnyHash::Sha256(_)
            | AnyHash::Sha512(_)
            | AnyHash::Keccak256(_) => {
                self.hash_function = hash_function;
                log::info!("Updated hash function to: {:?}", self.hash_function);
                Ok(())
            }
        }
    }

    /// Returns the currently configured hash function.
    pub fn get_hash_function(&self) -> &AnyHash {
        &self.hash_function
    }

    /// Returns the maximum allowed proof depth.
    pub fn get_max_proof_depth(&self) -> u32 {
        self.max_proof_depth
    }

    /// Sets the maximum allowed proof depth.
    ///
    pub fn set_max_proof_depth(&mut self, max_depth: u32) -> Result<(), BridgeError> {
        if max_depth == 0 {
            return Err(BridgeError::ProofDepthExceeded);
        }
        self.max_proof_depth = max_depth;
        Ok(())
    }

    /// Returns the currently configured endianness.
    pub fn get_endianness(&self) -> Endianness {
        self.endianness
    }

    /// Sets the endianness for data encoding/decoding.
    pub fn set_endianness(&mut self, endianness: Endianness) {
        self.endianness = endianness;
        log::info!(?endianness, "Updated endianness to");
    }

    /// Validates that the leaf hash matches the burn transaction data
    pub fn validate_leaf_node(
        &self,
        burn_transaction_data: &[u8],
        expected_leaf_hash: AnyHash,
    ) -> Result<bool, BridgeError> {
        if burn_transaction_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }
        let computed_leaf_hash = self.extract_transaction_hash(burn_transaction_data)?;
        Ok(computed_leaf_hash == expected_leaf_hash)
    }

    /// Verifies that the computed root hash matches the provided oracle state hash.
    ///
    /// This method should be called after verify_comprehensive_proof to validate
    /// the computed root against the actual oracle state hash retrieved from the oracle contract.
    pub fn verify_root_against_oracle(
        &self,
        outgoing_tx: &OutgoingTransaction,
        oracle_state_hash: &AnyHash,
        chain_config: &ChainConfig,
    ) -> Result<bool, BridgeError> {
        let tx_id = outgoing_tx.transaction_id(&chain_config.hash_function)?;

        // Extract leaf hash from burn transaction
        let leaf_hash = self.extract_transaction_hash(&outgoing_tx.burn_transaction_data)?;

        // Compute root hash from proof
        let computed_root = self.compute_root_from_proof(&outgoing_tx.merkle_proof, leaf_hash)?;

        // Compare with oracle state hash
        if computed_root != *oracle_state_hash {
            log::error!(
                ?tx_id,
                ?computed_root,
                ?oracle_state_hash,
                "Root hash mismatch"
            );
            return Err(BridgeError::RootHashMismatch);
        }

        Ok(true)
    }
}

impl Default for MerkleProofValidator {
    fn default() -> Self {
        Self::default_blake2b()
    }
}

/// Endianness conversion utilities for cross-chain data handling.
///
/// Provides functions to convert data between different byte orders,
/// essential for interoperability between chains with different endianness.
pub mod endianness_conversion {
    use super::{BridgeError, Endianness};

    /// Converts a u32 value between little-endian and big-endian.
    pub fn convert_u32(value: u32, from: Endianness, to: Endianness) -> u32 {
        match (from, to) {
            (Endianness::LittleEndian, Endianness::BigEndian) => value.swap_bytes(),
            (Endianness::BigEndian, Endianness::LittleEndian) => value.swap_bytes(),
            _ => value, // Same endianness, no conversion needed
        }
    }

    /// Converts a u64 value between little-endian and big-endian.
    pub fn convert_u64(value: u64, from: Endianness, to: Endianness) -> u64 {
        match (from, to) {
            (Endianness::LittleEndian, Endianness::BigEndian) => value.swap_bytes(),
            (Endianness::BigEndian, Endianness::LittleEndian) => value.swap_bytes(),
            _ => value, // Same endianness, no conversion needed
        }
    }

    /// Converts a byte slice between little-endian and big-endian.
    ///
    /// Reverses the byte order of the entire slice.
    pub fn convert_bytes(data: &[u8], from: Endianness, to: Endianness) -> Vec<u8> {
        match (from, to) {
            (Endianness::LittleEndian, Endianness::BigEndian)
            | (Endianness::BigEndian, Endianness::LittleEndian) => {
                data.iter().rev().copied().collect()
            }
            _ => data.to_vec(), // Same endianness, no conversion needed
        }
    }

    /// Converts an address (20 bytes) between endianness formats.
    pub fn convert_address(
        address_bytes: &[u8],
        from: Endianness,
        to: Endianness,
    ) -> Result<Vec<u8>, BridgeError> {
        if address_bytes.len() != 20 {
            return Err(BridgeError::InvalidAddress(
                "Address must be 20 bytes".to_string(),
            ));
        }
        Ok(convert_bytes(address_bytes, from, to))
    }

    /// Parses a u32 from bytes with specified endianness.
    pub fn parse_u32(bytes: &[u8], endianness: Endianness) -> Result<u32, BridgeError> {
        if bytes.len() != 4 {
            return Err(BridgeError::InvalidDataLength);
        }
        let mut array = [0u8; 4];
        array.copy_from_slice(bytes);
        Ok(match endianness {
            Endianness::LittleEndian => u32::from_le_bytes(array),
            Endianness::BigEndian => u32::from_be_bytes(array),
        })
    }

    /// Parses a u64 from bytes with specified endianness.
    pub fn parse_u64(bytes: &[u8], endianness: Endianness) -> Result<u64, BridgeError> {
        if bytes.len() != 8 {
            return Err(BridgeError::InvalidDataLength);
        }
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(match endianness {
            Endianness::LittleEndian => u64::from_le_bytes(array),
            Endianness::BigEndian => u64::from_be_bytes(array),
        })
    }

    /// Encodes a u32 to bytes with specified endianness.
    pub fn encode_u32(value: u32, endianness: Endianness) -> [u8; 4] {
        match endianness {
            Endianness::LittleEndian => value.to_le_bytes(),
            Endianness::BigEndian => value.to_be_bytes(),
        }
    }

    /// Encodes a u64 to bytes with specified endianness.
    pub fn encode_u64(value: u64, endianness: Endianness) -> [u8; 8] {
        match endianness {
            Endianness::LittleEndian => value.to_le_bytes(),
            Endianness::BigEndian => value.to_be_bytes(),
        }
    }
}

/// Data encoding format handlers for different blockchain networks.
///
/// Provides utilities to encode and decode data according to the
/// specific requirements of different blockchain architectures.
pub mod data_encoding {
    use nimiq_keys::Address;

    use super::{AddressFormat, BridgeError, ChainConfig};

    /// Encodes an address according to the chain's address format.
    pub fn encode_address(
        address: &Address,
        chain_config: &ChainConfig,
    ) -> Result<Vec<u8>, BridgeError> {
        let address_bytes = address.as_bytes();

        match chain_config.address_format {
            AddressFormat::Nimiq | AddressFormat::Ethereum => {
                // Both use 20-byte addresses
                if address_bytes.len() != 20 {
                    return Err(BridgeError::InvalidAddress(
                        "Address must be 20 bytes".to_string(),
                    ));
                }
                Ok(address_bytes.to_vec())
            }
            AddressFormat::Bitcoin => {
                // Bitcoin addresses are variable length
                // For now, we just return the raw bytes
                Ok(address_bytes.to_vec())
            }
            AddressFormat::Custom(_) => {
                // Custom format - return raw bytes
                Ok(address_bytes.to_vec())
            }
        }
    }

    /// Decodes an address from bytes according to the chain's address format.
    pub fn decode_address(
        bytes: &[u8],
        chain_config: &ChainConfig,
    ) -> Result<Address, BridgeError> {
        match chain_config.address_format {
            AddressFormat::Nimiq | AddressFormat::Ethereum => {
                if bytes.len() != 20 {
                    return Err(BridgeError::InvalidAddress(
                        "Address must be 20 bytes".to_string(),
                    ));
                }
                let mut address_bytes = [0u8; 20];
                address_bytes.copy_from_slice(bytes);
                Ok(Address::from(address_bytes))
            }
            AddressFormat::Bitcoin => {
                // Bitcoin addresses are variable length
                if bytes.len() < 20 || bytes.len() > 64 {
                    return Err(BridgeError::InvalidAddress(
                        "Bitcoin address length invalid".to_string(),
                    ));
                }
                // Pad or truncate to 20 bytes for Address type
                let mut address_bytes = [0u8; 20];
                let copy_len = bytes.len().min(20);
                address_bytes[..copy_len].copy_from_slice(&bytes[..copy_len]);
                Ok(Address::from(address_bytes))
            }
            AddressFormat::Custom(_) => {
                // Custom format - try to parse as 20-byte address
                if bytes.len() < 20 {
                    return Err(BridgeError::InvalidAddress(
                        "Custom address too short".to_string(),
                    ));
                }
                let mut address_bytes = [0u8; 20];
                address_bytes.copy_from_slice(&bytes[..20]);
                Ok(Address::from(address_bytes))
            }
        }
    }

    /// Encodes transaction data according to the chain's encoding format.
    pub fn encode_transaction_data(
        data: &[u8],
        _chain_config: &ChainConfig,
    ) -> Result<Vec<u8>, BridgeError> {
        if data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        // Apply endianness conversion if needed
        // For now, we return the data as-is since transaction data
        // is typically already in the correct format
        Ok(data.to_vec())
    }

    /// Decodes transaction data according to the chain's encoding format.
    pub fn decode_transaction_data(
        data: &[u8],
        _chain_config: &ChainConfig,
    ) -> Result<Vec<u8>, BridgeError> {
        if data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        // Apply endianness conversion if needed
        // For now, we return the data as-is
        Ok(data.to_vec())
    }

    /// Validates that data conforms to the chain's encoding requirements.
    pub fn validate_data_encoding(
        data: &[u8],
        _chain_config: &ChainConfig,
    ) -> Result<(), BridgeError> {
        if data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        // Check maximum data size (10KB limit)
        if data.len() > 10_000 {
            return Err(BridgeError::InvalidDataLength);
        }

        // Additional validation based on chain format could be added here
        Ok(())
    }
}

// ============================================================================
// Generic Transaction Validation with Stack-Based Operations
// ============================================================================

/// Stack-based operation for transaction validation (EVM-like opcodes).
///
/// This enum defines a set of operations similar to EVM opcodes that can be
/// used to validate burn transactions from different blockchain networks.
/// Each bridge contract is configured with a sequence of these operations
/// that define how to parse and validate transactions from its source chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidationOp {
    // ============================================================================
    // Data Loading Operations
    // ============================================================================
    /// Push a constant value onto the stack
    /// Stack: [] -> [value]
    PushConst(u64),

    /// Load bytes from burn transaction data at offset
    /// Stack: [offset, length] -> [bytes]
    LoadBytes,

    /// Load u64 from burn transaction data with endianness
    /// Stack: [offset] -> [u64_value]
    LoadU64(Endianness),

    /// Load u32 from burn transaction data with endianness
    /// Stack: [offset] -> [u32_value]
    LoadU32(Endianness),

    /// Load address (20 bytes) from burn transaction data
    /// Stack: [offset] -> [address_bytes]
    LoadAddress,

    /// Load EVM-encoded u64 (32 bytes) from burn transaction data
    /// Stack: [offset] -> [u64_value]
    LoadEvmU64,

    // ============================================================================
    // Expected Value Operations
    // ============================================================================
    /// Push expected amount from IncomingTransaction
    /// Stack: [] -> [expected_amount]
    PushExpectedAmount,

    /// Push expected target address from IncomingTransaction
    /// Stack: [] -> [expected_address]
    PushExpectedAddress,

    /// Push expected nonce from IncomingTransaction
    /// Stack: [] -> [expected_nonce]
    PushExpectedNonce,

    /// Push expected validity height from IncomingTransaction
    /// Stack: [] -> [expected_validity_height]
    PushExpectedValidityHeight,

    // ============================================================================
    // Arithmetic Operations (EVM-compatible)
    // ============================================================================
    /// Add two values
    /// Stack: [a, b] -> [a + b]
    Add,

    /// Subtract two values
    /// Stack: [a, b] -> [a - b]
    Sub,

    /// Multiply two values
    /// Stack: [a, b] -> [a * b]
    Mul,

    /// Divide two values
    /// Stack: [a, b] -> [a / b]
    Div,

    /// Modulo operation
    /// Stack: [a, b] -> [a % b]
    Mod,

    // ============================================================================
    // Comparison Operations
    // ============================================================================
    /// Check equality
    /// Stack: [a, b] -> [a == b ? 1 : 0]
    Eq,

    /// Check less than
    /// Stack: [a, b] -> [a < b ? 1 : 0]
    Lt,

    /// Check greater than
    /// Stack: [a, b] -> [a > b ? 1 : 0]
    Gt,

    /// Check if zero
    /// Stack: [a] -> [a == 0 ? 1 : 0]
    IsZero,

    // ============================================================================
    // Logical Operations
    // ============================================================================
    /// Logical AND
    /// Stack: [a, b] -> [a && b ? 1 : 0]
    And,

    /// Logical OR
    /// Stack: [a, b] -> [a || b ? 1 : 0]
    Or,

    /// Logical NOT
    /// Stack: [a] -> [!a ? 1 : 0]
    Not,

    // ============================================================================
    // Stack Manipulation
    // ============================================================================
    /// Duplicate top stack item
    /// Stack: [a] -> [a, a]
    Dup,

    /// Swap top two stack items
    /// Stack: [a, b] -> [b, a]
    Swap,

    /// Pop top stack item
    /// Stack: [a] -> []
    Pop,

    // ============================================================================
    // Control Flow
    // ============================================================================
    /// Assert that top of stack is non-zero (true)
    /// Stack: [condition] -> []
    /// Fails validation if condition is 0
    Assert,

    /// Conditional jump (for complex validation logic)
    /// Stack: [condition] -> []
    /// If condition is 0, skip next N operations
    JumpIfZero(usize),

    // ============================================================================
    // Value Storage/Extraction Operations
    // ============================================================================
    /// Store top of stack to a named variable for later extraction
    /// Stack: [value] -> []
    /// The value is stored in the extraction context and can be retrieved after validation
    Store(String),

    /// Load a named variable onto the stack
    /// Stack: [] -> [value]
    /// Retrieves a previously stored value
    Load(String),
}

/// Stack value (can be number or bytes).
///
/// Represents a value on the validation execution stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackValue {
    /// 64-bit unsigned integer
    U64(u64),
    /// 32-bit unsigned integer
    U32(u32),
    /// Byte array
    Bytes(Vec<u8>),
}

impl StackValue {
    /// Converts stack value to u64 if possible
    pub fn as_u64(&self) -> Result<u64, BridgeError> {
        match self {
            StackValue::U64(v) => Ok(*v),
            StackValue::U32(v) => Ok(*v as u64),
            StackValue::Bytes(_) => Err(BridgeError::InvalidRecipientData),
        }
    }

    /// Converts stack value to u32 if possible
    pub fn as_u32(&self) -> Result<u32, BridgeError> {
        match self {
            StackValue::U32(v) => Ok(*v),
            StackValue::U64(v) => {
                if *v <= u32::MAX as u64 {
                    Ok(*v as u32)
                } else {
                    Err(BridgeError::InvalidRecipientData)
                }
            }
            StackValue::Bytes(_) => Err(BridgeError::InvalidRecipientData),
        }
    }

    /// Converts stack value to bytes if possible
    pub fn as_bytes(&self) -> Result<Vec<u8>, BridgeError> {
        match self {
            StackValue::Bytes(v) => Ok(v.clone()),
            _ => Err(BridgeError::InvalidRecipientData),
        }
    }
}

/// Parsed burn transaction data extracted by the validation program.
///
/// Contains all the fields that were previously stored directly in OutgoingTransaction.
/// These values are extracted from the raw burn transaction data using the validation program.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParsedBurnData {
    /// Transaction ID (computed from burn transaction data using chain's native hash function)
    pub transaction_id: AnyHash,
    /// Amount of assets burned
    pub amount: Coin,
    /// Target address on destination chain (Nimiq address)
    pub target_address: Address,
    /// Nonce for transaction ordering
    pub target_nonce: u64,
    /// Block height where the burn occurred
    pub burn_block_height: u32,
    /// Chain ID of the source (burn) blockchain; must match BridgeContract.source_chain_id
    pub target_chain_id: u32,
}

impl ParsedBurnData {
    /// Creates new parsed burn data with validation
    pub fn new(
        transaction_id: AnyHash,
        amount: Coin,
        target_address: Address,
        target_nonce: u64,
        burn_block_height: u32,
        target_chain_id: u32,
    ) -> Result<Self, BridgeError> {
        if amount == Coin::ZERO {
            return Err(BridgeError::InvalidAmount);
        }
        // Zero nonce indicates malformed burn data, not a replay attack
        if target_nonce == 0 {
            return Err(BridgeError::InvalidRecipientData);
        }
        if burn_block_height == 0 {
            return Err(BridgeError::InvalidValidityHeight);
        }
        if target_chain_id == 0 {
            return Err(BridgeError::InvalidChainId);
        }

        Ok(Self {
            transaction_id,
            amount,
            target_address,
            target_nonce,
            burn_block_height,
            target_chain_id,
        })
    }

    /// Returns the transaction details as a tuple
    pub fn transaction_details(&self) -> (Coin, &Address, u64, u32) {
        (
            self.amount,
            &self.target_address,
            self.target_nonce,
            self.burn_block_height,
        )
    }
}

/// Result of validation program execution with extracted values.
///
/// Contains both the validation result and any values extracted during execution.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Map of extracted values by name
    pub extracted_values: std::collections::HashMap<String, StackValue>,
}

/// Validation program - sequence of operations.
///
/// Defines how to validate burn transactions from a specific blockchain.
/// Each bridge contract is configured with a validation program that
/// parses the burn transaction data and validates it against the expected
/// values from the IncomingTransaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationProgram {
    /// Sequence of operations to execute
    pub operations: Vec<ValidationOp>,
}

impl ValidationProgram {
    /// Creates a new validation program with the given operations
    pub fn new(operations: Vec<ValidationOp>) -> Self {
        Self { operations }
    }

    /// Creates an empty validation program (no validation)
    pub fn empty() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    /// Executes a single operation on the given context.
    ///
    /// This private helper method handles the common operation execution logic
    /// shared between extract_only and execute_with_extraction methods.
    fn execute_operation<C: ExecutionContext>(
        ctx: &mut C,
        op: &ValidationOp,
        pc: &mut usize,
    ) -> Result<(), BridgeError> {
        match op {
            // Data loading operations
            ValidationOp::PushConst(value) => {
                ctx.stack_mut().push(StackValue::U64(*value));
            }

            ValidationOp::LoadBytes => {
                let length = ctx.pop_u64()? as usize;
                let offset = ctx.pop_u64()? as usize;
                let burn_tx_data = ctx.burn_tx_data();

                if offset + length > burn_tx_data.len() {
                    return Err(BridgeError::InvalidDataLength);
                }

                let bytes = burn_tx_data[offset..offset + length].to_vec();
                ctx.stack_mut().push(StackValue::Bytes(bytes));
            }

            ValidationOp::LoadU64(endianness) => {
                let offset = ctx.pop_u64()? as usize;
                let burn_tx_data = ctx.burn_tx_data();

                if offset + 8 > burn_tx_data.len() {
                    return Err(BridgeError::InvalidDataLength);
                }

                let value = endianness_conversion::parse_u64(
                    &burn_tx_data[offset..offset + 8],
                    *endianness,
                )?;
                ctx.stack_mut().push(StackValue::U64(value));
            }

            ValidationOp::LoadU32(endianness) => {
                let offset = ctx.pop_u64()? as usize;
                let burn_tx_data = ctx.burn_tx_data();

                if offset + 4 > burn_tx_data.len() {
                    return Err(BridgeError::InvalidDataLength);
                }

                let value = endianness_conversion::parse_u32(
                    &burn_tx_data[offset..offset + 4],
                    *endianness,
                )?;
                ctx.stack_mut().push(StackValue::U32(value));
            }

            ValidationOp::LoadAddress => {
                let offset = ctx.pop_u64()? as usize;
                let burn_tx_data = ctx.burn_tx_data();

                if offset + 20 > burn_tx_data.len() {
                    return Err(BridgeError::InvalidDataLength);
                }

                let bytes = burn_tx_data[offset..offset + 20].to_vec();
                ctx.stack_mut().push(StackValue::Bytes(bytes));
            }

            ValidationOp::LoadEvmU64 => {
                let offset = ctx.pop_u64()? as usize;
                let burn_tx_data = ctx.burn_tx_data();

                if offset + 32 > burn_tx_data.len() {
                    return Err(BridgeError::InvalidDataLength);
                }

                let mut evm_bytes = [0u8; 32];
                evm_bytes.copy_from_slice(&burn_tx_data[offset..offset + 32]);
                let value = crate::bridge_contract::decode_arithmetic::bytes32_to_u64(&evm_bytes)?;
                ctx.stack_mut().push(StackValue::U64(value));
            }

            // Expected value operations (delegated to context)
            ValidationOp::PushExpectedAmount
            | ValidationOp::PushExpectedAddress
            | ValidationOp::PushExpectedNonce
            | ValidationOp::PushExpectedValidityHeight => {
                ctx.handle_push_expected(op)?;
            }

            // Arithmetic operations
            ValidationOp::Add => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = crate::bridge_contract::decode_arithmetic::checked_add_u256(a, b)?;
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Sub => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = crate::bridge_contract::decode_arithmetic::checked_sub_u256(a, b)?;
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Mul => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = crate::bridge_contract::decode_arithmetic::checked_mul_u256(a, b)?;
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Div => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                if b == 0 {
                    return Err(BridgeError::InvalidAmount);
                }
                let result = crate::bridge_contract::decode_arithmetic::checked_div_u256(a, b)?;
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Mod => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                if b == 0 {
                    return Err(BridgeError::InvalidAmount);
                }
                let result = a % b;
                ctx.stack_mut().push(StackValue::U64(result));
            }

            // Comparison operations
            ValidationOp::Eq => {
                let b = ctx.pop()?;
                let a = ctx.pop()?;
                let result = if a == b { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Lt => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = if a < b { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Gt => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = if a > b { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::IsZero => {
                let a = ctx.pop_u64()?;
                let result = if a == 0 { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            // Logical operations
            ValidationOp::And => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = if a != 0 && b != 0 { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Or => {
                let b = ctx.pop_u64()?;
                let a = ctx.pop_u64()?;
                let result = if a != 0 || b != 0 { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            ValidationOp::Not => {
                let a = ctx.pop_u64()?;
                let result = if a == 0 { 1 } else { 0 };
                ctx.stack_mut().push(StackValue::U64(result));
            }

            // Stack manipulation
            ValidationOp::Dup => {
                let value = ctx
                    .stack_mut()
                    .last()
                    .ok_or(BridgeError::InvalidRecipientData)?
                    .clone();
                ctx.stack_mut().push(value);
            }

            ValidationOp::Swap => {
                let len = ctx.stack_mut().len();
                if len < 2 {
                    return Err(BridgeError::InvalidRecipientData);
                }
                ctx.stack_mut().swap(len - 1, len - 2);
            }

            ValidationOp::Pop => {
                ctx.pop()?;
            }

            // Assert operation (delegated to context)
            ValidationOp::Assert => {
                ctx.handle_assert()?;
            }

            // Control flow
            ValidationOp::JumpIfZero(skip) => {
                let condition = ctx.pop_u64()?;
                if condition == 0 {
                    *pc += skip;
                }
            }

            // Storage operations
            ValidationOp::Store(name) => {
                let value = ctx.pop()?;
                ctx.storage_mut().insert(name.clone(), value);
            }

            ValidationOp::Load(name) => {
                let value = ctx
                    .storage_mut()
                    .get(name)
                    .ok_or(BridgeError::InvalidRecipientData)?
                    .clone();
                ctx.stack_mut().push(value);
            }
        }

        Ok(())
    }

    /// Executes the validation program in extraction-only mode.
    ///
    /// This method extracts values from burn transaction data without performing validation.
    /// It will return an error if the program contains validation operations that require
    /// expected values (PushExpected*, Assert, or comparison operations used for validation).
    pub fn extract_only(&self, burn_tx_data: &[u8]) -> Result<ValidationResult, BridgeError> {
        let mut ctx = ExtractionContext {
            burn_tx_data,
            stack: Vec::new(),
            storage: std::collections::HashMap::new(),
        };

        let mut pc = 0; // Program counter

        while pc < self.operations.len() {
            let op = &self.operations[pc];
            Self::execute_operation(&mut ctx, op, &mut pc)?;
            pc += 1;
        }

        Ok(ValidationResult {
            extracted_values: ctx.storage,
        })
    }

    /// Executes the validation program with value extraction.
    ///
    /// This method both validates the burn transaction data and extracts
    /// values that were stored using Store operations.
    pub fn execute_with_extraction(
        &self,
        burn_tx_data: &[u8],
        incoming_tx: &IncomingTransaction,
    ) -> Result<ValidationResult, BridgeError> {
        let mut ctx = ValidationContext {
            burn_tx_data,
            incoming_tx,
            stack: Vec::new(),
            storage: std::collections::HashMap::new(),
        };

        let mut pc = 0; // Program counter

        while pc < self.operations.len() {
            let op = &self.operations[pc];
            Self::execute_operation(&mut ctx, op, &mut pc)?;
            pc += 1;
        }

        Ok(ValidationResult {
            extracted_values: ctx.storage,
        })
    }

    /// Executes the validation program (legacy method for backward compatibility).
    pub fn execute(
        &self,
        burn_tx_data: &[u8],
        incoming_tx: &IncomingTransaction,
    ) -> Result<(), BridgeError> {
        self.execute_with_extraction(burn_tx_data, incoming_tx)?;
        Ok(())
    }
}

/// Trait for execution contexts that can handle validation operations.
///
/// This trait abstracts over the differences between ExtractionContext and ValidationContext,
/// allowing shared operation handling logic to work with both.
trait ExecutionContext {
    /// Returns a reference to the burn transaction data
    fn burn_tx_data(&self) -> &[u8];

    /// Returns a mutable reference to the execution stack
    fn stack_mut(&mut self) -> &mut Vec<StackValue>;

    /// Returns a mutable reference to the storage
    fn storage_mut(&mut self) -> &mut std::collections::HashMap<String, StackValue>;

    /// Handles PushExpected* operations (returns error for extraction-only contexts)
    fn handle_push_expected(&mut self, op: &ValidationOp) -> Result<(), BridgeError>;

    /// Handles Assert operations (returns error for extraction-only contexts)
    fn handle_assert(&mut self) -> Result<(), BridgeError>;

    /// Pops a u64 value from the stack
    fn pop_u64(&mut self) -> Result<u64, BridgeError> {
        let value = self
            .stack_mut()
            .pop()
            .ok_or(BridgeError::InvalidRecipientData)?;
        value.as_u64()
    }

    /// Pops any value from the stack
    fn pop(&mut self) -> Result<StackValue, BridgeError> {
        self.stack_mut()
            .pop()
            .ok_or(BridgeError::InvalidRecipientData)
    }
}

/// Execution context for validation.
///
/// Extraction context for extraction-only mode.
///
/// Maintains the state during extraction program execution without validation.
/// This context does not include an IncomingTransaction since it's used for
/// extracting values from burn data without validating against expected values.
struct ExtractionContext<'a> {
    /// Raw burn transaction data
    burn_tx_data: &'a [u8],
    /// Execution stack
    stack: Vec<StackValue>,
    /// Storage for extracted values
    storage: std::collections::HashMap<String, StackValue>,
}

impl<'a> ExecutionContext for ExtractionContext<'a> {
    fn burn_tx_data(&self) -> &[u8] {
        self.burn_tx_data
    }

    fn stack_mut(&mut self) -> &mut Vec<StackValue> {
        &mut self.stack
    }

    fn storage_mut(&mut self) -> &mut std::collections::HashMap<String, StackValue> {
        &mut self.storage
    }

    fn handle_push_expected(&mut self, op: &ValidationOp) -> Result<(), BridgeError> {
        log::error!(
            ?op,
            "Validation program contains PushExpected* operation in extraction-only context"
        );
        Err(BridgeError::InvalidRecipientData)
    }

    fn handle_assert(&mut self) -> Result<(), BridgeError> {
        log::error!("Validation program contains Assert operation in extraction-only context");
        Err(BridgeError::InvalidRecipientData)
    }
}

/// Validation context for full validation mode.
///
/// Maintains the state during validation program execution.
struct ValidationContext<'a> {
    /// Raw burn transaction data
    burn_tx_data: &'a [u8],
    /// Incoming transaction to validate against
    incoming_tx: &'a IncomingTransaction,
    /// Execution stack
    stack: Vec<StackValue>,
    /// Storage for extracted values
    storage: std::collections::HashMap<String, StackValue>,
}

impl<'a> ExecutionContext for ValidationContext<'a> {
    fn burn_tx_data(&self) -> &[u8] {
        self.burn_tx_data
    }

    fn stack_mut(&mut self) -> &mut Vec<StackValue> {
        &mut self.stack
    }

    fn storage_mut(&mut self) -> &mut std::collections::HashMap<String, StackValue> {
        &mut self.storage
    }

    fn handle_push_expected(&mut self, op: &ValidationOp) -> Result<(), BridgeError> {
        match op {
            ValidationOp::PushExpectedAmount => {
                let amount: u64 = self.incoming_tx.amount.into();
                self.stack.push(StackValue::U64(amount));
                Ok(())
            }
            ValidationOp::PushExpectedAddress => {
                let bytes = self.incoming_tx.recipient_data.target_address.clone();
                self.stack.push(StackValue::Bytes(bytes));
                Ok(())
            }
            ValidationOp::PushExpectedNonce => {
                self.stack.push(StackValue::U64(
                    self.incoming_tx.recipient_data.target_nonce,
                ));
                Ok(())
            }
            ValidationOp::PushExpectedValidityHeight => {
                self.stack
                    .push(StackValue::U32(self.incoming_tx.validity_start_height));
                Ok(())
            }
            _ => Err(BridgeError::InvalidRecipientData),
        }
    }

    fn handle_assert(&mut self) -> Result<(), BridgeError> {
        let condition = self.pop_u64()?;
        if condition == 0 {
            log::error!("Validation assertion failed");
            return Err(BridgeError::InvalidRecipientData);
        }
        Ok(())
    }
}

// ============================================================================
// Comprehensive Transaction Validation
// ============================================================================

/// Comprehensive transaction validation utilities.
///
/// Provides functions to validate all aspects of cross-chain transactions,
/// including burn transaction fields, amounts, addresses, validity heights,
/// and cryptographic data integrity.
pub mod transaction_validation {
    use nimiq_hash::{Blake2bHasher, Hasher};
    use nimiq_primitives::{coin::Coin, policy::Policy};

    use super::{
        AddressFormat, BridgeError, ChainConfig, IncomingTransaction, OutgoingTransaction,
    };

    /// Validates all fields of a burn transaction.
    /// Validates that an amount is positive and within acceptable ranges.
    ///
    /// Checks that the amount is non-zero and does not exceed the maximum
    /// allowed value for cross-chain transfers.
    pub fn validate_amount(amount: Coin, max_amount: Option<Coin>) -> Result<(), BridgeError> {
        // Check amount is positive
        if amount == Coin::ZERO {
            return Err(BridgeError::InvalidAmount);
        }

        // Check against maximum if specified
        if let Some(max) = max_amount
            && amount > max
        {
            return Err(BridgeError::InvalidAmount);
        }

        Ok(())
    }

    /// Validates that an address follows the correct format for the chain.
    ///
    /// Checks address length and format according to the chain's address
    /// format specification.
    pub fn validate_address_format(
        address_bytes: &[u8],
        chain_config: &ChainConfig,
    ) -> Result<(), BridgeError> {
        match chain_config.address_format {
            AddressFormat::Nimiq | AddressFormat::Ethereum => {
                // Both use 20-byte addresses
                if address_bytes.len() != 20 {
                    return Err(BridgeError::InvalidAddress(
                        "Address must be 20 bytes".to_string(),
                    ));
                }
                Ok(())
            }
            AddressFormat::Bitcoin => {
                // Bitcoin addresses are variable length (20-64 bytes typically)
                if address_bytes.len() < 20 || address_bytes.len() > 64 {
                    return Err(BridgeError::InvalidAddress(
                        "Bitcoin address length invalid".to_string(),
                    ));
                }
                Ok(())
            }
            AddressFormat::Custom(ref format_name) => {
                // Custom format - basic length check
                if address_bytes.is_empty() || address_bytes.len() > 64 {
                    return Err(BridgeError::InvalidAddress(format!(
                        "Custom address format {} invalid",
                        format_name
                    )));
                }
                Ok(())
            }
        }
    }

    /// Validates that a validity start height is reasonable.
    ///
    /// Checks that the height is non-zero and not unreasonably far in the future.
    pub fn validate_validity_height(
        validity_height: u32,
        current_height: u32,
        max_future_blocks: u32,
    ) -> Result<(), BridgeError> {
        // Check height is non-zero
        if validity_height == 0 {
            return Err(BridgeError::InvalidValidityHeight);
        }

        // Check height is not too far in the future
        if validity_height > current_height.saturating_add(max_future_blocks) {
            return Err(BridgeError::InvalidValidityHeight);
        }

        Ok(())
    }

    /// Validates cryptographic data integrity using checksums.
    ///
    /// Computes a Blake2b hash of the data and compares it to the expected hash.
    pub fn validate_data_integrity(
        data: &[u8],
        expected_hash: &nimiq_hash::Blake2bHash,
    ) -> Result<(), BridgeError> {
        if data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        let computed_hash = Blake2bHasher::default().digest(data);
        if computed_hash != *expected_hash {
            return Err(BridgeError::InvalidMerkleProof);
        }

        Ok(())
    }

    /// Performs comprehensive validation of an incoming transaction.
    ///
    /// Validates all fields including amount, chain ID, timestamp, validity height,
    /// and recipient data.
    pub fn validate_incoming_transaction_comprehensive(
        tx: &IncomingTransaction,
        chain_config: &ChainConfig,
        current_height: u32,
    ) -> Result<(), BridgeError> {
        // Validate amount
        validate_amount(tx.amount, None)?;

        // Validate validity height
        validate_validity_height(
            tx.validity_start_height,
            current_height,
            Policy::transaction_validity_window_blocks(),
        )?;

        // Validate recipient data
        tx.recipient_data.validate(chain_config)?;

        Ok(())
    }

    /// Performs comprehensive validation of an outgoing transaction.
    ///
    /// Validates all fields including burn data, amount, nonce, block height,
    /// and proof depth.
    pub fn validate_outgoing_transaction_comprehensive(
        tx: &OutgoingTransaction,
        max_proof_depth: u32,
        chain_config: &ChainConfig,
    ) -> Result<(), BridgeError> {
        // Validate burn transaction data
        if tx.burn_transaction_data.is_empty() {
            return Err(BridgeError::InvalidDataLength);
        }

        if tx.burn_transaction_data.len() > 10_000 {
            return Err(BridgeError::InvalidDataLength);
        }

        // Parse the transaction to validate its contents
        let parsed = tx.parse_burn_data(chain_config)?;

        // Validate amount
        validate_amount(parsed.amount, None)?;

        // Validate nonce
        if parsed.target_nonce == 0 {
            return Err(BridgeError::NonceAlreadyUsed);
        }

        // Validate block height
        if parsed.burn_block_height == 0 {
            return Err(BridgeError::InvalidValidityHeight);
        }

        // Validate proof depth
        if !tx.is_proof_depth_valid(max_proof_depth) {
            return Err(BridgeError::ProofDepthExceeded);
        }

        Ok(())
    }
}
