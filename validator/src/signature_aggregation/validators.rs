use std::sync::Arc;
use std::collections::HashMap;

use primitives::validators::Validators;
use bls::bls12_381::PublicKey;
use network::Peer;

use handel::identity::{IdentityRegistry, WeightRegistry};
use handel::verifier::Verifier;



pub struct ValidatorRegistry {
    validators: Validators,
    peers: HashMap<usize, Arc<Peer>>,
}


impl ValidatorRegistry {
    pub fn new(validators: Validators, peers: HashMap<usize, Arc<Peer>>) -> Self {
        Self {
            validators,
            peers
        }
    }

    pub fn peer(&self, id: usize) -> Option<Arc<Peer>> {
        self.peers.get(&id).cloned()
    }
}


impl IdentityRegistry for ValidatorRegistry {
    fn public_key(&self, id: usize) -> Option<PublicKey> {
        self.validators.get(id).and_then(|validator| validator.1.uncompressed())
    }
}

impl WeightRegistry for ValidatorRegistry {
    fn weight(&self, id: usize) -> Option<usize> {
        self.validators.get(id).map(|validator| validator.0 as usize)
    }
}
