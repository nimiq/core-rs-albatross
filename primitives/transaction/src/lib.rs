#[macro_use]
extern crate log;

use std::{
    cmp::{Ord, Ordering},
    convert::TryFrom,
    io,
    sync::Arc,
};

use bitflags::bitflags;
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_keys::{Address, PublicKey, Signature};
use nimiq_network_interface::network::Topic;
use nimiq_primitives::{
    account::AccountType, coin::Coin, networks::NetworkId, policy::Policy,
    transaction::TransactionError,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::merkle::{Blake2bMerklePath, Blake2bMerkleProof};
use num_traits::SaturatingAdd;
use thiserror::Error;

use crate::account::AccountTransactionVerification;

pub mod account;
pub mod extended_transaction;
pub mod history_proof;
pub mod inherent;
pub mod reward;

/// Transaction topic for the Mempool to request transactions from the network
#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "transactions";
    const VALIDATE: bool = true;
}

/// Control Transaction topic for the Mempool to request control transactions from the network
#[derive(Clone, Debug, Default)]
pub struct ControlTransactionTopic;

impl Topic for ControlTransactionTopic {
    type Item = Transaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "Controltransactions";
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
    #[derive(Default, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureProof {
    pub public_key: PublicKey,
    pub merkle_path: Blake2bMerklePath,
    pub signature: Signature,
}

impl SignatureProof {
    pub fn from(public_key: PublicKey, signature: Signature) -> Self {
        SignatureProof {
            public_key,
            merkle_path: Blake2bMerklePath::empty(),
            signature,
        }
    }

    pub fn compute_signer(&self) -> Address {
        let merkle_root = self.merkle_path.compute_root(&self.public_key);
        Address::from(merkle_root)
    }

    pub fn is_signed_by(&self, address: &Address) -> bool {
        self.compute_signer() == *address
    }

    pub fn verify(&self, message: &[u8]) -> bool {
        self.public_key.verify(&self.signature, message)
    }
}

impl Default for SignatureProof {
    fn default() -> Self {
        SignatureProof {
            public_key: Default::default(),
            merkle_path: Default::default(),
            signature: Signature::from_bytes(&[0u8; Signature::SIZE]).unwrap(),
        }
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

    pub fn hash(&self) -> Blake2bHash {
        match self {
            ExecutedTransaction::Ok(txn) => txn.hash(),
            ExecutedTransaction::Err(txn) => txn.hash(),
        }
    }
}

#[derive(Clone, Eq, Debug)]
#[repr(C)]
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
            if let Ok(signature_proof) = SignatureProof::deserialize_from_vec(&self.proof) {
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

        if self.recipient == Policy::STAKING_CONTRACT_ADDRESS
            && self.recipient_type != AccountType::Staking
        {
            return Err(TransactionError::InvalidForRecipient);
        }

        // Should not be necessary as the sender would have to sign the transaction
        // and the private key for the staking contract is unknown
        if self.sender == Policy::STAKING_CONTRACT_ADDRESS
            && self.sender_type != AccountType::Staking
        {
            return Err(TransactionError::InvalidForSender);
        }

        if self.sender == self.recipient {
            error!(
                "The following transaction can't have the same sender and recipient:\n{:?}",
                self
            );
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

    pub fn check_set_valid(&mut self, tx: &Arc<Transaction>) {
        if tx.valid && self.hash::<Blake2bHash>() == tx.hash() {
            self.valid = true;
        }
    }

    pub fn is_valid_at(&self, block_height: u32) -> bool {
        let window = Policy::transaction_validity_window();
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
        // Serialize data as in PoW (2 bytes for the length and then the data
        // which in PoS is the recipient data) for backwards compatibility
        let mut res: Vec<u8> = (self.recipient_data.len() as u16)
            .to_be_bytes()
            .serialize_to_vec();
        res.append(&mut self.recipient_data.clone());
        res.append(&mut self.sender.serialize_to_vec());
        res.append(&mut self.sender_type.serialize_to_vec());
        res.append(&mut self.recipient.serialize_to_vec());
        res.append(&mut self.recipient_type.serialize_to_vec());
        res.append(&mut self.value.serialize_to_vec());
        res.append(&mut self.fee.serialize_to_vec());
        res.append(&mut self.validity_start_height.to_be_bytes().serialize_to_vec());
        res.append(&mut self.network_id.serialize_to_vec());
        res.append(&mut self.flags.serialize_to_vec());
        // Only serialize the sender data if the network ID is a PoS one for
        // backwards compatibility
        if self.network_id.is_albatross() {
            res.append(&mut self.sender_data.serialize_to_vec());
        }
        res
    }

    pub fn total_value(&self) -> Coin {
        // Avoid wrapping in case this is called before verify().
        self.value.saturating_add(&self.fee)
    }

    pub fn sender(&self) -> &Address {
        &self.sender
    }

    pub fn recipient(&self) -> &Address {
        &self.recipient
    }
}

impl SerializeContent for Transaction {
    fn serialize_content<W: io::Write, H>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.serialize_content())
    }
}

impl std::hash::Hash for Transaction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.serialize_content(), state);
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

    use serde::{
        de::{EnumAccess, Error, SeqAccess, VariantAccess, Visitor},
        ser::{Error as SerError, SerializeStructVariant},
    };

    use super::*;

    const ENUM_NAME: &str = "Transaction";
    const VARIANTS: &[&str] = &["Basic", "Extended"];
    const BASIC_FIELDS: &[&str] = &[
        "public_key",
        "recipient",
        "value",
        "fee",
        "validity_start_height",
        "network_id",
        "signature",
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
                    let signature_proof: SignatureProof =
                        Deserialize::deserialize_from_vec(&self.proof)
                            .map_err(|_| S::Error::custom("Could not serialize signature proof"))?;
                    sv.serialize_field(BASIC_FIELDS[0], &signature_proof.public_key)?;
                    sv.serialize_field(BASIC_FIELDS[1], &self.recipient)?;
                    sv.serialize_field(BASIC_FIELDS[2], &self.value)?;
                    sv.serialize_field(BASIC_FIELDS[3], &self.fee)?;
                    sv.serialize_field(BASIC_FIELDS[4], &self.validity_start_height.to_be_bytes())?;
                    sv.serialize_field(BASIC_FIELDS[5], &self.network_id)?;
                    sv.serialize_field(BASIC_FIELDS[6], &signature_proof.signature)?;
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
            let public_key: PublicKey = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
            let recipient: Address = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
            let value: Coin = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
            let fee: Coin = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
            let validity_start_height: [u8; 4] = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(4, &self))?;
            let network_id: NetworkId = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(5, &self))?;
            let signature: Signature = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(6, &self))?;
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
                proof: SignatureProof::from(public_key, signature).serialize_to_vec(),
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
                .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
            let sender_type: AccountType = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
            let sender_data: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
            let recipient: Address = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
            let recipient_type: AccountType = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(4, &self))?;
            let recipient_data: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(5, &self))?;
            let value: Coin = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(6, &self))?;
            let fee: Coin = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(7, &self))?;
            let validity_start_height: [u8; 4] = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(8, &self))?;
            let network_id: NetworkId = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(9, &self))?;
            let flags: TransactionFlags = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(10, &self))?;
            let proof: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| serde::de::Error::invalid_length(11, &self))?;
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
