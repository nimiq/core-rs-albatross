#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_bls as bls;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_macros as macros;
extern crate nimiq_primitives as primitives;
extern crate nimiq_utils as utils;
extern crate strum_macros;

use std::cmp::{Ord, Ordering};
use std::convert::TryFrom;
use std::io;
use std::sync::Arc;

use bitflags::bitflags;
use thiserror::Error;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_keys::Address;
use nimiq_keys::{PublicKey, Signature};
use nimiq_utils::merkle::{Blake2bMerklePath, Blake2bMerkleProof};
use primitives::account::AccountType;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use primitives::policy;

use crate::account::AccountTransactionVerification;

pub mod account;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionsProof {
    #[beserial(len_type(u16))]
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
pub enum TransactionFormat {
    Basic = 0,
    Extended = 1,
}

bitflags! {
    #[derive(Default, Serialize)]
    #[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize), serde(try_from = "u8", into = "u8"))]
    pub struct TransactionFlags: u8 {
        const CONTRACT_CREATION = 0b1;
        const SIGNALLING = 0b10;
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

// Fail when deserializing invalid flags.
impl Deserialize for TransactionFlags {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let flag_data: u8 = reader.read_u8()?;
        TransactionFlags::from_bits(flag_data).ok_or(SerializingError::InvalidValue)
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

#[derive(Clone, Eq, Debug)]
#[repr(C)]
pub struct Transaction {
    pub data: Vec<u8>,
    pub sender: Address,
    pub sender_type: AccountType,
    pub recipient: Address,
    pub recipient_type: AccountType,
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
            data: Vec::new(),
            sender,
            sender_type: AccountType::Basic,
            recipient,
            recipient_type: AccountType::Basic,
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
        recipient: Address,
        recipient_type: AccountType,
        value: Coin,
        fee: Coin,
        data: Vec<u8>,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        Self {
            data,
            sender,
            sender_type,
            recipient,
            recipient_type,
            value,
            fee,
            validity_start_height,
            network_id,
            flags: TransactionFlags::empty(),
            proof: Vec::new(),
            valid: false,
        }
    }

    pub fn new_signalling(
        sender: Address,
        sender_type: AccountType,
        recipient: Address,
        recipient_type: AccountType,
        value: Coin,
        fee: Coin,
        data: Vec<u8>,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        Self {
            data,
            sender,
            sender_type,
            recipient,
            recipient_type,
            value,
            fee,
            validity_start_height,
            network_id,
            flags: TransactionFlags::SIGNALLING,
            proof: Vec::new(),
            valid: false,
        }
    }

    pub fn new_contract_creation(
        data: Vec<u8>,
        sender: Address,
        sender_type: AccountType,
        recipient_type: AccountType,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Self {
        let mut tx = Self {
            data,
            sender,
            sender_type,
            recipient: Address::from([0u8; Address::SIZE]),
            recipient_type,
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
            && self.data.is_empty()
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

    pub fn cmp_mempool_order(&self, other: &Transaction) -> Ordering {
        Ordering::Equal
            .then_with(|| {
                self.fee_per_byte()
                    .partial_cmp(&other.fee_per_byte())
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| self.fee.cmp(&other.fee))
            .then_with(|| self.value.cmp(&other.value))
            .then_with(|| self.recipient.cmp(&other.recipient))
            .then_with(|| self.validity_start_height.cmp(&other.validity_start_height))
            .then_with(|| self.sender.cmp(&other.sender))
            .then_with(|| self.recipient_type.cmp(&other.recipient_type))
            .then_with(|| self.sender_type.cmp(&other.sender_type))
            .then_with(|| self.flags.cmp(&other.flags))
            .then_with(|| self.data.len().cmp(&other.data.len()))
            .then_with(|| self.data.cmp(&other.data))
    }

    pub fn cmp_block_order(&self, other: &Transaction) -> Ordering {
        Ordering::Equal
            .then_with(|| self.recipient.cmp(&other.recipient))
            .then_with(|| self.validity_start_height.cmp(&other.validity_start_height))
            .then_with(|| other.fee.cmp(&self.fee))
            .then_with(|| other.value.cmp(&self.value))
            .then_with(|| self.sender.cmp(&other.sender))
            .then_with(|| self.recipient_type.cmp(&other.recipient_type))
            .then_with(|| self.sender_type.cmp(&other.sender_type))
            .then_with(|| self.flags.cmp(&other.flags))
            .then_with(|| self.data.len().cmp(&other.data.len()))
            .then_with(|| self.data.cmp(&other.data))
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

        if self.network_id != network_id {
            return Err(TransactionError::ForeignNetwork);
        }

        // Check that value > 0 except if it is a signalling transaction..
        if self.flags.contains(TransactionFlags::SIGNALLING) {
            if self.value != Coin::ZERO {
                return Err(TransactionError::InvalidForRecipient);
            }
        } else if self.value == Coin::ZERO {
            return Err(TransactionError::ZeroValue);
        }

        // Check that value + fee doesn't overflow.
        match self.value.checked_add(self.fee) {
            Some(coin) => {
                if coin > Coin::from_u64_unchecked(policy::TOTAL_SUPPLY) {
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
        let window = policy::TRANSACTION_VALIDITY_WINDOW;
        block_height >= self.validity_start_height
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
        let mut res: Vec<u8> = self.data.serialize_to_vec::<u16>();
        res.append(&mut self.sender.serialize_to_vec());
        res.append(&mut self.sender_type.serialize_to_vec());
        res.append(&mut self.recipient.serialize_to_vec());
        res.append(&mut self.recipient_type.serialize_to_vec());
        res.append(&mut self.value.serialize_to_vec());
        res.append(&mut self.fee.serialize_to_vec());
        res.append(&mut self.validity_start_height.serialize_to_vec());
        res.append(&mut self.network_id.serialize_to_vec());
        res.append(&mut self.flags.serialize_to_vec());
        res
    }

    pub fn total_value(&self) -> Result<Coin, TransactionError> {
        self.value
            .checked_add(self.fee)
            .ok_or(TransactionError::Overflow)
    }
}

impl Serialize for Transaction {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self.format() {
            TransactionFormat::Basic => {
                let signature_proof = SignatureProof::deserialize_from_vec(&self.proof)?;
                let mut size = 0;
                size += Serialize::serialize(&TransactionFormat::Basic, writer)?;
                size += Serialize::serialize(&signature_proof.public_key, writer)?;
                size += Serialize::serialize(&self.recipient, writer)?;
                size += Serialize::serialize(&self.value, writer)?;
                size += Serialize::serialize(&self.fee, writer)?;
                size += Serialize::serialize(&self.validity_start_height, writer)?;
                size += Serialize::serialize(&self.network_id, writer)?;
                size += Serialize::serialize(&signature_proof.signature, writer)?;
                Ok(size)
            }
            TransactionFormat::Extended => {
                let mut size = 0;
                size += Serialize::serialize(&TransactionFormat::Extended, writer)?;
                size += SerializeWithLength::serialize::<u16, W>(&self.data, writer)?;
                size += Serialize::serialize(&self.sender, writer)?;
                size += Serialize::serialize(&self.sender_type, writer)?;
                size += Serialize::serialize(&self.recipient, writer)?;
                size += Serialize::serialize(&self.recipient_type, writer)?;
                size += Serialize::serialize(&self.value, writer)?;
                size += Serialize::serialize(&self.fee, writer)?;
                size += Serialize::serialize(&self.validity_start_height, writer)?;
                size += Serialize::serialize(&self.network_id, writer)?;
                size += Serialize::serialize(&self.flags, writer)?;
                size += SerializeWithLength::serialize::<u16, W>(&self.proof, writer)?;
                Ok(size)
            }
        }
    }

    fn serialized_size(&self) -> usize {
        match self.format() {
            TransactionFormat::Basic => {
                let signature_proof = SignatureProof::deserialize_from_vec(&self.proof).unwrap();
                let mut size = 1;
                size += Serialize::serialized_size(&signature_proof.public_key);
                size += Serialize::serialized_size(&self.recipient);
                size += Serialize::serialized_size(&self.value);
                size += Serialize::serialized_size(&self.fee);
                size += Serialize::serialized_size(&self.validity_start_height);
                size += Serialize::serialized_size(&self.network_id);
                size += Serialize::serialized_size(&signature_proof.signature);
                size
            }
            TransactionFormat::Extended => {
                let mut size = 1;
                size += SerializeWithLength::serialized_size::<u16>(&self.data);
                size += Serialize::serialized_size(&self.sender);
                size += Serialize::serialized_size(&self.sender_type);
                size += Serialize::serialized_size(&self.recipient);
                size += Serialize::serialized_size(&self.recipient_type);
                size += Serialize::serialized_size(&self.value);
                size += Serialize::serialized_size(&self.fee);
                size += Serialize::serialized_size(&self.validity_start_height);
                size += Serialize::serialized_size(&self.network_id);
                size += Serialize::serialized_size(&self.flags);
                size += SerializeWithLength::serialized_size::<u16>(&self.proof);
                size
            }
        }
    }
}

impl Deserialize for Transaction {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let transaction_type: TransactionFormat = Deserialize::deserialize(reader)?;
        match transaction_type {
            TransactionFormat::Basic => {
                let sender_public_key: PublicKey = Deserialize::deserialize(reader)?;
                Ok(Transaction {
                    data: vec![],
                    sender: Address::from(&sender_public_key),
                    sender_type: AccountType::Basic,
                    recipient: Deserialize::deserialize(reader)?,
                    recipient_type: AccountType::Basic,
                    value: Deserialize::deserialize(reader)?,
                    fee: Deserialize::deserialize(reader)?,
                    validity_start_height: Deserialize::deserialize(reader)?,
                    network_id: Deserialize::deserialize(reader)?,
                    flags: TransactionFlags::empty(),
                    proof: SignatureProof::from(
                        sender_public_key,
                        Deserialize::deserialize(reader)?,
                    )
                    .serialize_to_vec(),
                    valid: false,
                })
            }
            TransactionFormat::Extended => Ok(Transaction {
                data: DeserializeWithLength::deserialize::<u16, R>(reader)?,
                sender: Deserialize::deserialize(reader)?,
                sender_type: Deserialize::deserialize(reader)?,
                recipient: Deserialize::deserialize(reader)?,
                recipient_type: Deserialize::deserialize(reader)?,
                value: Deserialize::deserialize(reader)?,
                fee: Deserialize::deserialize(reader)?,
                validity_start_height: Deserialize::deserialize(reader)?,
                network_id: Deserialize::deserialize(reader)?,
                flags: Deserialize::deserialize(reader)?,
                proof: DeserializeWithLength::deserialize::<u16, R>(reader)?,
                valid: false,
            }),
        }
    }
}

impl SerializeContent for Transaction {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        size += SerializeWithLength::serialize::<u16, W>(&self.data, writer)?;
        size += Serialize::serialize(&self.sender, writer)?;
        size += Serialize::serialize(&self.sender_type, writer)?;
        size += Serialize::serialize(&self.recipient, writer)?;
        size += Serialize::serialize(&self.recipient_type, writer)?;
        size += Serialize::serialize(&self.value, writer)?;
        size += Serialize::serialize(&self.fee, writer)?;
        size += Serialize::serialize(&self.validity_start_height, writer)?;
        size += Serialize::serialize(&self.network_id, writer)?;
        size += Serialize::serialize(&self.flags, writer)?;
        Ok(size)
    }
}

impl Hash for Transaction {}

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
            && self.data == other.data
    }
}

impl PartialOrd for Transaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Transaction {
    fn cmp(&self, other: &Self) -> Ordering {
        self.cmp_mempool_order(other)
    }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum TransactionError {
    #[error("Transaction is for a foreign network")]
    ForeignNetwork,
    #[error("Transaction has 0 value")]
    ZeroValue,
    #[error("Transaction has invalid value")]
    InvalidValue,
    #[error("Overflow")]
    Overflow,
    #[error("Sender same as recipient")]
    SenderEqualsRecipient,
    #[error("Transaction is invalid for sender")]
    InvalidForSender,
    #[error("Invalid transaction proof")]
    InvalidProof,
    #[error("Transaction is invalid for recipient")]
    InvalidForRecipient,
    #[error("Invalid transaction data")]
    InvalidData,
    #[error("Invalid serialization: {0}")]
    InvalidSerialization(#[from] SerializingError),
}
