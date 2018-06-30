use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, WriteBytesExt};
use consensus::base::account::AccountType;
use consensus::base::primitive::Address;
use consensus::base::primitive::crypto::{PublicKey, Signature};
use consensus::base::primitive::hash::{Hash, SerializeContent};
use std::io;
use utils::merkle::Blake2bMerklePath;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum TransactionType {
    Basic = 0,
    Extended = 1,
}

#[derive(Serialize, Deserialize)]
pub struct SignatureProof {
    pub public_key: PublicKey,
    pub merkle_path: Blake2bMerklePath,
    pub signature: Signature,
}

impl SignatureProof {
    fn from(public_key: PublicKey, signature: Signature) -> Self {
        return SignatureProof {
            public_key,
            merkle_path: Blake2bMerklePath::empty(),
            signature,
        };
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
#[repr(C)]
pub struct Transaction {
    pub data: Vec<u8>,
    pub sender: Address,
    pub sender_type: AccountType,
    pub recipient: Address,
    pub recipient_type: AccountType,
    pub value: u64,
    pub fee: u64,
    pub validity_start_height: u32,
    pub network_id: u8,
    pub flags: u8,
    pub proof: Vec<u8>,
}

impl Transaction {
    fn transaction_type(&self) -> TransactionType {
        if let Result::Ok(signature_proof) = SignatureProof::deserialize_from_vec(&self.proof) {
            if self.sender_type == AccountType::Basic &&
                self.recipient_type == AccountType::Basic &&
                self.data.len() == 0 &&
                self.flags == 0 &&
                self.sender == Address::from(&signature_proof.public_key) &&
                signature_proof.merkle_path.len() == 0 {
                return TransactionType::Basic;
            }
        }
        return TransactionType::Extended;
    }
}

impl Serialize for Transaction {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        match self.transaction_type() {
            TransactionType::Basic => {
                let signature_proof = SignatureProof::deserialize_from_vec(&self.proof)?;
                let mut size = 0;
                size += Serialize::serialize(&TransactionType::Basic, writer)?;
                size += Serialize::serialize(&signature_proof.public_key, writer)?;
                size += Serialize::serialize(&self.recipient, writer)?;
                size += Serialize::serialize(&self.value, writer)?;
                size += Serialize::serialize(&self.fee, writer)?;
                size += Serialize::serialize(&self.validity_start_height, writer)?;
                size += Serialize::serialize(&self.network_id, writer)?;
                size += Serialize::serialize(&signature_proof.signature, writer)?;
                return Ok(size);
            }
            TransactionType::Extended => {
                let mut size = 0;
                size += Serialize::serialize(&TransactionType::Extended, writer)?;
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
        match self.transaction_type() {
            TransactionType::Basic => {
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
            TransactionType::Extended => {
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
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let transaction_type: TransactionType = Deserialize::deserialize(reader)?;
        match transaction_type {
            TransactionType::Basic => {
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
                    flags: 0,
                    proof: SignatureProof::from(sender_public_key.clone(), Deserialize::deserialize(reader)?).serialize_to_vec(),
                });
            }
            TransactionType::Extended => {
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
        return Ok(size);
    }
}

impl Hash for Transaction {}
