use futures::Future;

use crate::contribution::AggregatableContribution;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationResult {
    Ok,
    UnknownSigner { signer: usize },
    Forged,
}

impl VerificationResult {
    pub fn is_ok(&self) -> bool {
        *self == VerificationResult::Ok
    }
}

/// Trait for a signature verification backend
pub trait Verifier {
    type Contribution: AggregatableContribution;
    type Output: Future<Item = VerificationResult, Error = ()> + Send + Sync + 'static;

    fn verify(&self, signature: &Self::Contribution) -> Self::Output;
}

// /// A dummy verifier that accepts all signatures
// pub struct DummyVerifier();

// impl Verifier for DummyVerifier {
//     type Output = FutureResult<VerificationResult, ()>;

//     fn verify(&self, _signature: &Signature) -> Self::Output {
//         future::ok(VerificationResult::Ok)
//     }
// }
