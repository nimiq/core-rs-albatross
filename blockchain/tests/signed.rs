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
use nimiq_vrf::vrf::VrfEntropy;

// /// Still in for future reference, in case this key is needed again
// const SECRET_KEY: &str = "8e44b45f308dae1e2d4390a0f96cea993960d4178550c62aeaba88e9e168d165\
// a8dadd6e1c553412d5c0f191e83ffc5a4b71bf45df6b5a125ec2c4a9a40643597cb6b5c3b588d55a363f1b56ac839eee4a6\
// ff848180500f2fc29d1c0595f0000";
///  works with NetworkId::UnitAlbatross
const SECRET_KEY: &str = "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

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
