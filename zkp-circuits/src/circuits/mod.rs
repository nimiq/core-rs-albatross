//! This module contains the zk-SNARK circuits that are used in the light macro sync. Each circuit produces
//! a proof and they can be "chained" together by using one's output as another's input.

pub mod mnt4;
pub mod mnt6;
pub mod vk_commitments;

use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::{Field, PrimeField};

pub trait CircuitInput {
    const NUM_INPUTS: usize;
}

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;
pub const fn num_inputs<P: Pairing>(num_bytes: &[usize]) -> usize {
    let capacity = BasePrimeField::<P>::MODULUS_BIT_SIZE as usize - 1;

    let mut num_inputs = 0;
    let mut i = 0;
    loop {
        // ceiling div: (self + rhs - 1) / rhs
        num_inputs += (num_bytes[i] * 8 + capacity - 1) / capacity;
        i += 1;
        if i >= num_bytes.len() {
            break;
        }
    }
    num_inputs
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::MNT4_753;
    use ark_mnt6_753::MNT6_753;

    use super::*;

    #[test]
    fn test_num_inputs() {
        assert_eq!(num_inputs::<MNT4_753>(&[32, 95]), 3);
        assert_eq!(num_inputs::<MNT6_753>(&[32, 95]), 3);

        assert_eq!(num_inputs::<MNT4_753>(&[95 * 2]), 3);
        assert_eq!(num_inputs::<MNT6_753>(&[95 * 2]), 3);

        assert_eq!(num_inputs::<MNT4_753>(&[32, 32, 95]), 4);
        assert_eq!(num_inputs::<MNT6_753>(&[32, 32, 95]), 4);
    }
}
