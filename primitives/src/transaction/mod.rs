use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializingError, SerializeWithLength, WriteBytesExt};
use crate::account::{Account, AccountTransactionInteraction, AccountType};
use crate::coin::Coin;
use keys::Address;
use keys::{PublicKey, Signature};
use hash::{Hash, SerializeContent, Blake2bHash};
use crate::networks::NetworkId;
use crate::policy;
use std::cmp::{Ord, Ordering};
use std::io;
use utils::merkle::Blake2bMerklePath;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum TransactionFormat {
    Basic = 0,
    Extended = 1,
}

bitflags! {
    #[derive(Default, Serialize, Deserialize)]
    pub struct TransactionFlags: u8 {
        const CONTRACT_CREATION = 0b1;
    }
}

#[derive(Serialize, Deserialize)]
pub struct SignatureProof {
    pub public_key: PublicKey,
    pub merkle_path: Blake2bMerklePath,
    pub signature: Signature,
}

impl SignatureProof {
    pub fn from(public_key: PublicKey, signature: Signature) -> Self {
        return SignatureProof {
            public_key,
            merkle_path: Blake2bMerklePath::empty(),
            signature,
        };
    }

    pub fn compute_signer(&self) -> Address {
        let merkle_root = self.merkle_path.compute_root(&self.public_key);
        return Address::from(merkle_root);
    }

    pub fn is_signed_by(&self, address: &Address) -> bool {
        return self.compute_signer() == *address;
    }

    pub fn verify(&self, data: &[u8]) -> bool {
        return self.public_key.verify(&self.signature, data);
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
}

impl Transaction {
    /// The size in bytes of the smallest possible transaction (basic single-sig).
    pub const MIN_SIZE: usize = 138;

    pub fn new_basic(sender: Address, recipient: Address, value: Coin, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Self {
        return Self {
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
            proof: Vec::new()
        };
    }

    pub fn new_contract_creation(data: Vec<u8>, sender: Address, sender_type: AccountType, recipient_type: AccountType, value: Coin, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Self {
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
            proof: Vec::new()
        };
        tx.recipient = tx.contract_creation_address();
        return tx;
    }

    pub fn format(&self) -> TransactionFormat {
        if self.sender_type == AccountType::Basic
            && self.recipient_type == AccountType::Basic
            && self.data.len() == 0
            && self.flags.is_empty() {

            if let Ok(signature_proof) = SignatureProof::deserialize_from_vec(&self.proof) {
                if self.sender == Address::from(&signature_proof.public_key)
                    && signature_proof.merkle_path.len() == 0 {
                    return TransactionFormat::Basic;
                }
            }
        }
        return TransactionFormat::Extended;
    }

    pub fn cmp_mempool_order(&self, other: &Transaction) -> Ordering {
        return Ordering::Equal
            .then_with(|| self.fee_per_byte().partial_cmp(&other.fee_per_byte()).unwrap_or(Ordering::Equal))
            .then_with(|| self.fee.cmp(&other.fee))
            .then_with(|| self.value.cmp(&other.value))
            .then_with(|| self.recipient.cmp(&other.recipient))
            .then_with(|| self.validity_start_height.cmp(&other.validity_start_height))
            .then_with(|| self.sender.cmp(&other.sender))
            .then_with(|| self.recipient_type.cmp(&other.recipient_type))
            .then_with(|| self.sender_type.cmp(&other.sender_type))
            .then_with(|| self.flags.cmp(&other.flags))
            .then_with(|| self.data.len().cmp(&other.data.len()))
            .then_with(|| self.data.cmp(&other.data));
    }

    pub fn cmp_block_order(&self, other: &Transaction) -> Ordering {
        return Ordering::Equal
            .then_with(|| self.recipient.cmp(&other.recipient))
            .then_with(|| self.validity_start_height.cmp(&other.validity_start_height))
            .then_with(|| other.fee.cmp(&self.fee))
            .then_with(|| other.value.cmp(&self.value))
            .then_with(|| self.sender.cmp(&other.sender))
            .then_with(|| self.recipient_type.cmp(&other.recipient_type))
            .then_with(|| self.sender_type.cmp(&other.sender_type))
            .then_with(|| self.flags.cmp(&other.flags))
            .then_with(|| self.data.len().cmp(&other.data.len()))
            .then_with(|| self.data.cmp(&other.data));
    }

    pub fn verify(&self, network_id: NetworkId) -> Result<(), TransactionError> {
        if self.network_id != network_id {
            return Err(TransactionError::ForeignNetwork);
        }

        // Check that sender != recipient.
        if self.recipient == self.sender {
            return Err(TransactionError::SenderEqualsRecipient);
        }

        // Check that value > 0.
        if self.value == Coin::ZERO {
            return Err(TransactionError::ZeroValue);
        }

        // Check that value + fee doesn't overflow.
        // TODO also check max supply?
        if self.value.checked_add(self.fee).is_none() {
            return Err(TransactionError::Overflow);
        }

        // TODO Check account types valid?

        // Check transaction validity for sender account.
        Account::verify_outgoing_transaction(&self)?;

        // Check transaction validity for recipient account.
        Account::verify_incoming_transaction(&self)?;

        return Ok(());
    }

    pub fn is_valid_at(&self, block_height: u32) -> bool {
        return block_height >= self.validity_start_height
            && block_height < self.validity_start_height + policy::TRANSACTION_VALIDITY_WINDOW;
    }

    pub fn contract_creation_address(&self) -> Address {
        let mut tx = self.clone();
        tx.recipient = Address::from([0u8; Address::SIZE]);
        let hash: Blake2bHash = tx.hash();
        return Address::from(hash);
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
        return res;
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
                return Ok(size);
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
                return Ok(size);
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
                return size;
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
                return size;
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
                return Ok(Transaction {
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
                    proof: SignatureProof::from(sender_public_key.clone(), Deserialize::deserialize(reader)?).serialize_to_vec(),
                });
            }
            TransactionFormat::Extended => {
                return Ok(Transaction {
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
                });
            }
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
        return Ok(size);
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

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub enum TransactionError {
    ForeignNetwork,
    ZeroValue,
    Overflow,
    SenderEqualsRecipient,
    InvalidForSender,
    InvalidProof,
    InvalidForRecipient,
    InvalidData,
    InvalidSerialization(SerializingError)
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // TODO: Don't use debug formatter
        write!(f, "{:?}", self)
    }
}

impl From<SerializingError> for TransactionError {
    fn from(e: SerializingError) -> Self {
        TransactionError::InvalidSerialization(e)
    }
}
