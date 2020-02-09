use super::*;

#[derive(Clone, Copy)]
pub struct SecretKey {
    /// This is simply a number in the finite field Fr.
    /// Fr is also the prime subgroup of the elliptic curve.
    pub secret_key: Fr,
}

impl SecretKey {
    pub const SIZE: usize = 32;

    /// Creates a signature given a message.
    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.sign_hash(msg.hash())
    }

    /// Creates a signature given a hash.
    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.sign_g1(Signature::hash_to_g1(hash))
    }

    /// Creates a signature given a G1 point.
    fn sign_g1(&self, hash_curve: G1Projective) -> Signature {
        Signature {
            signature: hash_curve * &self.secret_key,
        }
    }
}

impl SecureGenerate for SecretKey {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        SecretKey {
            secret_key: Fr::rand(rng),
        }
    }
}

impl Eq for SecretKey {}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.secret_key.eq(&other.secret_key)
    }
}

#[cfg(feature = "beserial")]
impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&::hex::encode(self.serialize_to_vec()))
    }
}
