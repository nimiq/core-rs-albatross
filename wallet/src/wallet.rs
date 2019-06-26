use std::io;

use beserial::{Deserialize, Serialize, SerializingError, ReadBytesExt};
use keys::{KeyPair, Address};
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::{Transaction, SignatureProof};
use database::{FromDatabaseValue, IntoDatabaseValue};


#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Wallet {
    pub key_pair: KeyPair,
    #[beserial(skip)]
    pub address: Address,
}


impl Wallet {
    pub fn generate() -> Self {
        Wallet::from(KeyPair::generate())
    }

    pub fn create_transaction(&self, recipient: Address, value: Coin, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Transaction {
        let mut transaction = Transaction::new_basic(self.address.clone(), recipient, value, fee, validity_start_height, network_id);
        transaction.proof = self.sign_transaction(&transaction).serialize_to_vec();
        transaction
    }

    pub fn sign_transaction(&self, transaction: &Transaction) -> SignatureProof {
        let signature = self.key_pair.sign(transaction.serialize_content().as_slice());
        SignatureProof::from(self.key_pair.public, signature)
    }
}

impl Deserialize for Wallet {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let key_pair: KeyPair = Deserialize::deserialize(reader)?;
        Ok(Wallet::from(key_pair))
    }
}

impl From<KeyPair> for Wallet {
    fn from(key_pair: KeyPair) -> Self {
        let address = Address::from(&key_pair);
        Self {
            key_pair,
            address,
        }
    }
}

impl IntoDatabaseValue for Wallet {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for Wallet {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
