use nimiq_bls::AggregateSignature;
use nimiq_collections::bitset::BitSet;
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};

/*
This does not really belong here, but as there would otherwise be a cyclic dependency it needs to be here for now.
TODO: Move this out of primitives and into validator/aggregation once the messages crate is no longer required.
*/

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct MultiSignature {
    pub signature: AggregateSignature,
    #[serialize_size(bitset_max_elem = Policy::SLOTS as usize)]
    pub signers: BitSet,
}

impl MultiSignature {
    pub fn new(signature: AggregateSignature, signers: BitSet) -> Self {
        Self { signature, signers }
    }

    pub fn contributors(&self) -> BitSet {
        self.signers.clone()
    }

    pub fn combine(&mut self, other: &MultiSignature) -> Result<(), BitSet> {
        // TODO: If we don't need the overlapping IDs for the error, we can use `intersection_size`
        let overlap = &self.signers & &other.signers;

        if overlap.is_empty() {
            self.signature.merge_into(&other.signature);
            self.signers = &self.signers | &other.signers;
            Ok(())
        } else {
            Err(overlap)
        }
    }
}
