use std::io;

use beserial::{Serialize, Serialize8, Deserialize, Deserialize16, ReadBytesExt, WriteBytesExt};
use super::account::AccountType;
use super::primitive::Address;
use super::primitive::crypto::{PublicKey,Signature};

#[derive(Clone,Copy,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
#[repr(u8)]
pub enum TransactionType {
    Basic = 0,
    Extended = 1
}

pub struct MerklePathNode {}

impl Serialize for MerklePathNode {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}

pub struct MerklePath(Vec<MerklePathNode>);

impl Serialize for MerklePath {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        return self.0.serialize8(writer);
    }

    fn serialized_size(&self) -> usize {
        return self.0.serialize8d_size();
    }
}

#[derive(Serialize)]
pub struct SignatureProof<'a> {
    public_key: &'a PublicKey,
    merkle_path: MerklePath,
    signature: Signature
}

impl<'a> SignatureProof<'a> {
    fn from(public_key: &'a PublicKey, signature: Signature) -> Self {
        return SignatureProof {
            public_key,
            merkle_path: MerklePath(vec![]),
            signature
        };
    }
}

#[derive(Clone,PartialEq,PartialOrd,Eq,Ord,Debug)]
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
    pub proof: Vec<u8>
}

impl Deserialize for Transaction {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let transaction_type: TransactionType = Deserialize::deserialize(reader)?;
        return Ok(match transaction_type {
            TransactionType::Basic => {
                let sender_public_key: PublicKey = Deserialize::deserialize(reader)?;
                Transaction {
                    data: Vec::new(),
                    sender: Address::from(&sender_public_key),
                    sender_type: AccountType::Basic,
                    recipient: Deserialize::deserialize(reader)?,
                    recipient_type: AccountType::Basic,
                    value: Deserialize::deserialize(reader)?,
                    fee: Deserialize::deserialize(reader)?,
                    validity_start_height: Deserialize::deserialize(reader)?,
                    network_id: Deserialize::deserialize(reader)?,
                    flags: 0,
                    proof: SignatureProof::from(&sender_public_key, Deserialize::deserialize(reader)?).serialize_to_vec()
                }
            },
            TransactionType::Extended => {
                Transaction {
                    data: Deserialize16::deserialize16(reader)?,
                    sender: Deserialize::deserialize(reader)?,
                    sender_type: Deserialize::deserialize(reader)?,
                    recipient: Deserialize::deserialize(reader)?,
                    recipient_type: Deserialize::deserialize(reader)?,
                    value: Deserialize::deserialize(reader)?,
                    fee: Deserialize::deserialize(reader)?,
                    validity_start_height: Deserialize::deserialize(reader)?,
                    network_id: Deserialize::deserialize(reader)?,
                    flags: Deserialize::deserialize(reader)?,
                    proof: Deserialize16::deserialize16(reader)?,
                }
            }
        });
    }
}
