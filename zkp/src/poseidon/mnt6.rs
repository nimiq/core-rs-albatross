use super::{create_parameters, DefaultPoseidonParameters};
use ark_mnt6_753::Fq;
use ark_sponge::poseidon::{PoseidonConfig, PoseidonDefaultConfigEntry};

// Parameters have been generated using the reference implementation here: https://extgit.iaik.tugraz.at/krypto/hadeshash/-/blob/master/code/generate_parameters_grain.sage
// Partial rounds have been rounded up to the nearest multiple of `t`.
impl DefaultPoseidonParameters for Fq {
    const PARAMS_T3: PoseidonDefaultConfigEntry = PoseidonDefaultConfigEntry::new(2, 11, 8, 39, 0);
    const PARAMS_T9: PoseidonDefaultConfigEntry = PoseidonDefaultConfigEntry::new(8, 11, 8, 40, 0);
}

pub fn poseidon_mnt6_t3_parameters() -> PoseidonConfig<Fq> {
    create_parameters(Fq::PARAMS_T3)
}

pub fn poseidon_mnt6_t9_parameters() -> PoseidonConfig<Fq> {
    create_parameters(Fq::PARAMS_T9)
}
