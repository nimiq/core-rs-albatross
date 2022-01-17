use super::{create_parameters, DefaultPoseidonParameters};
use ark_mnt4_753::Fq;
use ark_sponge::poseidon::{PoseidonDefaultParametersEntry, PoseidonParameters};

// Parameters have been generated using the reference implementation here: https://extgit.iaik.tugraz.at/krypto/hadeshash/-/blob/master/code/generate_parameters_grain.sage
// Partial rounds have been rounded up to the nearest multiple of `t`.
impl DefaultPoseidonParameters for Fq {
    const PARAMS_T3: PoseidonDefaultParametersEntry =
        PoseidonDefaultParametersEntry::new(2, 13, 8, 36, 0);
    const PARAMS_T9: PoseidonDefaultParametersEntry =
        PoseidonDefaultParametersEntry::new(8, 13, 8, 36, 0);
}

pub fn poseidon_mnt4_t3_parameters() -> PoseidonParameters<Fq> {
    create_parameters(Fq::PARAMS_T3)
}

pub fn poseidon_mnt4_t9_parameters() -> PoseidonParameters<Fq> {
    create_parameters(Fq::PARAMS_T9)
}
