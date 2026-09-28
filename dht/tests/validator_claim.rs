use std::sync::Arc;

use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_dht::Verifier;
use nimiq_keys::{Address, KeyPair};
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::{
    validator_claim::ValidatorClaimSigner, validator_record::ValidatorRecord,
};
use nimiq_network_libp2p::{
    dht::{DhtVerifierError, Verifier as DhtVerifier},
    discovery::{
        InvalidReason, SignedValidatorClaim, UnverifiableReason, ValidatorClaimVerifier,
        ValidatorVerification,
    },
    libp2p::kad::{Record, RecordKey},
    PeerId,
};
use nimiq_primitives::networks::NetworkId;
use nimiq_serde::Serialize;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::{signing_key, validator_address, validator_key};
use nimiq_utils::{
    tagged_signing::{TaggedSignature, TaggedSigned},
    time::OffsetTime,
};
use parking_lot::RwLock;

/// A verifier backed by a full blockchain at the unit-test genesis, whose staking contract holds a
/// single validator: the one at [`validator_address`], with [`signing_key`] as its signing key.
fn full_verifier() -> Verifier {
    let blockchain = Blockchain::new(
        MdbxDatabase::new_volatile(Default::default()).unwrap(),
        BlockchainConfig::default(),
        NetworkId::UnitAlbatross,
        Arc::new(OffsetTime::new()),
    )
    .unwrap();
    Verifier::new(BlockchainProxy::from(Arc::new(RwLock::new(blockchain))))
}

/// A verifier backed by a light blockchain at the unit-test genesis.
fn light_verifier() -> Verifier {
    let blockchain = LightBlockchain::new(NetworkId::UnitAlbatross);
    Verifier::new(BlockchainProxy::from(Arc::new(RwLock::new(blockchain))))
}

/// Signs a claim binding a random peer ID to `validator_address` with `signing_key`, the same way a
/// validator does for its own peer contact. The verifier does not look at the timestamp.
fn sign_claim(validator_address: Address, signing_key: KeyPair) -> SignedValidatorClaim {
    ValidatorClaimSigner::new(validator_address, signing_key).sign(PeerId::random(), 1_700_000_000)
}

#[test]
fn claim_signed_with_the_on_chain_signing_key_is_verified() {
    let signed_claim = sign_claim(validator_address(), signing_key());

    assert_eq!(
        full_verifier().verify_validator_claim(&signed_claim),
        ValidatorVerification::Verified,
    );
}

#[test]
fn claim_signed_with_another_key_has_an_invalid_signature() {
    // The validator key is the validator's own key, not its signing key, so it must not be
    // accepted in place of the latter.
    let signed_claim = sign_claim(validator_address(), validator_key());

    assert_eq!(
        full_verifier().verify_validator_claim(&signed_claim),
        ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
    );
}

#[test]
fn claim_for_an_address_that_is_not_a_validator_is_invalid() {
    // Signed with the genesis validator's signing key, so that only the address is at fault.
    let signed_claim = sign_claim(Address::from([0x42; 20]), signing_key());

    assert_eq!(
        full_verifier().verify_validator_claim(&signed_claim),
        ValidatorVerification::Invalid(InvalidReason::UnknownValidator),
    );
}

#[test]
fn claim_is_unverifiable_on_a_light_client() {
    // The same claim is verified on a full node.
    let signed_claim = sign_claim(validator_address(), signing_key());

    assert_eq!(
        light_verifier().verify_validator_claim(&signed_claim),
        ValidatorVerification::Unverifiable(UnverifiableReason::LightClient),
    );
}

/// Replaces the signature of `signed_claim` with one of the wrong size.
fn with_malformed_signature(mut signed_claim: SignedValidatorClaim) -> SignedValidatorClaim {
    signed_claim.signature = TaggedSignature::from_bytes(vec![0; 10]);
    signed_claim
}

#[test]
fn claim_with_a_signature_of_the_wrong_size_is_invalid_without_a_lookup() {
    // A light client cannot look up any signing key, so answering anything but `LightClient` shows
    // that the claim was rejected before the lookup.
    let signed_claim = with_malformed_signature(sign_claim(validator_address(), signing_key()));

    assert_eq!(
        light_verifier().verify_validator_claim(&signed_claim),
        ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
    );
    assert_eq!(
        full_verifier().verify_validator_claim(&signed_claim),
        ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
    );
}

#[test]
fn record_with_a_signature_of_the_wrong_size_is_rejected_without_a_lookup() {
    let peer_id = PeerId::random();
    let record = ValidatorRecord::new(peer_id, validator_address(), 1);
    let signed_record = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::new(
        record,
        TaggedSignature::from_bytes(vec![0; 10]),
    );
    let mut dht_record = Record::new(
        RecordKey::new(&validator_address().serialize_to_vec()),
        signed_record.serialize_to_vec(),
    );
    dht_record.publisher = Some(peer_id);

    // A light client answers `UnknownTag` once it gets to the lookup.
    assert!(matches!(
        DhtVerifier::verify(&light_verifier(), &dht_record),
        Err(DhtVerifierError::InvalidSignature)
    ));
    assert!(matches!(
        DhtVerifier::verify(&full_verifier(), &dht_record),
        Err(DhtVerifierError::InvalidSignature)
    ));
}
