//! # TODO
//!
//! - Move this to somewhere appropriate (utils maybe).
//! - We have something similar for BLS signatures. This trait might be used there too. In general this can be
//!   used for all kinds of signature.
//!

use std::{
    io::{Cursor, Write},
    marker::PhantomData,
};

use nimiq_serde::{Deserialize, Serialize};

/// A trait for objects that can be signed. You have to choose an unique `TAG` that is used as prefix for
/// the message that will be signed. This is used to avoid replay attacks.
///
/// This also allows to use have typed signatures so that they can't be mixed up accidentally.
///
/// # Tags
///
/// Please document the tags used here to avoid collisions:
///
///  - `0x01`: [`ChallengeNonce`](../../nimiq_network_libp2p/discovery/protocol/struct.ChallengeNonce.html)
///  - `0x02`: [`PeerContact`](../../nimiq_network_libp2p/discovery/peer_contacts/struct.PeerContact.html)
///  - `0x03`: [`ValidatorRecord`](../../nimiq_validator_network/validator_record/struct.ValidatorRecord.html)
///
pub trait TaggedSignable: Serialize {
    const TAG: u8;

    fn message_data(&self) -> Vec<u8> {
        let n = self.serialized_size();

        let mut buf = Cursor::new(Vec::with_capacity(n + 1));

        let tag = [Self::TAG; 1];
        buf.write_all(&tag).expect("Failed to write tag");
        self.serialize_to_writer(&mut buf)
            .expect("Failed to serialize message");

        buf.into_inner()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TaggedSignature<TSignable, TScheme>
where
    TSignable: TaggedSignable,
    TScheme: TaggedKeypair,
{
    data: Vec<u8>,

    _tagged: PhantomData<TSignable>,
    _scheme: PhantomData<TScheme>,
}

impl<TSignable, TScheme> PartialEq for TaggedSignature<TSignable, TScheme>
where
    TSignable: TaggedSignable,
    TScheme: TaggedKeypair,
{
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl<TSignable, TScheme> Eq for TaggedSignature<TSignable, TScheme>
where
    TSignable: TaggedSignable,
    TScheme: TaggedKeypair,
{
}

impl<TSignable, TScheme> TaggedSignature<TSignable, TScheme>
where
    TSignable: TaggedSignable,
    TScheme: TaggedKeypair,
{
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            _tagged: PhantomData,
            _scheme: PhantomData,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn tagged_verify(&self, message: &TSignable, public_key: &TScheme::PublicKey) -> bool {
        public_key.verify(&message.message_data(), &self.data)
    }
}

impl<TSignable, TScheme> std::fmt::Debug for TaggedSignature<TSignable, TScheme>
where
    TSignable: TaggedSignable,
    TScheme: TaggedKeypair,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.data.fmt(f)
    }
}

impl<TSignable, TScheme> AsRef<[u8]> for TaggedSignature<TSignable, TScheme>
where
    TSignable: TaggedSignable,
    TScheme: TaggedKeypair,
{
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

pub trait TaggedKeypair: Sized {
    type PublicKey: TaggedPublicKey;

    fn sign(&self, message: &[u8]) -> Vec<u8>;

    fn tagged_sign<TSignable>(&self, message: &TSignable) -> TaggedSignature<TSignable, Self>
    where
        TSignable: TaggedSignable,
    {
        let data = self.sign(&message.message_data());

        TaggedSignature::from_bytes(data)
    }
}

pub trait TaggedPublicKey {
    fn verify(&self, msg: &[u8], sig: &[u8]) -> bool;
}

#[cfg(test)]
mod tests {
    use nimiq_keys::{KeyPair, PublicKey, SecureGenerate, Signature};
    use nimiq_serde::{Deserialize, Serialize};
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng::test_rng;

    use super::{TaggedKeypair, TaggedPublicKey, TaggedSignable, TaggedSignature};

    struct TestKeypair(KeyPair);
    struct TestPublicKey(PublicKey);

    impl TestKeypair {
        pub fn generate() -> Self {
            Self(KeyPair::generate(&mut test_rng(false)))
        }

        pub fn public_key(&self) -> TestPublicKey {
            TestPublicKey(self.0.public)
        }
    }

    impl TaggedKeypair for TestKeypair {
        type PublicKey = TestPublicKey;

        fn sign(&self, message: &[u8]) -> Vec<u8> {
            self.0.sign(message).as_bytes().to_vec()
        }
    }

    impl TaggedPublicKey for TestPublicKey {
        fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
            self.0.verify(&Signature::from_bytes(sig).unwrap(), msg)
        }
    }

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

        let keypair = TestKeypair::generate();

        let sig = keypair.tagged_sign(&msg);

        assert!(sig.tagged_verify(&msg, &keypair.public_key()));
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

        let keypair = TestKeypair::generate();

        let sig1 = keypair.tagged_sign(&msg1);
        let sig2 = keypair.tagged_sign(&msg2);

        // This should still work, just making sure
        assert!(sig1.tagged_verify(&msg1, &keypair.public_key()));
        assert!(sig2.tagged_verify(&msg2, &keypair.public_key()));

        // First of all the signatures should be different. But the would anyway for non-deterministic signature
        // schemes.
        assert_ne!(sig1.data, sig2.data);

        // To even simulate a replay attack, we need to first craft new `TaggedSignature`s with correct types.
        // Otherwise otherwise the compiler will already prevent this ;)
        let sig1_replayed = TaggedSignature::<AnotherMessage, TestKeypair>::from_bytes(sig1.data);
        let sig2_replayed = TaggedSignature::<Message, TestKeypair>::from_bytes(sig2.data);

        // But even though we signed messages that serialize to the same data, the signature of one message must not
        // verify the other message.
        assert!(!sig2_replayed.tagged_verify(&msg1, &keypair.public_key()));
        assert!(!sig1_replayed.tagged_verify(&msg2, &keypair.public_key()));
    }
}

#[cfg(feature = "libp2p")]
mod impl_for_libp2p {
    use libp2p_core::identity::{Keypair, PublicKey};

    use super::{TaggedKeypair, TaggedPublicKey};

    impl TaggedKeypair for Keypair {
        type PublicKey = PublicKey;

        fn sign(&self, message: &[u8]) -> Vec<u8> {
            Keypair::sign(self, message).expect("Signing failed")
        }
    }

    impl TaggedPublicKey for PublicKey {
        fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
            PublicKey::verify(self, msg, sig)
        }
    }
}

#[cfg(feature = "nimiq-keys")]
mod impl_for_keys {
    use nimiq_keys::{KeyPair, PublicKey, Signature};

    impl TaggedKeypair for KeyPair {
        type PublicKey = PublicKey;

        fn sign(&self, message: &[u8]) -> Vec<u8> {
            self.0.sign(message).as_bytes().to_owned()
        }
    }

    impl TaggedPublicKey for PublicKey {
        fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
            self.verify(&Signature::from_bytes(sig).unwrap(), msg)
        }
    }
}
