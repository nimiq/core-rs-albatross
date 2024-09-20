#[macro_use]
extern crate log;

use std::{
    cmp::{Ord, Ordering},
    collections::BTreeSet,
    convert::TryFrom,
    io::{self, Write},
};

use bitflags::bitflags;
use historic_transaction::RawTransactionHash;
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_keys::{Address, PublicKey};
use nimiq_network_interface::network::Topic;
use nimiq_primitives::{
    account::AccountType,
    coin::{Coin, CoinBe},
    networks::NetworkId,
    policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::merkle::Blake2bMerkleProof;
pub use signature_proof::*;
use thiserror::Error;

use crate::account::{
    htlc_contract::{CreationTransactionData as HtlcCreationData, OutgoingHTLCTransactionProof},
    staking_contract::IncomingStakingTransactionData,
    vesting_contract::CreationTransactionData as VestingCreationData,
    AccountTransactionVerification,
};

mod control_transaction;
mod equivocation_locator;

pub mod account;
pub mod historic_transaction;
pub mod history_proof;
pub mod inherent;
pub mod reward;
pub mod signature_proof;

pub use self::{
    control_transaction::ControlTransaction,
    equivocation_locator::{
        DoubleProposalLocator, DoubleVoteLocator, EquivocationLocator, ForkLocator,
    },
};

/// Transaction topic for the Mempool to request transactions from the network
#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "regular-transaction";
    const VALIDATE: bool = true;
}

/// Control Transaction topic for the Mempool to request control transactions from the network
#[derive(Clone, Debug, Default)]
pub struct ControlTransactionTopic;

impl Topic for ControlTransactionTopic {
    type Item = ControlTransaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "control-transaction";
    const VALIDATE: bool = true;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionsProof {
    pub transactions: Vec<Transaction>,
    pub proof: Blake2bMerkleProof,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub transaction_hash: Blake2bHash,
    pub block_hash: Blake2bHash,
    pub block_height: u32,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
#[cfg_attr(
    feature = "ts-types",
    derive(tsify::Tsify),
    serde(rename = "PlainTransactionFormat", rename_all = "lowercase"),
    wasm_bindgen::prelude::wasm_bindgen
)]
pub enum TransactionFormat {
    Basic = 0,
    Extended = 1,
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    #[serde(try_from = "u8", into = "u8")]
    pub struct TransactionFlags: u8 {
        const CONTRACT_CREATION = 0b1;
        const SIGNALING = 0b10;
    }
}

#[derive(Debug, Error)]
#[error("Invalid transaction flags: {0}")]
pub struct TransactionFlagsConvertError(u8);

impl TryFrom<u8> for TransactionFlags {
    type Error = TransactionFlagsConvertError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        TransactionFlags::from_bits(value).ok_or(TransactionFlagsConvertError(value))
    }
}

impl From<TransactionFlags> for u8 {
    fn from(flags: TransactionFlags) -> Self {
        flags.bits()
    }
}

/// A wrapper around the Transaction struct that encodes the result of executing such transaction
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ExecutedTransaction {
    /// A successfully executed transaction
    Ok(Transaction),
    /// A failed transaction (only fees are deducted)
    Err(Transaction),
}

impl ExecutedTransaction {
    /// Obtains the underlying transaction, regardless of execution result
    pub fn get_raw_transaction(&self) -> &Transaction {
        match self {
            ExecutedTransaction::Ok(txn) => txn,
            ExecutedTransaction::Err(txn) => txn,
        }
    }
    pub fn failed(&self) -> bool {
        match self {
            ExecutedTransaction::Ok(_) => false,
            ExecutedTransaction::Err(..) => true,
        }
    }

    pub fn succeeded(&self) -> bool {
        match self {
            ExecutedTransaction::Ok(_) => true,
            ExecutedTransaction::Err(..) => false,
        }
    }

    pub fn serialize_content(&self) -> Vec<u8> {
        let mut result = Vec::new();
        SerializeContent::serialize_content::<_, Blake2bHash>(self, &mut result).unwrap();
        result
    }

