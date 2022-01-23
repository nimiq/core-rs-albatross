use std::sync::Arc;

use beserial::Deserialize;
use nimiq_block::{
    MacroBlock, MultiSignature, SignedViewChange, TendermintIdentifier, TendermintProof,
    TendermintStep, TendermintVote, ViewChange, ViewChangeProof,
};
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_bls::{lazy::LazyPublicKey, AggregateSignature, KeyPair};
use nimiq_collections::bitset::BitSet;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;

use nimiq_keys::{Address, PublicKey};

use nimiq_primitives::policy;
use nimiq_primitives::slots::{Validator, Validators};
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::VrfEntropy;

/// Secret keys of validator. Tests run with `genesis/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str = "689bf3a52a07af0c7a0901a17e5d285496bb4b60626f4ebd9365ab05a093998edf28b3bf2cf25bdeb5458a14c2ade928ada0215de265ace57616c5ce4f76991d2f350fb8df796dbb2da4c492817d27a1578e5006b43be2f05b938fb5134f0000";

#[test]
fn test_view_change_single_signature() {
    // parse key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();

    // create a view change
    let view_change = ViewChange {
        block_number: 1234,
        new_view_number: 42,
        vrf_entropy: VrfEntropy::default(),
    };

    // sign view change and build view change proof
    let signature = AggregateSignature::from_signatures(&[SignedViewChange::from_message(
        view_change.clone(),
        &key_pair.secret_key,
        0,
    )
    .signature
    .multiply(policy::SLOTS)]);
    // ViewChangeProof is just a MultiSignature, but for ease of getting there an individual Signature is created first.
    let mut signers = BitSet::new();
    for i in 0..policy::SLOTS {
        signers.insert(i as usize);
    }

    let view_change_proof: ViewChangeProof = ViewChangeProof {
        sig: MultiSignature::new(signature, signers),
    };

    // verify view change proof
    let validators = Validators::new(vec![Validator::new(
        Address::default(),
        LazyPublicKey::from(key_pair.public_key),
        PublicKey::from([0u8; 32]),
        (0, policy::SLOTS),
    )]);

    assert!(view_change_proof.verify(&view_change, &validators));
}

#[test]
/// Tests if an attacker can use the prepare signature to fake a commit signature. If we would
/// only sign the `block_hash`, this would work, but `SignedMessage` adds a prefix byte.
fn test_replay() {
    let time = Arc::new(OffsetTime::new());
    // Create a blockchain to have access to the validator slots.
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap());

    // load key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();

    // create dummy block
    let mut block = MacroBlock::default();
    block.header.block_number = 1;

    // create hash and prepare message
    let block_hash = block.nano_zkp_hash();

    let validators = blockchain.current_validators().unwrap();

    // create a TendermintVote for the PreVote round
    let vote = TendermintVote {
        proposal_hash: Some(block_hash.clone()),
        id: TendermintIdentifier {
            block_number: 1u32,
            step: TendermintStep::PreVote,
            round_number: 0,
        },
    };

    let signature = AggregateSignature::from_signatures(&[key_pair
        .secret_key
        .sign(&vote)
        .multiply(policy::SLOTS)]);

    // create and populate signers BitSet.
    let mut signers = BitSet::new();
    for i in 0..policy::SLOTS {
        signers.insert(i as usize);
    }

    // create the TendermintProof
    block.justification = Some(TendermintProof {
        round: 0,
        sig: MultiSignature::new(signature, signers),
    });

    // verify commit - this should fail
    assert!(!TendermintProof::verify(&block, &validators));

    // create the same thing again but for the PreCommit round
    let vote = TendermintVote {
        proposal_hash: Some(block_hash),
        id: TendermintIdentifier {
            block_number: 1u32,
            step: TendermintStep::PreCommit,
            round_number: 0,
        },
    };

    let signature = AggregateSignature::from_signatures(&[key_pair
        .secret_key
        .sign(&vote)
        .multiply(policy::SLOTS)]);

    // create and populate signers BitSet.
    let mut signers = BitSet::new();
    for i in 0..policy::SLOTS {
        signers.insert(i as usize);
    }

    // create the TendermintProof
    block.justification = Some(TendermintProof {
        round: 0,
        sig: MultiSignature::new(signature, signers),
    });

    // verify commit - this should not fail as this time it is the correct round
    assert!(TendermintProof::verify(&block, &validators));
}
