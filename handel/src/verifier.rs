use std::future::Future;

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
pub trait Verifier: Send + Sync {
    type Contribution: AggregatableContribution;

    /// Verifies the correctness of `contribution`
    /// * `contribution` - The contribution to verify
    fn verify(
        &self,
        contribution: &Self::Contribution,
    ) -> impl Future<Output = VerificationResult> + Send;
}
