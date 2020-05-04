#![allow(dead_code)]

use std::ops::Add;

use algebra::mnt6_753::{Fr, G2Projective};
use algebra::test_rng;
use algebra_core::fields::Field;
use algebra_core::ProjectiveCurve;
use r1cs_core::ConstraintSynthesizer;
use r1cs_std::test_constraint_system::TestConstraintSystem;
use rand::RngCore;

use nano_sync::circuits::mnt4::PKTree3Circuit;
use nano_sync::constants::{
    sum_generator_g2_mnt6, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS,
};
use nano_sync::primitives::{merkle_tree_construct, merkle_tree_prove};
use nano_sync::utils::{bytes_to_bits, serialize_g2_mnt6};

// When running tests you are advised to set VALIDATOR_SLOTS in constants.rs to a more manageable
// number. We advise setting VALIDATOR_SLOTS = 64. Even then, you can only run one test at a time or
// you'll run out of RAM.

#[test]
fn everything_works() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
        pks_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        pks_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        position,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    println!("Number of constraints: {}", test_cs.num_constraints());

    if !test_cs.is_satisfied() {
        println!("Unsatisfied @ {}", test_cs.which_is_unsatisfied().unwrap());
        assert!(false);
    }
}

#[test]
fn wrong_pks() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Create fake public keys.
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);
    let fake_pks = [pk; VALIDATOR_SLOTS / PK_TREE_BREADTH];

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        fake_pks.to_vec(),
        pks_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        pks_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        position,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_merkle_proof() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let _pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        pks_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        position,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_agg_pk() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
        pks_nodes.clone(),
        agg_pk_chunks[1],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        pks_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        position,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_commitment() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let _pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
        pks_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        position,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_bitmap() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
        pks_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        pks_commitment.clone(),
        [1u8; VALIDATOR_SLOTS].to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        position,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}

#[test]
fn wrong_position() {
    // Create random public key.
    let rng = &mut test_rng();
    let mut bytes = [0u8; 96];
    rng.fill_bytes(&mut bytes[2..]);
    let x = Fr::from_random_bytes(&bytes).unwrap();
    let pk = G2Projective::prime_subgroup_generator().mul(x);

    // Create random bitmap.
    let mut bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bitmap);
    let bitmap_bits = bytes_to_bits(&bitmap);

    // Create inputs.
    let _position = 0;

    let path = vec![false; PK_TREE_DEPTH];

    let pks = [pk; VALIDATOR_SLOTS];

    // Create pks commitment and Merkle proof for the given position in the tree.
    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut bits = Vec::new();
        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            bits.extend(bytes_to_bits(&serialize_g2_mnt6(
                pks[i * PK_TREE_BREADTH + j],
            )));
        }
        pks_bits.push(bits);
    }

    let pks_commitment = merkle_tree_construct(pks_bits.clone());

    let pks_nodes = merkle_tree_prove(pks_bits, path.clone());

    // Create agg pk commitment and Merkle proof for the given position in the tree.
    let mut agg_pk_chunks = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        let mut key = sum_generator_g2_mnt6();

        for j in 0..VALIDATOR_SLOTS / PK_TREE_BREADTH {
            if bitmap_bits[i * PK_TREE_BREADTH + j] {
                key = key.add(&pks[i * PK_TREE_BREADTH + j]);
            }
        }

        agg_pk_chunks.push(key);
    }

    let mut agg_pk_chunks_bits = Vec::new();

    for i in 0..agg_pk_chunks.len() {
        let bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk_chunks[i]));
        agg_pk_chunks_bits.push(bits);
    }

    let agg_pk_commitment = merkle_tree_construct(agg_pk_chunks_bits.clone());

    let agg_pk_nodes = merkle_tree_prove(agg_pk_chunks_bits, path.clone());

    // Test constraint system.
    let mut test_cs = TestConstraintSystem::new();

    let c = PKTree3Circuit::new(
        pks[0..VALIDATOR_SLOTS / PK_TREE_BREADTH].to_vec(),
        pks_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        agg_pk_chunks[0],
        agg_pk_nodes.clone(),
        pks_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        bitmap.to_vec(),
        agg_pk_commitment.clone(),
        1,
    );

    c.generate_constraints(&mut test_cs).unwrap();

    assert!(!test_cs.is_satisfied())
}
