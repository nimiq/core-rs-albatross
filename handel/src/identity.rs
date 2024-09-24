use std::ops::{BitXor, BitXorAssign};

use nimiq_bls::PublicKey;
use nimiq_collections::bitset::BitSet;

use crate::contribution::AggregatableContribution;

/// Struct that defines an identity.
/// An identity is composed of zero, one or more signers of a contribution.
#[derive(Clone, std::fmt::Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Identity {
    signers: BitSet,
}

impl Identity {
    /// Creates a new identity given a bitset of signers of a contribution.
    pub fn new(signers: BitSet) -> Self {
        Self { signers }
    }

    /// Returns the number of signers in this identity.
    pub fn len(&self) -> usize {
        self.signers.len()
    }

    /// Returns whether this identity is empty.
    pub fn is_empty(&self) -> bool {
        self.signers.is_empty()
    }

    pub fn contains(&self, value: usize) -> bool {
        self.signers.contains(value)
    }

    /// Returns whether this identity is a superset of another identity.
    pub fn is_superset_of(&self, other: &Self) -> bool {
        self.signers.is_superset(&other.signers)
    }

    /// Returns the complement signers from another identity.
    pub fn complement(&self, other: &Self) -> Self {
        Self {
            signers: &(&self.signers & &other.signers) ^ &other.signers,
        }
    }

    /// Returns the identity as a vector of signers.
    pub fn as_vec(&self) -> Vec<u16> {
        self.signers.iter().map(|i| i.try_into().unwrap()).collect()
    }

    /// Returns the intersection size with another identity.
    pub fn intersection_size(&self, other: &Self) -> usize {
        self.signers.intersection_size(&other.signers)
    }

    /// Perform bitwise or. Panics if overlap exists when flag is set.
    pub fn combine(&mut self, other: &Self, allow_intersection: bool) {
        if !allow_intersection && self.signers.intersection_size(&other.signers) > 0 {
            panic!("Identities overlap");
        }
        self.signers = &self.signers | &other.signers;
    }

    /// Returns an iterator for this identity.
    pub fn iter(&self) -> impl Iterator<Item = Identity> + '_ {
        self.signers.iter().map(|id| Identity {
            signers: BitSet::from_iter([id]),
        })
    }
}

impl BitXor for Identity {
    type Output = Identity;

    fn bitxor(self, other: Self) -> Self::Output {
        Identity {
            signers: self.signers ^ other.signers,
        }
    }
}

impl BitXor for &Identity {
    type Output = Identity;

    fn bitxor(self, other: Self) -> Self::Output {
        Identity {
            signers: &self.signers ^ &other.signers,
        }
    }
}

impl BitXorAssign for Identity {
    fn bitxor_assign(&mut self, other: Self) {
        self.signers ^= other.signers;
    }
}

/// A registry that maps slots to public keys or identities
pub trait IdentityRegistry: Send + Sync {
    /// Maps from Slot to PublicKey, returns None if the Slot does not exist.
    fn public_key(&self, id: usize) -> Option<PublicKey>;

    /// For a Set of Slots returns the Identity represented by those Slots.
    fn signers_identity(&self, slots: &BitSet) -> Identity;
}

/// A registry that maps slots to the corresponding signer's weight.
pub trait WeightRegistry: Send + Sync {
    /// Given a Slot, returns the weight of that Slot.
    fn weight(&self, id: usize) -> Option<usize>;

    /// Given a set of slots, returns the weight of all of the provided slots.
    fn signers_weight(&self, slots: &BitSet) -> Option<usize> {
        let mut votes = 0;
        for slot in slots.iter() {
            votes += self.weight(slot)?;
        }
        Some(votes)
    }

    /// Returns the total weight of the signers in a signature
    fn signature_weight<C: AggregatableContribution>(&self, contribution: &C) -> Option<usize> {
        self.signers_weight(&contribution.contributors())
    }
}

#[cfg(test)]
mod test {
    use nimiq_collections::bitset::BitSet;
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn empty_identity() {
        let identity = Identity::new(BitSet::new());

        assert!(identity.is_empty());
        assert!(identity.is_empty());
    }

