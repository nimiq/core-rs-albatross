use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, WriteBytesExt};
use consensus::base::account::{Account, AccountType};
use consensus::base::primitive::Address;
use consensus::base::primitive::crypto::{PublicKey, Signature};
use consensus::base::primitive::hash::{Hash, SerializeContent, Blake2bHash};
use consensus::networks::NetworkId;
use std::cmp::{Ord, Ordering};
use std::io;
use utils::merkle::Blake2bMerklePath;
use consensus::base::primitive::coin::Coin;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum TransactionType {
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

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
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
    pub fn new_basic(sender: Address, recipient: Address, value: Coin, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Self {
        return Transaction {
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

    fn transaction_type(&self) -> TransactionType {
        if let Result::Ok(signature_proof) = SignatureProof::deserialize_from_vec(&self.proof) {
            if self.sender_type == AccountType::Basic &&
                self.recipient_type == AccountType::Basic &&
                self.data.len() == 0 &&
                self.flags.is_empty() &&
                self.sender == Address::from(&signature_proof.public_key) &&
                signature_proof.merkle_path.len() == 0 {
                return TransactionType::Basic;
            }
        }
        return TransactionType::Extended;
    }

    pub fn cmp_block_order(&self, tx: &Transaction) -> Ordering {
        return Ordering::Equal
            .then_with(|| self.recipient.cmp(&tx.recipient))
            .then_with(|| self.validity_start_height.cmp(&tx.validity_start_height))
            .then_with(|| self.fee.cmp(&tx.fee))
            .then_with(|| self.value.cmp(&tx.value))
            .then_with(|| self.sender.cmp(&tx.sender))
            .then_with(|| self.recipient_type.cmp(&tx.recipient_type))
            .then_with(|| self.sender_type.cmp(&tx.sender_type))
            .then_with(|| self.flags.cmp(&tx.flags));
    }

    pub fn verify(&self) -> bool {
        // TODO Check network id.

        // Check that sender != recipient.
        if self.recipient == self.sender {
            return false;
        }

        // Check that value > 0.
        if self.value == Coin::ZERO {
            return false;
        }

        // Check that value + fee doesn't overflow.
        // TODO also check max supply?
        if self.value.checked_add(self.fee).is_none() {
            return false;
        }

        // TODO Check account types valid?

        // Check transaction validity for sender account.
        if !Account::verify_outgoing_transaction(&self) {
            return false;
        }

        // Check transaction validity for recipient account.
        if !Account::verify_incoming_transaction(&self) {
            return false;
        }

        return true;
    }

    pub fn contract_creation_address(&self) -> Address {
        let mut tx = self.clone();
        tx.recipient = Address::from([0u8; Address::SIZE]);
        let hash: Blake2bHash = tx.hash();
        return Address::from(hash);
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
                    flags: TransactionFlags::empty(),
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
