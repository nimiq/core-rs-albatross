use nimiq_bls::AggregateSignature;
use nimiq_collections::bitset::BitSet;
use nimiq_handel::contribution::{AggregatableContribution, ContributionError};
use serde::{Deserialize, Serialize};

/*
This does not really belong here, but as there would otherwise be a cyclic dependency it needs to be here for now.
TODO: Move this out of primitives and into validator/aggregation once the messages crate is no longer required.
*/

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MultiSignature {
    pub signature: AggregateSignature,
    pub signers: BitSet,
}

impl MultiSignature {
    pub fn new(signature: AggregateSignature, signers: BitSet) -> Self {
        Self { signature, signers }
    }
}

impl AggregatableContribution for MultiSignature {
    fn contributors(&self) -> BitSet {
        self.signers.clone()
    }

    fn combine(&mut self, other: &MultiSignature) -> Result<(), ContributionError> {
        // TODO: If we don't need the overlapping IDs for the error, we can use `intersection_size`
        let overlap = &self.signers & &other.signers;

        if overlap.is_empty() {
            self.signature.merge_into(&other.signature);
            self.signers = &self.signers | &other.signers;
            Ok(())
        } else {
            Err(ContributionError::Overlapping(overlap))
        }
    }
}
