use ark_ec::Group;
use ark_mnt6_753::{Fr as MNT6Fr, G2Projective as G2MNT6};
use ark_std::{ops::MulAssign, UniformRand};
use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::{pk_tree_construct, state_commitment, MacroBlock};
use rand::{rngs::SmallRng, seq::SliceRandom, RngCore, SeedableRng};

/// Transforms a u8 into a vector of little endian bits.
pub fn byte_to_le_bits(mut byte: u8) -> Vec<bool> {
    let mut bits = vec![];

    for _ in 0..8 {
        bits.push(byte % 2 != 0);
        byte >>= 1;
    }

    bits
}

/// Create a macro block, validator keys and other information needed to produce a zkp SNARK
/// proof. It is used in the examples. It takes as input an index that represents the epoch that we are in.
/// Note that the RNG and seed aren't secure enough, so this function should only be used for test purposes.
pub fn create_test_blocks(
    index: u64,
) -> (
    Vec<G2MNT6>,
    [u8; 32],
    [u8; 95],
    Vec<G2MNT6>,
    MacroBlock,
    Option<[u8; 95]>,
) {
    // The random seed. It was generated using random.org.
    let seed = 12370426996209291122;

    // Create RNG.
    let mut rng = SmallRng::seed_from_u64(seed + index);

    // Create key pairs for the previous validators.
    let mut prev_sks = vec![];
    let mut prev_pks = vec![];

    for _ in 0..Policy::SLOTS {
        let sk = MNT6Fr::rand(&mut rng);
        let mut pk = G2MNT6::generator();
        pk.mul_assign(sk);
        prev_sks.push(sk);
        prev_pks.push(pk);
    }

    // Create the previous header hash.
    let mut prev_header_hash = [0u8; 32];
    rng.fill_bytes(&mut prev_header_hash);

    // Create a random signer bitmap.
    let mut signer_bitmap = vec![true; Policy::TWO_F_PLUS_ONE as usize];

    signer_bitmap.append(&mut vec![
        false;
        (Policy::SLOTS - Policy::TWO_F_PLUS_ONE) as usize
    ]);

    signer_bitmap.shuffle(&mut rng);

    // Restart the RNG with the next index.
    let mut rng = SmallRng::seed_from_u64(seed + index + 1);

    // Create key pairs for the final validators.
    let mut final_sks = vec![];
    let mut final_pks = vec![];

    for _ in 0..Policy::SLOTS {
        let sk = MNT6Fr::rand(&mut rng);
        let mut pk = G2MNT6::generator();
        pk.mul_assign(sk);
        final_sks.push(sk);
        final_pks.push(pk);
    }

    // Create the final header hash.
    let mut final_header_hash = [0u8; 32];
    rng.fill_bytes(&mut final_header_hash);

    // There is no more randomness being generated from this point on.

    // Calculate final public key tree root.
    let final_pk_tree_root = pk_tree_construct(final_pks.clone());

    // Create the macro block.
    let mut block = MacroBlock::without_signatures(
        Policy::blocks_per_epoch() * (index as u32 + 1),
        0,
        final_header_hash,
    );

    for i in 0..Policy::SLOTS as usize {
        if signer_bitmap[i] {
            block.sign(&prev_sks[i], i, &final_pk_tree_root);
        }
    }

    let prev_pk_tree_root = pk_tree_construct(prev_pks.clone());

    // If this is the first index (genesis), also return the genesis state commitment.
    let genesis_state_commitment = if index == 0 {
        Some(state_commitment(0, &prev_header_hash, &prev_pk_tree_root))
    } else {
        None
    };

    // Return the data.
    (
        prev_pks,
        prev_header_hash,
        prev_pk_tree_root,
        final_pks,
        block,
        genesis_state_commitment,
    )
}
