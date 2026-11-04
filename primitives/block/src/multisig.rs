use log::debug;
use nimiq_bls::{AggregateSignature, LazyAggregateSignature};
use nimiq_collections::bitset::BitSet;
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};
use serde::Deserializer;

/*
This does not really belong here, but as there would otherwise be a cyclic dependency it needs to be here for now.
TODO: Move this out of primitives and into validator/aggregation once the messages crate is no longer required.
*/

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct MultiSignature {
    /// Kept compressed so that deserializing stays cheap, it is only uncompressed when it is
    /// verified or combined.
    pub signature: LazyAggregateSignature,
    #[serde(deserialize_with = "deserialize_canonical_signers")]
    #[serialize_size(bitset_max_elem = Policy::SLOTS as usize)]
    pub signers: BitSet,
}

/// Decodes a signer set and drops trailing zero limbs. They signal no slot, the signature does not
/// depend on the encoding and no block hash covers a justification, so nothing a verifier checks
/// changes; keeping them would let any peer inflate a stored proof or a handel contribution. The
/// allocation they occupied is released too, so a padded copy costs no more memory than a canonical one.
fn deserialize_canonical_signers<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BitSet, D::Error> {
    let mut signers = <BitSet as serde::Deserialize>::deserialize(deserializer)?;
    if !signers.is_canonical() {
        debug!("Dropping trailing zero limbs from a signer set");
        signers.canonicalize();
    }
    Ok(signers)
}

impl MultiSignature {
    pub fn new(signature: AggregateSignature, signers: BitSet) -> Self {
        Self {
            signature: signature.into(),
            signers,
        }
    }

    pub fn contributors(&self) -> BitSet {
        self.signers.clone()
    }

    /// Combines `other` into this multisignature.
    ///
    /// Fails with the overlapping signers if both multisignatures share a signer, leaving `self`
    /// unchanged.
    ///
    /// # Panics
    ///
    /// Panics if either signature is not a valid curve point. Signatures that were verified or
    /// created locally always are. Handel only combines such contributions, since its verifiers
    /// return `Forged` for an invalid point before a contribution is stored.
    pub fn combine(&mut self, other: &MultiSignature) -> Result<(), BitSet> {
        // TODO: If we don't need the overlapping IDs for the error, we can use `intersection_size`
        let overlap = &self.signers & &other.signers;

        if overlap.is_empty() {
            let mut signature = *self.signature.uncompress().expect("Signature is verified");
            signature.merge_into(other.signature.uncompress().expect("Signature is verified"));
            self.signature = signature.into();
            self.signers = &self.signers | &other.signers;
            Ok(())
        } else {
            Err(overlap)
        }
    }
}

pub(crate) fn checked_signer_slots(signers: &BitSet) -> Option<Vec<u16>> {
    let mut slots = Vec::with_capacity(signers.len());
    for slot in signers.iter() {
        if slot >= Policy::SLOTS as usize || slot > u16::MAX as usize {
            return None;
        }
        slots.push(slot as u16);
    }
    Some(slots)
}

#[cfg(test)]
mod tests {
    use nimiq_collections::BitSet;
    use nimiq_serde::{Deserialize, Serialize};
    use nimiq_test_log::test;
    use nimiq_test_utils::bls::signature_not_on_curve;

    use super::MultiSignature;

    #[test]
    fn deserializing_does_not_uncompress_the_signature() {
        let multisig = MultiSignature {
            signature: signature_not_on_curve().into(),
            signers: BitSet::from_iter([0]),
        };

        let decoded = MultiSignature::deserialize_from_vec(&multisig.serialize_to_vec()).unwrap();

        assert!(!decoded.signature.has_uncompressed());
        assert!(decoded.signature.uncompress().is_none());
    }
}
