use beserial::{Serialize, Deserialize, uvar};
use nimiq_bls::bls12_381::{Signature, SecretKey, PublicKey};
use nimiq_bls::SigHash;
use hash::{Blake2bHasher, Blake2bHash, SerializeContent, Hasher, Hash};
use beserial::WriteBytesExt;

use std::fmt::Debug;
use std::io::Write;


// TODO: To use a binomial swap forest:
//  * instead of pk_idx use a bitvec
//  * instead of signature use a AggregateSignature
//  * (all, not just active) validators are identifiable by ID which has the common prefix metric
//  * The sender of a SignedValidatorMessage includes it's ID
//  * Add other messages (authenticated) for: Invite, Later, No


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedMessage<M: Message> {
    // the signed message
    //  - view change: (VIEW-CHANGE, i + 1, b)
    //    - i: current view change number
    //    - b: current block number
    //  - pbft-prepare: (PREPARE, h)
    //  - pbft-commit: (COMMIT, h)
    //    - h: block hash -> Blake2bHash
    // X the actual message doesn't contain the prefix. This is added only during signing
    pub message: M,

    // index of public key of signer
    // XXX they need to be indexable, because we will include a bitmap of all signers in the block
    pub pk_idx: uvar,

    // signature over message
    pub signature: Signature
}

impl<M: Message> SignedMessage<M> {
    pub fn verify(&self, public_key: &PublicKey) -> bool {
        public_key.verify_hash(self.message.hash_with_prefix(), &self.signature)
    }
}


// XXX The contents of ViewChangeMessage and PbftMessage (and any other message that is signed by
// a validator) must be distinguishable!
// Therefore all signed messages should be prefixed with a standarized type. We should keep those
// prefixed at one place to not accidently create collisions.

// prefix to sign view change messages
pub const PREFIX_VIEW_CHANGE: u8 = 0x01;
// prefix to sign pbft-prepare messages
pub const PREFIX_PBFT_PREPARE: u8 = 0x02;
// prefix to sign pbft-commit messages
pub const PREFIX_PBFT_COMMIT: u8 = 0x03;
// prefix to sign proof of knowledge of secret key
pub const PREFIX_POKOSK: u8 = 0x04;

pub trait Message: Clone + Debug + Serialize + Deserialize + SerializeContent {
    const PREFIX: u8;

    fn hash_with_prefix(&self) -> SigHash {
        let mut h = Blake2bHasher::new();
        h.write_u8(Self::PREFIX).expect("Failed to write prefix to hasher for signature.");
        self.serialize_content(&mut h).expect("Failed to write message to hasher for signature.");
        h.finish()
    }

    fn sign(&self, secret_key: &SecretKey) -> Signature {
        secret_key.sign_hash(self.hash_with_prefix())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent)]
pub struct PbftPrepareMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}

impl Message for PbftPrepareMessage {
    const PREFIX: u8 = PREFIX_PBFT_PREPARE;
}

#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent)]
pub struct PbftCommitMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}

impl Message for PbftCommitMessage {
    const PREFIX: u8 = PREFIX_PBFT_COMMIT;
}
