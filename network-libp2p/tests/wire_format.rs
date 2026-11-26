//! Golden vectors for the libp2p types that nimiq embeds in its own wire and disk formats.
//!
//! The expected bytes were produced with libp2p 0.56 (libp2p-identity 0.2, multiaddr 0.18), which
//! every release up to v2.2.0 ships. Peer contacts are signed over their serialized form and old
//! nodes decode discovery messages as a whole, so if a libp2p upgrade changes any of these
//! encodings, old and new nodes stop accepting each other's contacts. Keep these vectors unchanged
//! across libp2p upgrades.

use libp2p::{
    identity::{ed25519, Keypair},
    Multiaddr, PeerId,
};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use nimiq_network_libp2p::{
    discovery::{
        peer_contacts::{PeerContact, SignedPeerContact},
        protocol::{ChallengeNonce, DiscoveryMessage},
    },
    Libp2pKeyPair,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_utils::tagged_signing::TaggedKeyPair;

const PEER_KEY_FILE: &str = "42424242424242424242424242424242424242424242424242424242424242422152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12";
const PEER_ID: &str = "12D3KooWC4T1AXU2s2YBgGJ2FeaYVtsKoHZWJeubnWe9SnuSE7Zb";
const SIGNED_PEER_CONTACT: &str = "030a04cb0071070620fbdd03162920010db80000000000000000000000010620fbdd0318361173656564312e6578616d706c652e636f6d0601bbde032152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12230080c3bbc2064088f3d98d3d45476e06eaed25701fe186c90592e549f6e3ec645210ffbb288bcbf3898175c961f7166cf07f03fd192810898042c8e9497b7bdb5aa0b643d9c70f";
const HANDSHAKE: &str = "000a04c633641706c822dd0317171717171717171717171717171717171717171717171717171717171717175a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a408101";
const HANDSHAKE_ACK: &str = "01030a04cb0071070620fbdd03162920010db80000000000000000000000010620fbdd0318361173656564312e6578616d706c652e636f6d0601bbde032152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12230080c3bbc2064088f3d98d3d45476e06eaed25701fe186c90592e549f6e3ec645210ffbb288bcbf3898175c961f7166cf07f03fd192810898042c8e9497b7bdb5aa0b643d9c70f40377f7cd2ccef0e45850dc15a5bab083efc1498a7278eb5079b0124cc2b816d85e849f4f5530c985424dde808bed0bc069852a3dcd0f5b07d08660332f3295b0b013c01030a04cb0071070620fbdd03162920010db80000000000000000000000010620fbdd0318361173656564312e6578616d706c652e636f6d0601bbde032152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12230080c3bbc2064088f3d98d3d45476e06eaed25701fe186c90592e549f6e3ec645210ffbb288bcbf3898175c961f7166cf07f03fd192810898042c8e9497b7bdb5aa0b643d9c70f";
const PEER_ADDRESSES: &str = "0201030a04cb0071070620fbdd03162920010db80000000000000000000000010620fbdd0318361173656564312e6578616d706c652e636f6d0601bbde032152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12230080c3bbc2064088f3d98d3d45476e06eaed25701fe186c90592e549f6e3ec645210ffbb288bcbf3898175c961f7166cf07f03fd192810898042c8e9497b7bdb5aa0b643d9c70f";

fn keypair() -> Keypair {
    let secret = ed25519::SecretKey::try_from_bytes([0x42u8; 32]).unwrap();
    Keypair::from(ed25519::Keypair::from(secret))
}

fn signed_peer_contact(keypair: &Keypair) -> SignedPeerContact {
    let addresses: Vec<Multiaddr> = vec![
        "/ip4/203.0.113.7/tcp/8443/ws".parse().unwrap(),
        "/dns4/seed1.example.com/tcp/443/wss".parse().unwrap(),
        "/ip6/2001:db8::1/tcp/8443/ws".parse().unwrap(),
    ];
    PeerContact::new(
        addresses,
        keypair.public(),
        Services::FULL_BLOCKS | Services::HISTORY | Services::MEMPOOL,
        1_750_000_000,
    )
    .unwrap()
    .sign(keypair)
}

/// Checks that `message` serializes to `expected` and that `expected` decodes and re-serializes
/// to the same bytes.
fn assert_discovery_message(message: DiscoveryMessage, expected: &str) {
    assert_eq!(hex::encode(message.serialize_to_vec()), expected);

    let decoded = DiscoveryMessage::deserialize_from_vec(&hex::decode(expected).unwrap()).unwrap();
    assert_eq!(hex::encode(decoded.serialize_to_vec()), expected);
}

#[test]
fn peer_key_file_encoding_is_stable() {
    let keypair = keypair();
    assert_eq!(
        hex::encode(Libp2pKeyPair(keypair.clone()).serialize_to_vec()),
        PEER_KEY_FILE
    );
    assert_eq!(PeerId::from(keypair.public()).to_string(), PEER_ID);

    let decoded =
        Libp2pKeyPair::deserialize_from_vec(&hex::decode(PEER_KEY_FILE).unwrap()).unwrap();
    assert_eq!(decoded.0.public(), keypair.public());
}

#[test]
fn signed_peer_contact_encoding_is_stable() {
    let contact = signed_peer_contact(&keypair());
    assert_eq!(hex::encode(contact.serialize_to_vec()), SIGNED_PEER_CONTACT);

    let decoded =
        SignedPeerContact::deserialize_from_vec(&hex::decode(SIGNED_PEER_CONTACT).unwrap())
            .unwrap();
    assert!(decoded.verify());
    assert_eq!(decoded, contact);
    assert_eq!(decoded.peer_id().to_string(), PEER_ID);
}

#[test]
fn discovery_message_encodings_are_stable() {
    let keypair = keypair();

    assert_discovery_message(
        DiscoveryMessage::Handshake {
            observed_address: "/ip4/198.51.100.23/tcp/51234/ws".parse().unwrap(),
            challenge_nonce: ChallengeNonce::from([0x17u8; 32]),
            genesis_hash: Blake2bHash::from([0x5au8; 32]),
            limit: 64,
            services: Services::FULL_BLOCKS | Services::VALIDATOR,
        },
        HANDSHAKE,
    );
    assert_discovery_message(
        DiscoveryMessage::HandshakeAck {
            peer_contact: signed_peer_contact(&keypair),
            response_signature: keypair.tagged_sign(&ChallengeNonce::from([0x17u8; 32])),
            update_interval: Some(60),
            peer_contacts: vec![signed_peer_contact(&keypair)],
        },
        HANDSHAKE_ACK,
    );
    assert_discovery_message(
        DiscoveryMessage::PeerAddresses {
            peer_contacts: vec![signed_peer_contact(&keypair)],
        },
        PEER_ADDRESSES,
    );
}