    /// Gets the inner transaction hash without the execution result.
    /// This hash is the only hash the mempool and users know.
    pub fn raw_tx_hash(&self) -> RawTransactionHash {
        self.get_raw_transaction().hash::<Blake2bHash>().into()
    }
}

impl SerializeContent for ExecutedTransaction {
    fn serialize_content<W: Write, H>(&self, writer: &mut W) -> io::Result<()> {
        matches!(self, ExecutedTransaction::Ok(_)).serialize_to_writer(writer)?;

        match self {
            ExecutedTransaction::Ok(txn) | ExecutedTransaction::Err(txn) => {
                txn.serialize_to_writer(writer)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Eq, Debug)]
pub struct Transaction {
    pub sender: Address,
    pub sender_type: AccountType,
    pub sender_data: Vec<u8>,
    pub recipient: Address,
    pub recipient_type: AccountType,
    pub recipient_data: Vec<u8>,
    pub value: Coin,
    pub fee: Coin,
    pub validity_start_height: u32,
    pub network_id: NetworkId,
    pub flags: TransactionFlags,
    pub proof: Vec<u8>,
    valid: bool,
}

impl Transaction {
    /// The size in bytes of the smallest possible transaction (basic single-sig).
    pub const MIN_SIZE: usize = 138;

    pub fn new_basic(
        sender: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        Self {
            sender,
            sender_type: AccountType::Basic,
            sender_data: Vec::new(),
            recipient,
            recipient_type: AccountType::Basic,
            recipient_data: Vec::new(),
            value,
            fee,
            validity_start_height,
            network_id,
            flags: TransactionFlags::empty(),
            proof: Vec::new(),
            valid: false,
        }
    }

    pub fn new_extended(
        sender: Address,
        sender_type: AccountType,
        sender_data: Vec<u8>,
        recipient: Address,
        recipient_type: AccountType,
        recipient_data: Vec<u8>,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        Self {
            sender,
            sender_type,
            sender_data,
            recipient,
            recipient_type,
            recipient_data,
            value,
            fee,
            validity_start_height,
            network_id,
            flags: TransactionFlags::empty(),
            proof: Vec::new(),
            valid: false,
        }
    }

    pub fn new_signaling(
        sender: Address,
        sender_type: AccountType,
        recipient: Address,
        recipient_type: AccountType,
        fee: Coin,
        recipient_data: Vec<u8>,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        Self {
            sender,
            sender_type,
            sender_data: Vec::new(),
            recipient,
            recipient_type,
            recipient_data,
            value: Coin::ZERO,
            fee,
            validity_start_height,
            network_id,
            flags: TransactionFlags::SIGNALING,
            proof: Vec::new(),
            valid: false,
        }
    }

    pub fn new_contract_creation(
        sender: Address,
        sender_type: AccountType,
        sender_data: Vec<u8>,
        recipient_type: AccountType,
        recipient_data: Vec<u8>,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        let mut tx = Self {
            sender,
            sender_type,
            sender_data,
            recipient: Address::from([0u8; Address::SIZE]),
            recipient_type,
            recipient_data,
            value,
            fee,
            validity_start_height,
            network_id,
            flags: TransactionFlags::CONTRACT_CREATION,
            proof: Vec::new(),
            valid: false,
        };
        tx.recipient = tx.contract_creation_address();
        tx
    }

    pub fn format(&self) -> TransactionFormat {
        if self.sender_type == AccountType::Basic
            && self.recipient_type == AccountType::Basic
            && self.recipient_data.is_empty()
            && self.flags.is_empty()
        {
            if let Ok(signature_proof) = SignatureProof::deserialize_all(&self.proof) {
                if self.sender == Address::from(&signature_proof.public_key)
                    && signature_proof.merkle_path.is_empty()
                {
                    return TransactionFormat::Basic;
                }
            }
        }
        TransactionFormat::Extended
    }

    pub fn verify_mut(&mut self, network_id: NetworkId) -> Result<(), TransactionError> {
        let ret = self.verify(network_id);
        if ret.is_ok() {
            self.valid = true;
        }
        ret
    }

    pub fn verify(&self, network_id: NetworkId) -> Result<(), TransactionError> {
        if self.valid {
            return Ok(());
        }

        if self.sender == self.recipient {
            return Err(TransactionError::SenderEqualsRecipient);
        }

        if self.network_id != network_id {
            return Err(TransactionError::ForeignNetwork);
        }

        // Check that value > 0 except if it is a signaling transaction.
        if self.flags.contains(TransactionFlags::SIGNALING) {
            if self.value != Coin::ZERO {
                return Err(TransactionError::InvalidForRecipient);
            }
        } else if self.value == Coin::ZERO {
            return Err(TransactionError::ZeroValue);
        }

        // Check that value + fee doesn't overflow.
        match self.value.checked_add(self.fee) {
            Some(coin) => {
                if coin > Coin::from_u64_unchecked(Policy::TOTAL_SUPPLY) {
                    return Err(TransactionError::Overflow);
                }
            }
            None => return Err(TransactionError::Overflow),
        }

        // Check transaction validity for sender account.
        AccountType::verify_outgoing_transaction(self)?;

        // Check transaction validity for recipient account.
        AccountType::verify_incoming_transaction(self)?;

        Ok(())
    }

    pub fn is_valid_at(&self, block_height: u32) -> bool {
        let window = Policy::transaction_validity_window_blocks();
        block_height
            >= self
                .validity_start_height
                .saturating_sub(Policy::blocks_per_batch())
            && block_height < self.validity_start_height + window
    }

    pub fn contract_creation_address(&self) -> Address {
        let mut tx = self.clone();
        tx.recipient = Address::from([0u8; Address::SIZE]);
        let hash: Blake2bHash = tx.hash();
        Address::from(hash)
    }

    pub fn fee_per_byte(&self) -> f64 {
        u64::from(self.fee) as f64 / self.serialized_size() as f64
    }

    pub fn serialize_content(&self) -> Vec<u8> {
        let mut result = Vec::new();
        SerializeContent::serialize_content::<_, Blake2bHash>(self, &mut result).unwrap();
        result
    }

    pub fn total_value(&self) -> Coin {
        // Avoid wrapping in case this is called before verify().
        self.value.saturating_add(self.fee)
    }

    pub fn sender(&self) -> &Address {
        &self.sender
    }

    pub fn recipient(&self) -> &Address {
        &self.recipient
    }

    pub fn related_addresses(&self) -> BTreeSet<Address> {
        let mut addresses = BTreeSet::new();

        // Add sender and recipient
        addresses.insert(self.sender.clone());
        addresses.insert(self.recipient.clone());

        // If proof is a regular signature proof, add signer of the proof
        if let Ok(proof) = SignatureProof::deserialize_all(&self.proof) {
            addresses.insert(proof.compute_signer());
        }

        match self.sender_type {
            AccountType::Basic | AccountType::Vesting => {}
            AccountType::HTLC => {
                if let Ok(proof) = OutgoingHTLCTransactionProof::deserialize_all(&self.proof) {
                    match proof {
                        OutgoingHTLCTransactionProof::RegularTransfer {
                            signature_proof, ..
                        } => {
                            // Add signer of the proof ("recipient" of the HTLC)
                            addresses.insert(signature_proof.compute_signer());
                        }
                        OutgoingHTLCTransactionProof::EarlyResolve {
                            signature_proof_recipient,
                            signature_proof_sender,
                        } => {
                            // Add signers of the proof (both "sender" and "recipient" of the HTLC)
                            addresses.insert(signature_proof_recipient.compute_signer());
                            addresses.insert(signature_proof_sender.compute_signer());
                        }
                        OutgoingHTLCTransactionProof::TimeoutResolve {
                            signature_proof_sender,
                        } => {
                            // Add signer of the proof ("sender" of the HTLC)
                            addresses.insert(signature_proof_sender.compute_signer());
                        }
                    }
                }
            }
            AccountType::Staking => {
                // Transactions from the staking contract are signed with a regular signature proof
                // by the validator or staker. Signers of regular transaction proofs have already
                // been added to the set above.
            }
        }

        match self.recipient_type {
            AccountType::Basic => {}
            AccountType::Vesting => {
                if let Ok(contract_data) = VestingCreationData::parse(self) {
                    // Add the owner of the new vesting contract
                    addresses.insert(contract_data.owner);
                }
            }
            AccountType::HTLC => {
                if let Ok(contract_data) = HtlcCreationData::parse(self) {
                    // Add both the "sender" and "recipient" of the new HTLC
                    addresses.insert(contract_data.sender);
                    addresses.insert(contract_data.recipient);
                }
            }
            AccountType::Staking => {
                if let Ok(contract_data) = IncomingStakingTransactionData::parse(self) {
                    match contract_data {
                        IncomingStakingTransactionData::CreateValidator {
                            signing_key,
                            reward_address,
                            proof,
                            ..
                        } => {
                            // Add the new validator's signing key
                            addresses.insert(Address::from(&signing_key));
                            // Add the new validator's reward address
                            addresses.insert(reward_address);
                            // The signer of the internal proof is the validator address
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::UpdateValidator {
                            new_signing_key,
                            new_reward_address,
                            proof,
                            ..
                        } => {
                            if let Some(new_signing_key) = new_signing_key {
                                // If the signing key is updated, add the new signing key
                                addresses.insert(Address::from(&new_signing_key));
                            }
                            if let Some(new_reward_address) = new_reward_address {
                                // If the reward address is updated, add the new reward address
                                addresses.insert(new_reward_address);
                            }
                            // The signer of the internal proof is the validator address
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::DeactivateValidator {
                            validator_address,
                            proof,
                        } => {
                            // Add the validator address
                            addresses.insert(validator_address);
                            // The signer of the internal proof is the validator's signing key
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::ReactivateValidator {
                            validator_address,
                            proof,
                        } => {
                            // Add the validator address
                            addresses.insert(validator_address);
                            // The signer of the internal proof is the validator's signing key
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::RetireValidator { proof } => {
                            // The signer of the internal proof is the validator address
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::CreateStaker { delegation, proof } => {
                            if let Some(delegation) = delegation {
                                // Add the validator address the new staker is delegating to
                                addresses.insert(delegation);
                            }
                            // The signer of the internal proof is the staker address
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::AddStake { staker_address } => {
                            // Add the staker address
                            addresses.insert(staker_address);
                        }
                        IncomingStakingTransactionData::UpdateStaker {
                            new_delegation,
                            proof,
                            ..
                        } => {
                            if let Some(new_delegation) = new_delegation {
                                // If the delegation is updated, add the new validator address
                                addresses.insert(new_delegation);
                            }
                            // The signer of the internal proof is the staker address
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::SetActiveStake { proof, .. } => {
                            // The signer of the internal proof is the staker address
                            addresses.insert(proof.compute_signer());
                        }
                        IncomingStakingTransactionData::RetireStake { proof, .. } => {
                            // The signer of the internal proof is the staker address
                            addresses.insert(proof.compute_signer());
                        }
                    }
                }
            }
        }

        addresses
    }
}

impl SerializeContent for Transaction {
    fn serialize_content<W: Write, H>(&self, writer: &mut W) -> io::Result<()> {
        // This implementation must be kept in sync with
        // `HistoricTransaction::tx_hash`.

        // Serialize data as in PoW (2 bytes for the length and then the data
        // which in PoS is the recipient data) for backwards compatibility
        writer.write_all(&(self.recipient_data.len() as u16).to_be_bytes())?;
        writer.write_all(&self.recipient_data)?;
        self.sender.serialize_to_writer(writer)?;
        self.sender_type.serialize_to_writer(writer)?;
        self.recipient.serialize_to_writer(writer)?;
        self.recipient_type.serialize_to_writer(writer)?;
        CoinBe::from(self.value).serialize_to_writer(writer)?;
        CoinBe::from(self.fee).serialize_to_writer(writer)?;
        writer.write_all(&self.validity_start_height.to_be_bytes())?;
        self.network_id.serialize_to_writer(writer)?;
        self.flags.serialize_to_writer(writer)?;
        // Only serialize the sender data if the network ID is a PoS one for
        // backwards compatibility
        if self.network_id.is_albatross() {
            self.sender_data.serialize_to_writer(writer)?;
        }
        Ok(())
    }
}

impl PartialEq for Transaction {
    fn eq(&self, other: &Self) -> bool {
        self.sender == other.sender
            && self.sender_type == other.sender_type
            && self.recipient == other.recipient
            && self.recipient_type == other.recipient_type
            && self.value == other.value
            && self.fee == other.fee
            && self.validity_start_height == other.validity_start_height
            && self.network_id == other.network_id
            && self.flags == other.flags
            && self.recipient_data == other.recipient_data
            && self.sender_data == other.sender_data
    }
}

impl PartialOrd for Transaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Transaction {
    fn cmp(&self, other: &Self) -> Ordering {
        Ordering::Equal
            .then_with(|| self.recipient.cmp(&other.recipient))
            .then_with(|| self.validity_start_height.cmp(&other.validity_start_height))
            .then_with(|| other.fee.cmp(&self.fee))
            .then_with(|| other.value.cmp(&self.value))
            .then_with(|| self.sender.cmp(&other.sender))
            .then_with(|| self.recipient_type.cmp(&other.recipient_type))
            .then_with(|| self.sender_type.cmp(&other.sender_type))
            .then_with(|| self.flags.cmp(&other.flags))
            .then_with(|| self.recipient_data.len().cmp(&other.recipient_data.len()))
            .then_with(|| self.recipient_data.cmp(&other.recipient_data))
            .then_with(|| self.sender_data.len().cmp(&other.sender_data.len()))
            .then_with(|| self.sender_data.cmp(&other.sender_data))
    }
}

mod serde_derive {
    use std::fmt;

    use nimiq_keys::{
        ES256PublicKey, ES256Signature, Ed25519PublicKey, Ed25519Signature, Signature,
    };
    use serde::{
        de::{EnumAccess, Error, SeqAccess, VariantAccess, Visitor},
        ser::{Error as SerError, SerializeStructVariant},
    };

    use super::*;

    const ENUM_NAME: &str = "Transaction";
    const VARIANTS: &[&str] = &["Basic", "Extended"];
    const BASIC_FIELDS: &[&str] = &[
        "proof_type_and_flags",
        "public_key",
        "recipient",
        "value",
        "fee",
        "validity_start_height",
        "network_id",
        "signature",
        "webauthn_fields",
    ];
    const EXTENDED_FIELDS: &[&str] = &[
        "sender",
        "sender_type",
        "sender_data",
        "recipient",
        "recipient_type",
        "recipient_data",
        "value",
        "fee",
        "validity_start_height",
        "network_id",
        "flags",
        "proof",
    ];

    struct TransactionVisitor;
    struct BasicTransactionVisitor;
    struct ExtendedTransactionVisitor;

    impl serde::Serialize for Transaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match self.format() {
                TransactionFormat::Basic => {
                    let mut sv = serializer.serialize_struct_variant(
                        ENUM_NAME,
                        0,
                        VARIANTS[0],
                        BASIC_FIELDS.len(),
                    )?;
                    let signature_proof = SignatureProof::deserialize_all(&self.proof)
                        .map_err(|_| S::Error::custom("Could not deserialize signature proof"))?;
                    // Serialize public_key and signature algorithm and if webauthn_fields exist in one u8
                    sv.serialize_field(
                        BASIC_FIELDS[0],
                        &signature_proof.make_type_and_flags_byte(),
                    )?;
                    match signature_proof.public_key {
                        PublicKey::Ed25519(ref public_key) => {
                            sv.serialize_field(BASIC_FIELDS[1], public_key)?;
                        }
                        PublicKey::ES256(ref public_key) => {
                            sv.serialize_field(BASIC_FIELDS[1], public_key)?;
                        }
                    }
                    sv.serialize_field(BASIC_FIELDS[2], &self.recipient)?;
                    sv.serialize_field(BASIC_FIELDS[3], &self.value)?;
                    sv.serialize_field(BASIC_FIELDS[4], &self.fee)?;
                    sv.serialize_field(BASIC_FIELDS[5], &self.validity_start_height.to_be_bytes())?;
                    sv.serialize_field(BASIC_FIELDS[6], &self.network_id)?;
                    match signature_proof.signature {
                        Signature::Ed25519(ref signature) => {
                            sv.serialize_field(BASIC_FIELDS[7], signature)?;
                        }
                        Signature::ES256(ref signature) => {
                            sv.serialize_field(BASIC_FIELDS[7], signature)?;
                        }
                    }
                    if signature_proof.webauthn_fields.is_some() {
                        sv.serialize_field(
                            BASIC_FIELDS[8],
                            signature_proof.webauthn_fields.as_ref().unwrap(),
                        )?;
                    }
                    sv.end()
                }
                TransactionFormat::Extended => {
                    let mut sv = serializer.serialize_struct_variant(
                        ENUM_NAME,
                        1,
                        VARIANTS[1],
                        EXTENDED_FIELDS.len(),
                    )?;
                    sv.serialize_field(EXTENDED_FIELDS[0], &self.sender)?;
                    sv.serialize_field(EXTENDED_FIELDS[1], &self.sender_type)?;
                    sv.serialize_field(EXTENDED_FIELDS[2], &self.sender_data)?;
                    sv.serialize_field(EXTENDED_FIELDS[3], &self.recipient)?;
                    sv.serialize_field(EXTENDED_FIELDS[4], &self.recipient_type)?;
                    sv.serialize_field(EXTENDED_FIELDS[5], &self.recipient_data)?;
                    sv.serialize_field(EXTENDED_FIELDS[6], &self.value)?;
                    sv.serialize_field(EXTENDED_FIELDS[7], &self.fee)?;
                    sv.serialize_field(
                        EXTENDED_FIELDS[8],
                        &self.validity_start_height.to_be_bytes(),
                    )?;
                    sv.serialize_field(EXTENDED_FIELDS[9], &self.network_id)?;
                    sv.serialize_field(EXTENDED_FIELDS[10], &self.flags)?;
                    sv.serialize_field(EXTENDED_FIELDS[11], &self.proof)?;
                    sv.end()
                }
            }
        }
    }

    impl<'de> serde::Deserialize<'de> for Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_enum(ENUM_NAME, VARIANTS, TransactionVisitor)
        }
    }

    impl<'de> Visitor<'de> for TransactionVisitor {
        type Value = Transaction;

        fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "a Transaction")
        }

        fn visit_enum<A>(self, value: A) -> Result<Transaction, A::Error>
        where
            A: EnumAccess<'de>,
        {
            let (index, tx_variant) = value.variant()?;
            match index {
                0 => tx_variant.struct_variant(BASIC_FIELDS, BasicTransactionVisitor),
                1 => tx_variant.struct_variant(EXTENDED_FIELDS, ExtendedTransactionVisitor),
                _ => Err(A::Error::custom("Undefined transaction type")),
            }
        }
    }

    impl<'de> Visitor<'de> for BasicTransactionVisitor {
        type Value = Transaction;

        fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "a BasicTransaction")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Transaction, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // Read type field to determine public_key and signature algorithm and if webauthn_fields exists
            let proof_type_and_flags: u8 = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(0, &self))?;
            let (algorithm, flags) =
                SignatureProof::parse_type_and_flags_byte(proof_type_and_flags)
                    .map_err(Error::custom)?;
            let public_key = match algorithm {
                SignatureProofAlgorithm::Ed25519 => {
                    let public_key: Ed25519PublicKey = seq
                        .next_element()?
                        .ok_or_else(|| Error::invalid_length(1, &self))?;
                    PublicKey::Ed25519(public_key)
                }
                SignatureProofAlgorithm::ES256 => {
                    let public_key: ES256PublicKey = seq
                        .next_element()?
                        .ok_or_else(|| Error::invalid_length(1, &self))?;
                    PublicKey::ES256(public_key)
                }
            };
            let recipient: Address = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(2, &self))?;
            let value: Coin = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(3, &self))?;
            let fee: Coin = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(4, &self))?;
            let validity_start_height: [u8; 4] = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(5, &self))?;
            let network_id: NetworkId = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(6, &self))?;
            let signature = match algorithm {
                SignatureProofAlgorithm::Ed25519 => {
                    let signature: Ed25519Signature = seq
                        .next_element()?
                        .ok_or_else(|| Error::invalid_length(3, &self))?;
                    Signature::Ed25519(signature)
                }
                SignatureProofAlgorithm::ES256 => {
                    let signature: ES256Signature = seq
                        .next_element()?
                        .ok_or_else(|| Error::invalid_length(3, &self))?;
                    Signature::ES256(signature)
                }
            };
            let webauthn_fields = if flags.contains(SignatureProofFlags::WEBAUTHN_FIELDS) {
                Some(
                    seq.next_element::<WebauthnExtraFields>()?
                        .ok_or_else(|| Error::invalid_length(8, &self))?,
                )
            } else {
                None
            };
            Ok(Transaction {
                sender: Address::from(&public_key),
                sender_type: AccountType::Basic,
                sender_data: vec![],
                recipient,
                recipient_type: AccountType::Basic,
                recipient_data: vec![],
                value,
                fee,
                validity_start_height: u32::from_be_bytes(validity_start_height),
                network_id,
                flags: TransactionFlags::empty(),
                proof: SignatureProof::from(public_key, signature, webauthn_fields)
                    .serialize_to_vec(),
                valid: false,
            })
        }
    }

    impl<'de> Visitor<'de> for ExtendedTransactionVisitor {
        type Value = Transaction;

        fn expecting(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f, "an ExtendedTransaction")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Transaction, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let sender: Address = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(0, &self))?;
            let sender_type: AccountType = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(1, &self))?;

            // Enforce maximum sender data length.
            let sender_data: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(2, &self))?;
            let sender_data_len = sender_data.len();
            if sender_data_len > Policy::MAX_TX_SENDER_DATA_SIZE {
                return Err(Error::custom(format!(
                    "sender data length exceeds the maximum allowed length of {}",
                    Policy::MAX_TX_SENDER_DATA_SIZE
                )));
            }

            let recipient: Address = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(3, &self))?;
            let recipient_type: AccountType = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(4, &self))?;

            // Enforce maximum recipient data length.
            let recipient_data: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(5, &self))?;
            if recipient_data.len() > Policy::MAX_TX_RECIPIENT_DATA_SIZE {
                return Err(Error::custom(format!(
                    "recipient data length exceeds the maximum allowed length of {}",
                    Policy::MAX_TX_RECIPIENT_DATA_SIZE
                )));
            }

            let value: Coin = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(6, &self))?;
            let fee: Coin = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(7, &self))?;
            let validity_start_height: [u8; 4] = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(8, &self))?;
            let network_id: NetworkId = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(9, &self))?;
            let flags: TransactionFlags = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(10, &self))?;
            let proof: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| Error::invalid_length(11, &self))?;
            Ok(Transaction {
                sender,
                sender_type,
                sender_data,
                recipient,
                recipient_type,
                recipient_data,
                value,
                fee,
                validity_start_height: u32::from_be_bytes(validity_start_height),
                network_id,
                flags,
                proof,
                valid: false,
            })
        }
    }
}
