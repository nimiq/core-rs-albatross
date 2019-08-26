extern crate nimiq_block_albatross as block_albatross;
extern crate nimiq_bls as bls;
extern crate nimiq_primitives as primitives;
extern crate beserial;
extern crate nimiq_hash as hash;

use std::iter::FromIterator;

use block_albatross::{ViewChangeProof, ViewChange, ViewChangeProofBuilder, SignedViewChange, PbftPrepareMessage, SignedPbftCommitMessage, PbftCommitMessage};
use bls::bls12_381::KeyPair;
use primitives::policy;
use block_albatross::signed::Message;
use nimiq_collections::grouped_list::{GroupedList, Group};
use bls::bls12_381::lazy::LazyPublicKey;
use beserial::Deserialize;
use hash::{Blake2bHash, Hash};


/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &'static str = "49ea68eb6b8afdf4ca4d4c0a0b295c76ca85225293693bc30e755476492b707f";


#[test]
fn test_view_change_single_signature() {
    // parse key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();

    // create a view change
    let view_change = ViewChange {
        block_number: 1234,
        new_view_number: 42
    };

    // sign view change and build view change proof
    let signed_message = SignedViewChange::from_message(view_change.clone(), &key_pair.secret, 0);
    let mut proof_builder = ViewChangeProofBuilder::new();
    proof_builder.add_signature(&key_pair.public, policy::SLOTS, &signed_message);
    let view_change_proof = proof_builder.build();

    // verify view change proof
    let validators = GroupedList(vec![Group(policy::SLOTS, LazyPublicKey::from(key_pair.public))]);
    view_change_proof.verify(&view_change, &validators, policy::TWO_THIRD_SLOTS);
}

#[test]
/// Tests if an attacker can use the prepare signature to fake a commit signature. If we would
/// only sign the `block_hash`, this would work, but `SignedMessage` adds a prefix byte.
fn test_replay() {
    // load key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();
    // create dummy hash and prepare message
    let block_hash = "foobar".hash::<Blake2bHash>();
    let prepare = PbftPrepareMessage { block_hash: block_hash.clone() };

    // sign prepare
    let prepare_signature = prepare.sign(&key_pair.secret);

    // fake commit
    let commit = PbftCommitMessage { block_hash };
    let signed_commit = SignedPbftCommitMessage { message: commit, signer_idx: 0, signature: prepare_signature };

    // verify commit - this should fail
    assert!(!signed_commit.verify(&key_pair.public));
}