    #[test]
    fn it_computes_superset() {
        let empty_identity = Identity::new(BitSet::new());

        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        bitset.insert(64);
        bitset.insert(93);
        let identity_1 = Identity::new(bitset);

        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(10);
        bitset.insert(43);
        bitset.insert(76);
        let identity_2 = Identity::new(bitset);

        // Identity 3 is a subset of identity 1
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        let identity_3 = Identity::new(bitset);

        // Test-out super set ops
        assert!(identity_1.is_superset_of(&empty_identity));
        assert!(identity_2.is_superset_of(&empty_identity));
        assert!(identity_1.is_superset_of(&identity_3));
        assert!(!identity_3.is_superset_of(&identity_1));
        assert!(!identity_1.is_superset_of(&identity_2));
        assert!(!identity_2.is_superset_of(&identity_1));
        assert!(!identity_2.is_superset_of(&identity_3));
        assert!(!identity_3.is_superset_of(&identity_2));
    }

    #[test]
    fn it_computes_complement() {
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        bitset.insert(64);
        bitset.insert(93);
        let identity_1 = Identity::new(bitset);

        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(10);
        bitset.insert(43);
        bitset.insert(76);
        let identity_2 = Identity::new(bitset);

        // Identity 3 is a subset of identity 1
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        let identity_3 = Identity::new(bitset);

        // Expected complement of identity 1 and 2
        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(43);
        bitset.insert(76);
        let exp_cmp_identity_1_2 = Identity::new(bitset);
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(64);
        bitset.insert(93);
        let exp_cmp_identity_2_1 = Identity::new(bitset);

        // Expected complement of identity 2 and 3
        let mut bitset = BitSet::new();
        bitset.insert(2);
        let exp_cmp_identity_2_3 = Identity::new(bitset);
        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(43);
        bitset.insert(76);
        let exp_cmp_identity_3_2 = Identity::new(bitset);

        // Expected complement of identity 1 and 3
        let bitset = BitSet::new();
        let exp_cmp_identity_1_3 = Identity::new(bitset);
        let mut bitset = BitSet::new();
        bitset.insert(64);
        bitset.insert(93);
        let exp_cmp_identity_3_1 = Identity::new(bitset);

        // Test-out super set ops
        assert_eq!(identity_1.complement(&identity_3), exp_cmp_identity_1_3);
        assert_eq!(identity_3.complement(&identity_1), exp_cmp_identity_3_1);
        assert_eq!(identity_1.complement(&identity_2), exp_cmp_identity_1_2);
        assert_eq!(identity_2.complement(&identity_1), exp_cmp_identity_2_1);
        assert_eq!(identity_2.complement(&identity_3), exp_cmp_identity_2_3);
        assert_eq!(identity_3.complement(&identity_2), exp_cmp_identity_3_2);
    }

    #[test]
    fn it_computes_intersection_size() {
        let empty_identity = Identity::new(BitSet::new());

        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        bitset.insert(64);
        bitset.insert(93);
        let identity_1 = Identity::new(bitset);

        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(15);
        bitset.insert(64);
        bitset.insert(76);
        let identity_2 = Identity::new(bitset);

        // Identity 3 is a subset of identity 1
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        let identity_3 = Identity::new(bitset);

        // Test-out intersection size
        assert_eq!(identity_1.intersection_size(&empty_identity), 0);
        assert_eq!(identity_2.intersection_size(&empty_identity), 0);
        assert_eq!(identity_1.intersection_size(&identity_3), 2);
        assert_eq!(identity_3.intersection_size(&identity_1), 2);
        assert_eq!(identity_1.intersection_size(&identity_2), 1);
        assert_eq!(identity_2.intersection_size(&identity_1), 1);
        assert_eq!(identity_2.intersection_size(&identity_3), 0);
        assert_eq!(identity_3.intersection_size(&identity_2), 0);
    }

