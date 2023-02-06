use ark_ff::{FpParameters, PrimeField};
use ark_sponge::poseidon::{
    find_poseidon_ark_and_mds, PoseidonDefaultParametersEntry, PoseidonParameters,
};

pub trait DefaultPoseidonParameters {
    const PARAMS_T3: PoseidonDefaultParametersEntry;
    const PARAMS_T9: PoseidonDefaultParametersEntry;
}

pub fn create_parameters<F: PrimeField>(
    param: PoseidonDefaultParametersEntry,
) -> PoseidonParameters<F> {
    let (ark, mds) = find_poseidon_ark_and_mds::<F>(
        <F::Params as FpParameters>::MODULUS_BITS as u64,
        param.rate,
        param.full_rounds as u64,
        param.partial_rounds as u64,
        param.skip_matrices as u64,
    );

    PoseidonParameters {
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
