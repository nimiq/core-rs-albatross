use bls::PublicKey;
use collections::bitset::BitSet;

use crate::multisig::Signature;

pub trait IdentityRegistry {
    fn public_key(&self, id: usize) -> Option<PublicKey>;
}

pub trait WeightRegistry {
    fn weight(&self, id: usize) -> Option<usize>;

    fn signers_weight(&self, signers: &BitSet) -> Option<usize> {
        let mut votes = 0;
        for signer in signers.iter() {
            votes += self.weight(signer)?;
        }
        Some(votes)
    }

    fn signature_weight(&self, signature: &Signature) -> Option<usize> {
        match signature {
            Signature::Individual(individual) => self.weight(individual.signer),
            Signature::Multi(multisig) => self.signers_weight(&multisig.signers),
        }
    }
}