    #[test]
    fn it_can_combine_allowing_intersection() {
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        bitset.insert(64);
        bitset.insert(93);
        let identity_1 = Identity::new(bitset);

        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(10);
        bitset.insert(43);
        bitset.insert(76);
        let identity_2 = Identity::new(bitset);

        // Identity 3 is a subset of identity 1
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        let identity_3 = Identity::new(bitset);

        // Expected combined identity of identity 1 and 2
        let mut bitset = identity_1.signers.clone();
        bitset.insert(7);
        bitset.insert(43);
        bitset.insert(76);
        let exp_cmb_identity_1_2 = Identity::new(bitset);

        // Expected combined identity of identity 2 and 3
        let mut bitset = identity_2.signers.clone();
        bitset.insert(2);
        let exp_cmb_identity_2_3 = Identity::new(bitset);

        // Expected combined identity of identity 1 and 3
        let exp_cmb_identity_1_3 = identity_1.clone();

        // Test-out combine op allowing intersection
        let mut cmb_identity_1_3 = identity_1.clone();
        cmb_identity_1_3.combine(&identity_3, true);
        let mut cmb_identity_3_1 = identity_3.clone();
        cmb_identity_3_1.combine(&identity_1, true);
        let mut cmb_identity_1_2 = identity_1.clone();
        cmb_identity_1_2.combine(&identity_2, true);
        let mut cmb_identity_2_1 = identity_2.clone();
        cmb_identity_2_1.combine(&identity_1, true);
        let mut cmb_identity_2_3 = identity_2.clone();
        cmb_identity_2_3.combine(&identity_3, true);
        let mut cmb_identity_3_2 = identity_3.clone();
        cmb_identity_3_2.combine(&identity_2, true);

        assert_eq!(cmb_identity_1_3, exp_cmb_identity_1_3);
        assert_eq!(cmb_identity_3_1, exp_cmb_identity_1_3);
        assert_eq!(cmb_identity_1_2, exp_cmb_identity_1_2);
        assert_eq!(cmb_identity_2_1, exp_cmb_identity_1_2);
        assert_eq!(cmb_identity_2_3, exp_cmb_identity_2_3);
        assert_eq!(cmb_identity_3_2, exp_cmb_identity_2_3);
    }

    #[test]
    #[should_panic]
    fn it_cant_combine_without_intersection() {
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        bitset.insert(64);
        bitset.insert(93);
        let identity_1 = Identity::new(bitset);

        // Identity 3 is a subset of identity 1
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        let identity_3 = Identity::new(bitset);

        // Test-out combine op without allowing intersection
        let mut cmb_identity_1_3 = identity_1.clone();
        // Since identities 1 and 3 intersect, there must be a panic in the next line
        cmb_identity_1_3.combine(&identity_3, false);
    }

    #[test]
    fn it_computes_xor() {
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        bitset.insert(64);
        bitset.insert(93);
        let identity_1 = Identity::new(bitset);

        let mut bitset = BitSet::new();
        bitset.insert(7);
        bitset.insert(10);
        bitset.insert(43);
        bitset.insert(76);
        let identity_2 = Identity::new(bitset);

        // Identity 3 is a subset of identity 1
        let mut bitset = BitSet::new();
        bitset.insert(2);
        bitset.insert(10);
        let identity_3 = Identity::new(bitset);

        // Expected xor of identity 1 and 2
        let mut bitset = identity_1.signers.clone();
        bitset.insert(7);
        bitset.insert(43);
        bitset.insert(76);
        bitset.remove(10);
        let exp_xor_identity_1_2 = Identity::new(bitset);

        // Expected xor of identity 2 and 3
        let mut bitset = identity_2.signers.clone();
        bitset.insert(2);
        bitset.remove(10);
        let exp_xor_identity_2_3 = Identity::new(bitset);

        // Expected xor of identity 1 and 3
        let mut bitset = identity_1.signers.clone();
        bitset.remove(2);
        bitset.remove(10);
        let exp_xor_identity_1_3 = Identity::new(bitset);

        // Test-out xor op
        assert_eq!(
            identity_1.clone() ^ identity_3.clone(),
            exp_xor_identity_1_3
        );
        assert_eq!(
            identity_3.clone() ^ identity_1.clone(),
            exp_xor_identity_1_3
        );
        assert_eq!(
            identity_1.clone() ^ identity_2.clone(),
            exp_xor_identity_1_2
        );
        assert_eq!(identity_2.clone() ^ identity_1, exp_xor_identity_1_2);
        assert_eq!(
            identity_2.clone() ^ identity_3.clone(),
            exp_xor_identity_2_3
        );
        assert_eq!(identity_3 ^ identity_2, exp_xor_identity_2_3);
    }
}
