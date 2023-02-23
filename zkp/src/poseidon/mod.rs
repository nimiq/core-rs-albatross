use ark_crypto_primitives::sponge::poseidon::{
    find_poseidon_ark_and_mds, PoseidonConfig, PoseidonDefaultConfigEntry,
};
use ark_ff::PrimeField;

pub trait DefaultPoseidonParameters {
    const PARAMS_T3: PoseidonDefaultConfigEntry;
    const PARAMS_T9: PoseidonDefaultConfigEntry;
}

pub fn create_parameters<F: PrimeField>(param: PoseidonDefaultConfigEntry) -> PoseidonConfig<F> {
    let (ark, mds) = find_poseidon_ark_and_mds::<F>(
        F::MODULUS_BIT_SIZE as u64,
        param.rate,
        param.full_rounds as u64,
        param.partial_rounds as u64,
        param.skip_matrices as u64,
    );

    PoseidonConfig {
        full_rounds: param.full_rounds,
        partial_rounds: param.partial_rounds,
        alpha: param.alpha as u64,
        ark,
        mds,
        rate: param.rate,
        capacity: 1,
    }
}

mod mnt4;
mod mnt6;
