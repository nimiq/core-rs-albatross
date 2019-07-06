use primitives::validators::Validators;
use bls::bls12_381::PublicKey;

use handel::identity::{IdentityRegistry, WeightRegistry};
use handel::verifier::Verifier;



pub struct ValidatorRegistry(Validators);


impl ValidatorRegistry {
    pub fn new(validators: Validators) -> Self {
        Self(validators)
    }
}


impl IdentityRegistry for ValidatorRegistry {
    fn public_key(&self, id: usize) -> Option<PublicKey> {
        self.0.get(id).and_then(|validator| validator.1.uncompressed())
    }
}

impl WeightRegistry for ValidatorRegistry {
    fn weight(&self, id: usize) -> Option<usize> {
        self.0.get(id).map(|validator| validator.0 as usize)
    }
}
