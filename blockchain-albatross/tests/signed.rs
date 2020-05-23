extern crate beserial;
extern crate nimiq_block_albatross as block_albatross;
extern crate nimiq_bls as bls;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;

use beserial::Deserialize;
use block_albatross::signed::Message;
use block_albatross::{
    PbftCommitMessage, PbftPrepareMessage, SignedPbftCommitMessage, SignedViewChange, ViewChange,
    ViewChangeProofBuilder,
};
use bls::lazy::LazyPublicKey;
use bls::KeyPair;
use hash::{Blake2bHash, Hash};
use keys::Address;
use nimiq_vrf::VrfSeed;
use primitives::policy;
use primitives::slot::{ValidatorSlotBand, ValidatorSlots};

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &'static str = "49ea68eb6b8afdf4ca4d4c0a0b295c76ca85225293693bc30e755476492b707f";

#[test]
fn test_view_change_single_signature() {
    // parse key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();

    // create a view change
    let view_change = ViewChange {
        block_number: 1234,
        new_view_number: 42,
        prev_seed: VrfSeed::default(),
    };

    // sign view change and build view change proof
    let signed_message = SignedViewChange::from_message(view_change.clone(), &key_pair.secret, 0);
    let mut proof_builder = ViewChangeProofBuilder::new();
    proof_builder.add_signature(&key_pair.public, policy::SLOTS, &signed_message);
    let view_change_proof = proof_builder.build();

    // verify view change proof
    let validators = ValidatorSlots::new(vec![ValidatorSlotBand::new(
        LazyPublicKey::from(key_pair.public),
        Address::default(),
        policy::SLOTS,
    )]);
    view_change_proof
        .verify(&view_change, &validators, policy::TWO_THIRD_SLOTS)
        .unwrap();
}

#[test]
/// Tests if an attacker can use the prepare signature to fake a commit signature. If we would
/// only sign the `block_hash`, this would work, but `SignedMessage` adds a prefix byte.
fn test_replay() {
    // load key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();
    // create dummy hash and prepare message
    let block_hash = "foobar".hash::<Blake2bHash>();
    let prepare = PbftPrepareMessage {
        block_hash: block_hash.clone(),
    };

    // sign prepare
    let prepare_signature = prepare.sign(&key_pair.secret);

    // fake commit
    let commit = PbftCommitMessage { block_hash };
    let signed_commit = SignedPbftCommitMessage {
        message: commit,
        signer_idx: 0,
        signature: prepare_signature,
    };

    // verify commit - this should fail
    assert!(!signed_commit.verify(&key_pair.public));
}
