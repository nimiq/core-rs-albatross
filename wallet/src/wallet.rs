use beserial::{Deserialize, Serialize, SerializingError, ReadBytesExt};
use keys::{KeyPair, PrivateKey, PublicKey, Address, Signature};
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::{Transaction, SignatureProof};


#[derive(Debug, Clone, Serialize)]
pub struct Wallet {
    pub key_pair: KeyPair,
    #[beserialize(skip)]
    pub address: Address,
}


impl Wallet {
    pub fn generate() -> Self {
        Self(KeyPair::generate())
    }

    pub fn create_transaction(&self, recipient: Address, value: Coin, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Transaction {
        let mut transaction = Transaction::new_basic(self.address.clone(), recipient, value, fee, validity_start_height, network_id);
        transaction.proof = self.sign_transaction(&transaction).serialize_to_vec();
        transaction
    }

    pub fn sign_transaction(&self, transaction: &Transaction) -> SignatureProof {
        let signature = self.key_pair.sign(transaction.serialize_content().as_slice());
        SignatureProof::single_sig(self.key_pair.public, signature)
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
        Self {
            key_pair,
            address: key_pair.into()
        }
    }
}