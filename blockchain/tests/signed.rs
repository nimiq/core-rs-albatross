use std::sync::Arc;

use nimiq_block::{
    MacroBlock, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo, SkipBlockProof, TendermintProof,
};
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::{lazy::LazyPublicKey, AggregateSignature, KeyPair};
use nimiq_collections::bitset::BitSet;
use nimiq_database::volatile::VolatileDatabase;
use nimiq_keys::{Address, EdDSAPublicKey};
use nimiq_primitives::{
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{Validator, Validators},
    TendermintIdentifier, TendermintStep, TendermintVote,
};
use nimiq_serde::Deserialize;
use nimiq_test_log::test;
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::VrfEntropy;

///  works with NetworkId::UnitAlbatross
const SECRET_KEY: &str = "99237809f3b37bd0878854d2b5b66e4cc00ba1a1d64377c374f2b6d1bf3dec7835bfae3e7ab89b6d331b3ef7d1e9a06a7f6967bf00edf9e0bcfe34b58bd1260e96406e09156e4c190ff8f69a9ce1183b4289383e6d798fd5104a3800fabd00";

#[test]
fn test_skip_block_single_signature() {
    // parse key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();

    // create skip block data
    let skip_block_info = SkipBlockInfo {
        block_number: 1234,
        vrf_entropy: VrfEntropy::default(),
    };

    // sign skip block and build skip block proof
    let signature = AggregateSignature::from_signatures(&[SignedSkipBlockInfo::from_message(
        skip_block_info.clone(),
        &key_pair.secret_key,
        0,
    )
    .signature
    .multiply(Policy::SLOTS)]);
    // SkipBlockProof is just a MultiSignature, but for ease of getting there an individual Signature is created first.
    let mut signers = BitSet::new();
    for i in 0..Policy::SLOTS {
        signers.insert(i as usize);
    }

    let skip_block_proof: SkipBlockProof = SkipBlockProof {
        sig: MultiSignature::new(signature, signers),
    };

    // verify skip block proof
    let validators = Validators::new(vec![Validator::new(
        Address::default(),
        LazyPublicKey::from(key_pair.public_key),
        EdDSAPublicKey::from([0u8; 32]),
        0..Policy::SLOTS,
    )]);

    assert!(skip_block_proof.verify(&skip_block_info, &validators));
}

#[test]
/// Tests if an attacker can use the prepare signature to fake a commit signature. If we would
/// only sign the `block_hash`, this would work, but `SignedMessage` adds a prefix byte.
fn test_replay() {
    let time = Arc::new(OffsetTime::new());
    // Create a blockchain to have access to the validator slots.
    let env = VolatileDatabase::new(20).unwrap();
    let blockchain = Arc::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    );

    // load key pair
    let key_pair = KeyPair::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap();

    // create dummy block
    let mut block = MacroBlock::default();
    block.header.network = NetworkId::UnitAlbatross;
    block.header.block_number = 1;

    // create hash and prepare message
    let block_hash = block.hash_blake2s();

    let validators = blockchain.current_validators().unwrap();

    // create a TendermintVote for the PreVote round
    let vote = TendermintVote {
        proposal_hash: Some(block_hash.clone()),
        id: TendermintIdentifier {
            network: NetworkId::UnitAlbatross,
            block_number: 1u32,
            step: TendermintStep::PreVote,
            round_number: 0,
        },
    };

    let signature = AggregateSignature::from_signatures(&[key_pair
        .secret_key
        .sign(&vote)
        .multiply(Policy::SLOTS)]);

    // create and populate signers BitSet.
    let mut signers = BitSet::new();
    for i in 0..Policy::SLOTS {
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
            network: NetworkId::UnitAlbatross,
            block_number: 1u32,
            step: TendermintStep::PreCommit,
            round_number: 0,
        },
    };

    let signature = AggregateSignature::from_signatures(&[key_pair
        .secret_key
        .sign(&vote)
        .multiply(Policy::SLOTS)]);

    // create and populate signers BitSet.
    let mut signers = BitSet::new();
    for i in 0..Policy::SLOTS {
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
