use crate::contribution::AggregatableContribution;
use async_trait::async_trait;

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
#[async_trait]
pub trait Verifier: Send + Sync {
    type Contribution: AggregatableContribution;

    /// Verifies the correectness of `contribution`
    /// * `contribution` - The contribution to verify
    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult;
}
