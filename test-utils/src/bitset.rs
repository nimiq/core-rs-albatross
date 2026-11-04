use nimiq_collections::BitSet;
use nimiq_serde::{Deserialize, Serialize};

/// Re-encodes `set` with `zero_limbs` trailing zero limbs appended, the only way a padded set can
/// arise: locally built sets are always canonical.
pub fn with_trailing_zero_limbs(set: &BitSet, zero_limbs: usize) -> BitSet {
    let mut limbs = Vec::<u64>::deserialize_from_vec(&set.serialize_to_vec()).unwrap();
    limbs.resize(limbs.len() + zero_limbs, 0);
    BitSet::deserialize_from_vec(&limbs.serialize_to_vec()).unwrap()
}
