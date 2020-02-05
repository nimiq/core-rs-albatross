use super::*;

#[derive(Clone, Copy)]
pub struct Signature {
    pub(crate) signature: G1Projective,
}

impl Signature {
    // Map hash to point in G1Projective
    // pub(crate) fn hash_to_g1(h: SigHash) -> G1Projective {
    //     G1Projective::random(&mut ChaChaRng::from_seed(h.into()))
    // }
}

impl Eq for Signature {}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.signature.eq(&other.signature)
    }
}
