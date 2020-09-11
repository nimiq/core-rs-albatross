#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InconsistentStore,
    EmptyTree,
    ProveInvalidLeaves,
    InvalidProof,
    IncompleteProof,
    ProofOutOfOrder,
}
