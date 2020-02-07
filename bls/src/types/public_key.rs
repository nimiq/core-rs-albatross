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

    pub fn verify_g1(&self, hash_curve: G1Projective, signature: &Signature) -> bool {
        let lhs = Bls12_377::pairing(
            signature.signature,
            G2Projective::prime_subgroup_generator(),
        );
        let rhs = Bls12_377::pairing(hash_curve, self.public_key);
        lhs == rhs
    }

    pub fn compress(&self) -> CompressedPublicKey {
        let mut buffer = [0u8; 96];
        self.public_key
            .into_affine()
            .serialize(&[], &mut buffer)
            .unwrap();
        CompressedPublicKey { public_key: buffer }
    }
}

impl Eq for PublicKey {}

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.public_key.eq(&other.public_key)
    }
}

impl PartialOrd<PublicKey> for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.public_key
            .into_affine()
            .lexicographic_cmp(&other.public_key.into_affine())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PublicKey({})", self.compress().to_hex())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.compress().to_hex())
    }
}
