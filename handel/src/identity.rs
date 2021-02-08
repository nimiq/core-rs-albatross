use bls::PublicKey;
use collections::bitset::BitSet;

use crate::contribution::AggregatableContribution;

#[derive(Clone, std::fmt::Debug)]
pub enum Identity {
    Single(usize),
    Multiple(Vec<usize>),
    None,
}

impl Identity {
    pub fn as_bitset(&self) -> BitSet {
        let mut bitset = BitSet::new();
        match self {
            Self::Single(id) => bitset.insert(*id),
            Self::Multiple(ids) => ids.iter().for_each(|id| bitset.insert(*id)),
            Self::None => {}
        }
        bitset
    }
}

pub trait IdentityRegistry: Send + Sync {
    fn public_key(&self, id: usize) -> Option<PublicKey>;

    fn signers_identity(&self, signers: &BitSet) -> Identity;
}

pub trait WeightRegistry: Send + Sync {
    fn weight(&self, id: usize) -> Option<usize>;

    fn signers_weight(&self, signers: &BitSet) -> Option<usize> {
        let mut votes = 0;
        for signer in signers.iter() {
            votes += self.weight(signer)?;
        }
        Some(votes)
    }

    fn signature_weight<C: AggregatableContribution>(&self, contribution: &C) -> Option<usize> {
        self.signers_weight(&contribution.contributors())
    }
}
