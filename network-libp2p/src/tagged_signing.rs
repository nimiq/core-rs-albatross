//! # TODO
//!
//! - Move th=is to somewhere appropriate (utils maybe).
//! - We have something similar for BLS signatures. This trait might be used there too. In general this can be
//!   used for all kinds of signature.
//!

use std::{
    io::{Cursor, Write},
    marker::PhantomData,
};

use libp2p::identity::{Keypair, PublicKey};

use beserial::{Serialize, Deserialize};


/// A trait for objects that can be signed. You have to choose an unique `TAG` that is used as prefix for
/// the message that will be signed. This is used to avoid replay attacks.
///
/// This also allows to use have typed signatures so that they can't be mixed up accidentally.
///
/// # Tags
///
/// Please document the tags used here to avoid collisions:
///
///  - `0x01`: [`ChallengeNonce`](nimiq_network_libp2p::discovery::protocol::ChallengeNonce)
///  - `0x02`: [`PeerContact`](nimiq_network_libp2p::discovery::peer_contacts::PeerContact)
///
pub trait TaggedSignable: Serialize {
    const TAG: u8;

    fn message_data(&self) -> Vec<u8> {
        let n = self.serialized_size();

        let mut buf = Cursor::new(Vec::with_capacity(n + 1));

        let tag = [Self::TAG; 1];
        buf.write(&tag).expect("Failed to write tag");
        self.serialize(&mut buf).expect("Failed to serialize message");

        buf.into_inner()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TaggedSignature<S: TaggedSignable> {
    #[beserial(len_type(u8))]
    data: Vec<u8>,

    #[beserial(skip)]
    _tagged: PhantomData<S>,
}

impl<S: TaggedSignable> TaggedSignature<S> {
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            _tagged: PhantomData,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

impl<S: TaggedSignable> std::fmt::Debug for TaggedSignature<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.data.fmt(f)
    }
}

impl<S: TaggedSignable> AsRef<[u8]> for TaggedSignature<S> {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

pub trait TaggedKeypair {
    fn sign(&self, message: &[u8]) -> Vec<u8>;

    fn tagged_sign<S: TaggedSignable>(&self, message: &S) -> TaggedSignature<S> {
        let data = self.sign(&message.message_data());

        TaggedSignature::from_bytes(data)
    }
}

pub trait TaggedPublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool;

    fn tagged_verify<S: TaggedSignable>(&self, message: &S, signature: &TaggedSignature<S>) -> bool {
        self.verify(&message.message_data(), &signature.data)
    }
}


impl TaggedKeypair for Keypair {
    fn sign(&self, message: &[u8]) -> Vec<u8> {
        Keypair::sign(&self, message)
            .expect("Signing failed")
    }
}

impl TaggedPublicKey for PublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        PublicKey::verify(self, msg, sig)
    }
}


#[cfg(test)]
mod tests {
    use libp2p::identity::{PublicKey, Keypair};

    use beserial::{Serialize, Deserialize};

    use super::{TaggedKeypair, TaggedSignature, TaggedPublicKey, TaggedSignable};

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Message(u64);

    impl TaggedSignable for Message {
        const TAG: u8 = 0x01;
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct AnotherMessage(u64);

    impl TaggedSignable for AnotherMessage {
        const TAG: u8 = 0x02;
    }

    #[test]
    fn it_signs_and_verifies() {
        let msg = Message(42);

        let keypair = Keypair::generate_ed25519();

        let sig = keypair.tagged_sign(&msg);

        assert!(keypair.public().tagged_verify(&msg, &sig))
    }

    #[test]
    fn message_data_is_different() {
        let msg1 = Message(42);
        let msg2 = AnotherMessage(42);

        assert_eq!(msg1.serialize_to_vec(), msg2.serialize_to_vec());
        assert_ne!(msg1.message_data(), msg2.message_data());
    }

    #[test]
    fn it_rejects_signatures_from_different_message_types() {
        let msg1 = Message(42);
        let msg2 = AnotherMessage(42);

        // The messages serialize to the same data and could be used for replay attacks in an untagged signature scheme.
        assert_eq!(msg1.serialize_to_vec(), msg2.serialize_to_vec());

        let keypair = Keypair::generate_ed25519();

        let sig1 = keypair.tagged_sign(&msg1);
        let sig2 = keypair.tagged_sign(&msg2);

        // This should still work, just making sure
        assert!(keypair.public().tagged_verify(&msg1, &sig1));
        assert!(keypair.public().tagged_verify(&msg2, &sig2));

        // First of all the signatures should be different. But the would anyway for non-deterministic signature
        // schemes.
        assert_ne!(sig1.data, sig2.data);

        // To even simulate a replay attack, we need to first craft new `TaggedSignature`s with correct types.
        // Otherwise otherwise the compiler will already prevent this ;)
        let sig1_replayed = TaggedSignature::<AnotherMessage>::from_bytes(sig1.data);
        let sig2_replayed = TaggedSignature::<Message>::from_bytes(sig2.data);


        // But even though we signed messages that serialize to the same data, the signature of one message must not
        // verify the other message.
        assert!(!keypair.public().tagged_verify(&msg1, &sig2_replayed));
        assert!(!keypair.public().tagged_verify(&msg2, &sig1_replayed));
    }
}
