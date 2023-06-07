use ark_crypto_primitives::sponge::poseidon::{PoseidonConfig, PoseidonDefaultConfigEntry};
use ark_mnt4_753::Fq;

use super::{create_parameters, DefaultPoseidonParameters};

// Parameters have been generated using the reference implementation here: https://extgit.iaik.tugraz.at/krypto/hadeshash/-/blob/master/code/generate_parameters_grain.sage
// Partial rounds have been rounded up to the nearest multiple of `t`.
impl DefaultPoseidonParameters for Fq {
    const PARAMS_T3: PoseidonDefaultConfigEntry = PoseidonDefaultConfigEntry::new(2, 13, 8, 36, 0);
    const PARAMS_T9: PoseidonDefaultConfigEntry = PoseidonDefaultConfigEntry::new(8, 13, 8, 36, 0);
}

pub fn poseidon_mnt4_t3_parameters() -> PoseidonConfig<Fq> {
    create_parameters(Fq::PARAMS_T3)
}

pub fn poseidon_mnt4_t9_parameters() -> PoseidonConfig<Fq> {
    create_parameters(Fq::PARAMS_T9)
}
