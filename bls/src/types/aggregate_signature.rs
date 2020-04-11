use std::fmt;

use algebra::mnt6_753::G1Projective;
use num_traits::Zero;

pub use utils::key_rng::{SecureGenerate, SecureRng};

use crate::Signature;

/// An aggregate signature. Mathematically, it is equivalent to a regular signature. However, we created a new type for it in order to help differentiate between the two use cases.
#[derive(Clone, Copy)]
pub struct AggregateSignature(pub Signature);

impl AggregateSignature {
    /// Creates a new "empty" aggregate signature. It is simply the identity element of the elliptic curve, also known as the point at infinity.
    pub fn new() -> Self {
        AggregateSignature(Signature {
            signature: G1Projective::zero(),
        })
    }

    /// Creates an aggregated signature from an array of regular signatures.
    pub fn from_signatures(sigs: &[Signature]) -> Self {
        let mut agg_sig = G1Projective::zero();
        for x in sigs {
            agg_sig += &x.signature;
        }
        return AggregateSignature(Signature { signature: agg_sig });
    }

    /// Adds a single regular signature to an aggregated signature.
    pub fn aggregate(&mut self, sig: &Signature) {
        self.0.signature += &sig.signature;
    }

    /// Merges two aggregated signatures.
    pub fn merge_into(&mut self, other: &Self) {
        self.0.signature += &other.0.signature;
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

impl fmt::Display for AggregateSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Debug::fmt(&self.0, f)
    }
}
