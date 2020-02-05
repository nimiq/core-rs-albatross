use super::*;

#[derive(Clone, Copy)]
pub struct SecretKey {
    pub(crate) secret_key: Fr,
}

impl SecretKey {
    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.sign_hash(msg.hash())
    }

    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.sign_g1(hash_to_g1(hash))
    }

    fn sign_g1(&self, hash_curve: G1Projective) -> Signature {
        Signature {
            signature: hash_curve * &self.secret_key,
        }
    }
}

impl SecureGenerate for SecretKey {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        SecretKey {
            secret_key: Fr::random(rng),
        }
    }
}

impl Eq for SecretKey {}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.secret_key.eq(&other.secret_key)
    }
}
