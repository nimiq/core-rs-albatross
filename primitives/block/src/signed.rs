use std::fmt::Debug;

use byteorder::WriteBytesExt;
use nimiq_bls::{PublicKey, SecretKey, SigHash, Signature};
use nimiq_hash::{Blake2sHash, Blake2sHasher, Hasher, SerializeContent};
use nimiq_serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound = "M: Message")]
pub struct SignedMessage<M: Message> {
    // The signed message. Note that the actual message doesn't contain the prefix. This is added
    // only during signing
    pub message: M,

    // The index of public key of signer. Signers need to be indexable, because we will include a
    // bitmap of all signers in the block.
    pub signer_idx: u16,

    // The signature over the message.
    pub signature: Signature,
}

impl<M: Message> SignedMessage<M> {
    /// Verify signed message
    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify_hash(self.message.hash_with_prefix(), &self.signature)
    }

    /// Create SignedMessage from message.
    pub fn from_message(message: M, secret_key: &SecretKey, signer_idx: u16) -> Self {
        let signature = message.sign(secret_key);
        Self {
            message,
            signer_idx,
            signature,
        }
    }
}

// The contents of ViewChangeMessage and TendermintMessage (and any other message that is signed by
// a validator) must be distinguishable!
// Therefore all signed messages should be prefixed with a standardized type. We should keep those
// prefixes at one place to not accidentally create collisions.

/// prefix to sign skip block info messages
pub const PREFIX_SKIP_BLOCK_INFO: u8 = 0x01;
/// prefix to sign a tendermint proposal
pub const PREFIX_TENDERMINT_PROPOSAL: u8 = 0x02;
/// prefix to sign tendermint prepare messages
pub const PREFIX_TENDERMINT_PREPARE: u8 = 0x03;
/// prefix to sign tendermint commit messages
pub const PREFIX_TENDERMINT_COMMIT: u8 = 0x04;
/// prefix to sign proof of knowledge of secret key
pub const PREFIX_POKOSK: u8 = 0x05;
/// prefix to sign a validator info
pub const PREFIX_VALIDATOR_INFO: u8 = 0x06;

pub trait Message:
    Clone
    + Debug
    + Serialize
    + Deserialize
    + SerializeContent
    + Send
    + Sync
    + Sized
    + PartialEq
    + 'static
{
    const PREFIX: u8;

    fn hash_with_prefix(&self) -> SigHash {
        let mut h = Blake2sHasher::new();
        h.write_u8(Self::PREFIX)
            .expect("Failed to write prefix to hasher for signature.");
        self.serialize_content::<_, Blake2sHash>(&mut h)
            .expect("Failed to write message to hasher for signature.");
        h.finish()
    }

    fn sign(&self, secret_key: &SecretKey) -> Signature {
        secret_key.sign_hash(self.hash_with_prefix())
    }
}
