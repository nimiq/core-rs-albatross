extern crate curve25519_dalek;
extern crate rand;
extern crate sha2;

use self::rand::{OsRng, Rng};
use self::sha2::Digest;
use self::curve25519_dalek::scalar::Scalar;
use self::curve25519_dalek::edwards::EdwardsPoint;
use std::ops::Add;
use std::ops::AddAssign;

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

#[derive(PartialEq,Eq)]
pub struct CommitmentPair {
    random_secret: RandomSecret,
    commitment: Commitment
}

impl CommitmentPair {
    pub fn generate() -> Option<CommitmentPair> {
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
            return None;
        }

        // Compute the point [scalar]B.
        let commitment: EdwardsPoint = &scalar * &curve25519_dalek::constants::ED25519_BASEPOINT_TABLE;

        let rs = RandomSecret(scalar);
        let ct = Commitment(commitment);
        return Some(CommitmentPair { random_secret: rs, commitment: ct });
    }
}

#[derive(PartialEq,Eq)]
pub struct PartialSignature {

}
