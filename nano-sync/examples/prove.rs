
use std::fs::{DirBuilder, File};
use std::path::Path;
use std::time::Instant;

use ark_crypto_primitives::SNARK;
use ark_ec::{ProjectiveCurve};



use ark_mnt6_753::{Fr as MNT6Fr, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};

use ark_serialize::{CanonicalSerialize};
use ark_std::ops::MulAssign;
use ark_std::{test_rng, UniformRand};
use rand::prelude::SliceRandom;
use rand::RngCore;





use nimiq_nano_sync::constants::{
    EPOCH_LENGTH, MIN_SIGNERS, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS,
};
use nimiq_nano_sync::primitives::{
    merkle_tree_prove, pk_tree_construct, serialize_g1_mnt6, serialize_g2_mnt6, state_commitment,
    vk_commitment, MacroBlock,
};

use nimiq_nano_sync::NanoZKP;

fn main() {
    println!("====== Generating random inputs ======");
    let rng = &mut test_rng();

    // Create key pairs for all the initial validators.
    let mut initial_sks = vec![];
    let mut initial_pks = vec![];

    for _ in 0..VALIDATOR_SLOTS {
        let sk = MNT6Fr::rand(rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        initial_sks.push(sk);
        initial_pks.push(pk);
    }

    // Create key pairs for all the final validators.
    let mut final_sks = vec![];
    let mut final_pks = vec![];

    for _ in 0..VALIDATOR_SLOTS {
        let sk = MNT6Fr::rand(rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        final_sks.push(sk);
        final_pks.push(pk);
    }

    // Calculate final public key tree root.
    let final_pk_tree_root = pk_tree_construct(final_pks.clone());

    // Create a random signer bitmap.
    let mut signer_bitmap = vec![true; MIN_SIGNERS];

    signer_bitmap.append(&mut vec![false; VALIDATOR_SLOTS - MIN_SIGNERS]);

    signer_bitmap.shuffle(rng);

    // Create a macro block
    let block_number = 0;

    let round_number = u32::rand(rng);

    let mut header_hash = [0u8; 32];
    rng.fill_bytes(&mut header_hash);

    let mut block = MacroBlock::without_signatures(block_number, round_number, header_hash);

    for i in 0..VALIDATOR_SLOTS {
        if signer_bitmap[i] {
            block.sign(initial_sks[i].clone(), i, final_pk_tree_root.clone());
        }
    }

    println!("====== Proof generation for Nano Sync initiated ======");
    let start = Instant::now();

    let proof = NanoZKP.prove(initial_pks, final_pks, block, None).unwrap();

    if !Path::new("proofs/").is_dir() {
        DirBuilder::new().create("proofs/").unwrap();
    }

    let mut file = File::create("proofs/proof_epoch_0.bin").unwrap();

    proof.serialize_unchecked(&mut file).unwrap();

    file.sync_all().unwrap();

    println!("====== Proof generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
