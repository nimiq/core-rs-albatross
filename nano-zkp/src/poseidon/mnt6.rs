use ark_sponge::poseidon::{PoseidonParameters, PoseidonDefaultParametersEntry};
use ark_mnt6_753::Fq;
use super::{DefaultPoseidonParameters, create_parameters};

// Parameters have been generated using the reference implementation here: https://extgit.iaik.tugraz.at/krypto/hadeshash/-/blob/master/code/generate_parameters_grain.sage
// Partial rounds have been rounded up to the nearest multiple of `t`.
impl DefaultPoseidonParameters for Fq {
    const PARAMS_T3: PoseidonDefaultParametersEntry = PoseidonDefaultParametersEntry::new(2, 11, 8, 39, 0);
    const PARAMS_T9: PoseidonDefaultParametersEntry = PoseidonDefaultParametersEntry::new(8, 11, 8, 40, 0);
}

pub fn poseidon_mnt6_t3_parameters() -> PoseidonParameters<Fq> {
    create_parameters(Fq::PARAMS_T3)
}

pub fn poseidon_mnt6_t9_parameters() -> PoseidonParameters<Fq> {
    create_parameters(Fq::PARAMS_T9)
}
