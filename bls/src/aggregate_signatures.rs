use super::*;

#[derive(Clone, Copy)]
pub struct AggregateSignature(pub Signature);

impl AggregateSignature {
    pub fn new() -> Self {
        AggregateSignature(Signature {
            signature: G1Projective::zero(),
        })
    }

    pub fn from_signatures(sigs: &[Signature]) -> Self {
        let mut agg_sig = G1Projective::zero();
        for x in sigs {
            agg_sig += &x.signature;
        }
        return AggregateSignature(Signature { signature: agg_sig });
    }

    pub fn aggregate(&mut self, sig: &Signature) {
        self.0.signature += &sig.signature;
    }

    pub fn merge_into(&mut self, other: &Self) {
        self.0.signature += &other.0.signature;
    }

    // Is this even needed? Placement here clashes with the rest of the verify methods being part of the public keys impl.
    // Maybe make all verify methods part of the Signature type, and remove them from the Keypair type.
    pub fn verify<M: Hash>(&self, public_keys: &[PublicKey], msgs: &[M]) -> bool {
        // Number of messages must coincide with number of public keys.
        if public_keys.len() != msgs.len() {
            panic!("Different amount of messages and public keys");
        }

        // compute hashes
        let mut hashes: Vec<SigHash> = msgs.iter().rev().map(|msg| msg.hash::<SigHash>()).collect();

        // Check pairings.
        let lhs = Bls12_377::pairing(self.0.signature, G2Projective::prime_subgroup_generator());
        let mut rhs = Fq12::one();
        for x in public_keys {
            // guaranteed to be available, since we check that there are as many messages/hashes
            // as public_keys.
            let h = hashes.pop().unwrap();
            rhs *= &Bls12_377::pairing(Signature::hash_to_g1(h), x.public_key);
        }
        lhs == rhs
    }
}

impl Eq for AggregateSignature {}

impl PartialEq for AggregateSignature {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Default for AggregateSignature {
    fn default() -> Self {
        Self::new()
    }
}
