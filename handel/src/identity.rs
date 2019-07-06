use bls::bls12_381::PublicKey;

use crate::multisig::Signature;


pub trait IdentityRegistry {
    fn public_key(&self, id: usize) -> Option<PublicKey>;
}

pub trait WeightRegistry {
    fn weight(&self, id: usize) -> Option<usize>;

    fn signature_weight(&self, signature: &Signature) -> Option<usize> {
        match signature {
            Signature::Individual(individual) => {
                self.weight(individual.signer)
            },
            Signature::Multi(multisig) => {
                let mut w = 0;
                for signer in multisig.signers.iter() {
                    w += self.weight(signer)?;
                }
                Some(w)
            }
        }
    }
}
