use super::*;

/// Simply a struct combining the secret key and the public key types.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyPair {
    pub secret_key: SecretKey,
    pub public_key: PublicKey,
}

impl KeyPair {
    /// Derives a key pair from a secret key. This function will panic if it is given zero as an input.
    pub fn from_secret(x: &SecretKey) -> Self {
        KeyPair::from(x.clone())
    }

    /// Signs a message using the key pair.
    pub fn sign<M: Hash>(&self, msg: &M) -> Signature {
        self.secret_key.sign::<M>(msg)
    }

    /// Signs a hash using the key pair.
    pub fn sign_hash(&self, hash: SigHash) -> Signature {
        self.secret_key.sign_hash(hash)
    }

    /// Verifies a signature of a message using the key pair.
    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        self.public_key.verify::<M>(msg, signature)
    }

    /// Verifies a signature of a hash using the key pair.
    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        self.public_key.verify_hash(hash, signature)
    }
}

impl SecureGenerate for KeyPair {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        let secret = SecretKey::generate(rng);
        KeyPair::from(secret)
    }
}

impl From<SecretKey> for KeyPair {
    /// Derives a key pair from a secret key. This function will produce an error if it is given zero as an input.
    fn from(secret: SecretKey) -> Self {
        let public = PublicKey::from_secret(&secret);
        return KeyPair {
            secret_key: secret,
            public_key: public,
        };
    }
}

#[cfg(feature = "beserial")]
impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&::hex::encode(self.serialize_to_vec()))
    }
}
