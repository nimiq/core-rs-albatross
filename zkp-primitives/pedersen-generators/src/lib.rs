use std::marker::PhantomData;

use ark_crypto_primitives::crh::pedersen::Window;
use ark_ff::PrimeField;
use ark_mnt6_753::G1Projective;
use generators::POINT_CAPACITY;
pub use generators::{pedersen_generator_powers, PedersenParameters};
use nimiq_primitives::policy::Policy;

mod generators;
mod rand_gen;

/// This is the depth of the PKTree circuit.
const PK_TREE_DEPTH: usize = 5;

/// This is the number of leaves in the PKTree circuit.
const PK_TREE_BREADTH: usize = 2_usize.pow(PK_TREE_DEPTH as u32);

const G2_MNT6_SIZE: usize = 285;

#[derive(Clone)]
pub struct DefaultWindow;
impl Window for DefaultWindow {
    const NUM_WINDOWS: usize = num_windows() - 1;
    const WINDOW_SIZE: usize = POINT_CAPACITY;
}

#[derive(Clone)]
pub struct GenericWindow<const NUM_WINDOWS: usize, F: PrimeField> {
    _f: PhantomData<F>,
}
impl<const NUM_WINDOWS: usize, F: PrimeField> Window for GenericWindow<NUM_WINDOWS, F> {
    const NUM_WINDOWS: usize = NUM_WINDOWS;
    const WINDOW_SIZE: usize = F::MODULUS_BIT_SIZE as usize - 1;
}

const fn num_windows() -> usize {
    let num_pks = Policy::SLOTS as usize;
    let num_bits = num_pks * G2_MNT6_SIZE * 8;
    let num_bits_per_leaf = num_bits / PK_TREE_BREADTH;

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = POINT_CAPACITY;

    let generators_needed_a = 4; // At least this much is required for the non-leaf nodes.
    let generators_needed_b = (num_bits_per_leaf + capacity - 1) / capacity + 1;

    // Choose maximum.
    if generators_needed_a > generators_needed_b {
        generators_needed_a
    } else {
        generators_needed_b
    }
}

pub fn default() -> PedersenParameters<G1Projective> {
    pedersen_generator_powers::<DefaultWindow>()
}
