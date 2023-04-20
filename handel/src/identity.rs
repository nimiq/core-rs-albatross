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
    pub fn as_vec(&self) -> Vec<usize> {
        self.signers.iter().collect()
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

pub trait IdentityRegistry: Send + Sync {
    /// Maps form Slot to PublicKey, returns None if the Slot does not exist.
    fn public_key(&self, id: usize) -> Option<PublicKey>;

    /// For a Set of Slots returns the Identity represented by those Slots.
    fn signers_identity(&self, slots: &BitSet) -> Identity;
}

pub trait WeightRegistry: Send + Sync {
    /// Given a Slot, returns the weight of that Slot.
    fn weight(&self, id: usize) -> Option<usize>;

    fn signers_weight(&self, slots: &BitSet) -> Option<usize> {
        let mut votes = 0;
        for slot in slots.iter() {
            votes += self.weight(slot)?;
        }
        Some(votes)
    }

    fn signature_weight<C: AggregatableContribution>(&self, contribution: &C) -> Option<usize> {
        self.signers_weight(&contribution.contributors())
    }
}
