use std::fmt;

use ark_ff::Zero;
use ark_mnt6_753::G1Projective;

use crate::{CompressedSignature, Signature};

/// An aggregate signature. Mathematically, it is equivalent to a regular signature. However, we created a new type for it in order to help differentiate between the two use cases.
#[derive(Clone, Copy)]
#[cfg_attr(
    feature = "serde-derive",
    derive(serde::Serialize, serde::Deserialize, nimiq_serde::SerializedSize),
    serde(transparent)
)]
pub struct AggregateSignature(pub Signature);

impl AggregateSignature {
    /// Creates a new "empty" aggregate signature. It is simply the identity element of the elliptic curve, also known as the point at infinity.
    pub fn new() -> Self {
        let signature = G1Projective::zero();
        AggregateSignature(Signature {
            signature,
            compressed: CompressedSignature::from(signature),
        })
    }

    /// Creates an aggregated signature from an array of regular signatures.
    pub fn from_signatures(sigs: &[Signature]) -> Self {
        let mut agg_sig = G1Projective::zero();
        for x in sigs {
            agg_sig += &x.signature;
        }
        AggregateSignature(Signature {
            signature: agg_sig,
            compressed: CompressedSignature::from(agg_sig),
        })
    }

    /// Adds a single regular signature to an aggregated signature.
    pub fn aggregate(&mut self, sig: &Signature) {
        self.0.signature += &sig.signature;
        self.0.compressed = CompressedSignature::from(self.0.signature);
    }

    /// Merges two aggregated signatures.
    pub fn merge_into(&mut self, other: &Self) {
        self.0.signature += &other.0.signature;
        self.0.compressed = CompressedSignature::from(self.0.signature);
    }

    pub fn get_point(&self) -> G1Projective {
        self.0.signature
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
