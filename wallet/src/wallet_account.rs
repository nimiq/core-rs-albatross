use std::io;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError};
use database::{FromDatabaseValue, IntoDatabaseValue};
use keys::{Address, KeyPair, PublicKey, SecureGenerate, Signature};
use nimiq_hash::{Hash, Sha256Hash};
use nimiq_utils::otp::Verify;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::{SignatureProof, Transaction};

pub const NIMIQ_SIGN_MESSAGE_PREFIX: &[u8] = b"\x16Nimiq Signed Message:\n";

#[derive(Default, Debug, Clone, Serialize, PartialEq)]
pub struct WalletAccount {
    pub key_pair: KeyPair,
    #[beserial(skip)]
    pub address: Address,
}

impl Verify for WalletAccount {
    fn verify(&self) -> bool {
        // Check that the public key corresponds to the private key.
        PublicKey::from(&self.key_pair.private) == self.key_pair.public
    }
}

impl WalletAccount {
    pub fn generate() -> Self {
        WalletAccount::from(KeyPair::generate_default_csprng())
    }

    pub fn create_transaction(&self, recipient: Address, value: Coin, fee: Coin, validity_start_height: u32, network_id: NetworkId) -> Transaction {
        let mut transaction = Transaction::new_basic(self.address.clone(), recipient, value, fee, validity_start_height, network_id);
        self.sign_transaction(&mut transaction);
        transaction
    }

    pub fn sign_transaction(&self, transaction: &mut Transaction) {
        let proof = self.create_signature_proof(transaction);
        transaction.proof = proof.serialize_to_vec();
    }

    pub fn create_signature_proof(&self, transaction: &Transaction) -> SignatureProof {
        let signature = self.key_pair.sign(transaction.serialize_content().as_slice());
        SignatureProof::from(self.key_pair.public, signature)
    }

    fn prepare_message_for_signature(message: &[u8]) -> Sha256Hash {
        /*
         * Adding a prefix to the message makes the calculated signature recognisable as
         * a Nimiq specific signature. This and the hashing prevents misuse where a malicious
         * request can sign arbitrary data (e.g. a transaction) and use the signature to
         * impersonate the victim. (https://github.com/ethereum/wiki/wiki/JSON-RPC#eth_sign)
         */
        let mut buffer = NIMIQ_SIGN_MESSAGE_PREFIX.to_vec();
        // Append length of message as encoded string.
        let mut encoded_len = message.len().to_string().into_bytes();
        buffer.append(&mut encoded_len);
        // Append actual message.
        buffer.extend_from_slice(message);

        // Hash and sign.
        buffer.hash::<Sha256Hash>()
    }

    pub fn sign_message(&self, message: &[u8]) -> (PublicKey, Signature) {
        let hash = Self::prepare_message_for_signature(message);
        (self.key_pair.public, self.key_pair.sign(hash.as_bytes()))
    }

    pub fn verify_message(public_key: &PublicKey, message: &[u8], signature: &Signature) -> bool {
        let hash = Self::prepare_message_for_signature(message);
        public_key.verify(signature, hash.as_bytes())
    }
}

impl Deserialize for WalletAccount {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let key_pair: KeyPair = Deserialize::deserialize(reader)?;
        Ok(WalletAccount::from(key_pair))
    }
}

impl From<KeyPair> for WalletAccount {
    fn from(key_pair: KeyPair) -> Self {
        let address = Address::from(&key_pair);
        Self {
            key_pair,
            address,
        }
    }
}

impl IntoDatabaseValue for WalletAccount {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for WalletAccount {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
