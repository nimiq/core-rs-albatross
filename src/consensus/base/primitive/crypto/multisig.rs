use rand::{OsRng, Rng};
use sha2::{self,Digest};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::constants;
use std::ops::Add;
use std::ops::AddAssign;
use std::fmt;
use std::error;
use super::{KeyPair,PublicKey};

#[derive(PartialEq,Eq)]
pub struct RandomSecret(Scalar);
#[derive(PartialEq,Eq)]
pub struct Commitment(EdwardsPoint);

impl<'a, 'b> Add<&'b Commitment> for &'a Commitment {
    type Output = Commitment;
    fn add(self, other: &'b Commitment) -> Commitment {
        Commitment(self.0 + other.0)
    }
}
impl<'b> Add<&'b Commitment> for Commitment {
    type Output = Commitment;
    fn add(self, rhs: &'b Commitment) -> Commitment {
        &self + rhs
    }
}

impl<'a> Add<Commitment> for &'a Commitment {
    type Output = Commitment;
    fn add(self, rhs: Commitment) -> Commitment {
        self + &rhs
    }
}

impl Add<Commitment> for Commitment {
    type Output = Commitment;
    fn add(self, rhs: Commitment) -> Commitment {
        &self + &rhs
    }
}

#[derive(Debug, Clone)]
pub struct InvalidScalarError;

impl fmt::Display for InvalidScalarError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        return write!(f, "Generated scalar was invalid (0 or 1).");
    }
}

impl error::Error for InvalidScalarError {
    fn description(&self) -> &str {
        "Generated scalar was invalid (0 or 1)."
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

#[derive(PartialEq,Eq)]
pub struct CommitmentPair {
    random_secret: RandomSecret,
    commitment: Commitment
}

impl CommitmentPair {
    pub fn generate() -> Result<CommitmentPair, InvalidScalarError> {
        // Create random 32 bytes.
        let mut cspring: OsRng = OsRng::new().unwrap();
        let mut randomness: [u8; 32] = [0u8; 32];
        cspring.fill_bytes(&mut randomness);

        // Decompress the 32 byte cryptographically secure random data to 64 byte.
        let mut h: sha2::Sha512 = sha2::Sha512::default();
        let mut hash:  [u8; 64] = [0u8; 64];

        h.input(&randomness);
        hash.copy_from_slice(h.result().as_slice());

        // Reduce to valid scalar.
        let scalar = Scalar::from_bytes_mod_order_wide(&hash);
        if scalar == Scalar::zero() || scalar == Scalar::one() {
            return Err(InvalidScalarError);
        }

        // Compute the point [scalar]B.
        let commitment: EdwardsPoint = &scalar * &constants::ED25519_BASEPOINT_TABLE;

        let rs = RandomSecret(scalar);
        let ct = Commitment(commitment);
        return Ok(CommitmentPair { random_secret: rs, commitment: ct });
    }
}

#[derive(PartialEq,Eq)]
pub struct PartialSignature ([u8; 32]);

pub fn partial_signature_create<Key: Into<KeyPair>>(key: &Key, public_keys: &Vec<PublicKey>, secret: &RandomSecret, commitments: &Vec<Commitment>, data: &[u8]) -> (PartialSignature, PublicKey, Commitment) {
    if public_keys.len() != commitments.len() {
        panic!("Number of public keys and commitments must be the same.");
    }
    if public_keys.len() == 0 {
        panic!("Number of public keys and commitments must be greater than 0.");
    }

    unimplemented!();
}
