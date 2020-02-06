use super::*;

#[derive(Clone, Copy)]
pub struct PublicKey {
    pub(crate) public_key: G2Projective,
}

impl PublicKey {
    pub fn from_secret(x: &SecretKey) -> Self {
        PublicKey {
            public_key: G2Projective::prime_subgroup_generator() * &x.secret_key,
        }
    }

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        self.verify_hash(msg.hash(), signature)
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.verify_g1(Signature::hash_to_g1(hash), signature)
    }

    fn verify_g1(&self, hash_curve: G1Projective, signature: &Signature) -> bool {
        let lhs = Bls12_377::pairing(
            signature.signature,
            G2Projective::prime_subgroup_generator(),
        );
        let rhs = Bls12_377::pairing(hash_curve, self.public_key);
        lhs == rhs
    }
}

impl Eq for PublicKey {}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.public_key.eq(&other.public_key)
    }
}
