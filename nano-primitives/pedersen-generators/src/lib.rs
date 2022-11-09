use std::cmp;
use std::fs::File;
use std::path::PathBuf;

use beserial::Serialize;
use beserial::SerializeWithLength;
pub use beserial::SerializingError;
use nimiq_bls::pedersen::pedersen_generator_powers;
use nimiq_primitives::policy;

/// This is the depth of the PKTree circuit.
const PK_TREE_DEPTH: usize = 5;

/// This is the number of leaves in the PKTree circuit.
const PK_TREE_BREADTH: usize = 2_usize.pow(PK_TREE_DEPTH as u32);

const G2_MNT6_SIZE: usize = 285;

pub fn generate_pedersen_generators(dest_path: PathBuf) -> Result<(), SerializingError> {
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

    // Compute generator powers.
    let generator_powers = pedersen_generator_powers(generators_needed);

    // Write to file.
    let mut file = File::create(dest_path)?;

    Serialize::serialize(&(generator_powers.len() as u16), &mut file)?;
    for powers in generator_powers {
        SerializeWithLength::serialize::<u16, _>(&powers, &mut file)?;
    }

    Ok(())
}
