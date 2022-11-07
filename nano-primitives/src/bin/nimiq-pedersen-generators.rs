use std::cmp;

use ark_serialize::CanonicalSerialize;
use nimiq_bls::pedersen::pedersen_generators;
use nimiq_nano_primitives::PK_TREE_BREADTH;
use nimiq_primitives::policy;

const G2_MNT6_SIZE: usize = 285;

fn main() {
    let num_pks = policy::SLOTS as usize;

    let num_bits = num_pks * G2_MNT6_SIZE * 8;

    let num_bits_per_leaf = num_bits / PK_TREE_BREADTH;

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = 752;

    let mut generators_needed = 4; // At least this much is required for the non-leaf nodes.

    generators_needed = cmp::max(
        generators_needed,
        (num_bits_per_leaf + capacity - 1) / capacity + 1,
    );

    let generators = pedersen_generators(generators_needed);

    let mut hex_generators = Vec::with_capacity(generators.len());
    for generator in generators {
        let mut buf = Vec::with_capacity(CanonicalSerialize::uncompressed_size(&generator));
        CanonicalSerialize::serialize_unchecked(&generator, &mut buf).unwrap();
        hex_generators.push(hex::encode(buf));
    }

    println!("{:?}", hex_generators);
}
