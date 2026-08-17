use std::{collections::HashSet, ops};

use nimiq_bls::PublicKey;
use nimiq_collections::BitSet;
use nimiq_handel::identity::{Identity, IdentityRegistry, WeightRegistry};
use nimiq_primitives::{policy::Policy, slots_allocation::Validators};

/// Implementation for Handel registry using a `Validators` list.
#[derive(Debug)]
pub(crate) struct ValidatorRegistry {
    validators: Validators,
}

impl ValidatorRegistry {
    pub fn new(validators: Validators) -> Self {
        Self { validators }
    }

    pub fn len(&self) -> usize {
        self.validators.num_validators()
    }

    pub fn get_slots(&self, idx: u16) -> ops::Range<u16> {
        self.validators.validators[idx as usize].slots.clone()
    }
}

/// Narrows a slot number to the `u16` the `Validators` accessors take, rejecting slots that do not
/// exist. The bound must be checked before the cast, as truncating first maps out of range slots
/// onto existing ones. `Validators::get_band_from_slot` panics on anything this rejects.
fn checked_slot(slot: usize) -> Option<u16> {
    (slot < Policy::SLOTS as usize).then_some(slot as u16)
}

impl IdentityRegistry for ValidatorRegistry {
    fn public_key(&self, slot_number: usize) -> Option<PublicKey> {
        self.validators
            // Get the validator owning the slot, if the slot exists
            .get_validator_by_slot_number(checked_slot(slot_number)?)
            // Get the public key for this validator
            .voting_key
            // and uncompress it
            .uncompress()
            .copied()
    }

    fn signers_identity(&self, slots: &BitSet) -> Identity {
        if slots.is_empty() {
            // if there is no signers there is no identity.
            return Identity::new(BitSet::default());
        }

        // Create a set of validator ids corresponding to the slots
        let mut ids: HashSet<u16> = HashSet::new();
        for slot in slots.iter() {
            // Reject the whole set if it references a slot that does not exist.
            let Some(slot) = checked_slot(slot) else {
                return Identity::new(BitSet::default());
            };
            // Insert each validator_address if there is one.
            let _ = ids.insert(self.validators.get_band_from_slot(slot));
        }

        // If there is no signer it needs to be rejected.
        // Should never happen as those get rejected earlier.
        if ids.is_empty() {
            return Identity::new(BitSet::default());
        }

        // Make sure that the bitset for the given validator's slots is exactly the same as the once given,
        // as otherwise there would be a 'partial' slot layout for at least one of the validators.
        // Since handel will not combine overlapping contributions those need to be rejected.
        // This holds for Single and Multiple signatories.
        let mut validators_slots = BitSet::new();
        for validator_id in ids.iter() {
            for slot in self.get_slots(*validator_id) {
                validators_slots.insert(slot as usize);
            }
        }

        if &validators_slots != slots {
            // Reject any slots which are not exhaustive for their validators.
            return Identity::new(BitSet::default());
        }

        Identity::new(BitSet::from_iter(ids.iter().map(|id| *id as usize)))
    }
}

impl WeightRegistry for ValidatorRegistry {
    fn weight(&self, id: usize) -> Option<usize> {
        // Every existing slot carries a single vote.
        checked_slot(id).map(|_| 1)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_bls::KeyPair as BlsKeyPair;
    use nimiq_collections::BitSet;
    use nimiq_handel::identity::{IdentityRegistry, WeightRegistry};
    use nimiq_keys::{Address, Ed25519PublicKey};
    use nimiq_primitives::{policy::Policy, slots_allocation::ValidatorsBuilder};
    use nimiq_utils::key_rng::SecureGenerate;
    use rand::{rngs::StdRng, SeedableRng};

    use super::ValidatorRegistry;

    /// The test set splits `Policy::SLOTS` evenly, so the whole slot range is allocated.
    const NUM_VALIDATORS: u16 = 4;
    const SLOTS_PER_VALIDATOR: u16 = Policy::SLOTS / NUM_VALIDATORS;

    /// First id above the valid range that truncates back into it when cast to `u16`.
    const WRAPPED: usize = u16::MAX as usize + 1;

    /// Registry covering the entire slot range, plus the voting keys in slot band order.
    fn setup() -> (ValidatorRegistry, Vec<BlsKeyPair>) {
        let mut rng = StdRng::from_rng(&mut rand::rng());
        let mut key_pairs = Vec::new();
        let mut builder = ValidatorsBuilder::new();

        // `ValidatorsBuilder` orders validators by address, so band `i` owns address `[i; 20]`.
        for band in 0..NUM_VALIDATORS {
            let key_pair = BlsKeyPair::generate(&mut rng);
            for _ in 0..SLOTS_PER_VALIDATOR {
                builder.push(
                    Address::from([band as u8; 20]),
                    key_pair.public_key,
                    Ed25519PublicKey::default(),
                );
            }
            key_pairs.push(key_pair);
        }

        (ValidatorRegistry::new(builder.build()), key_pairs)
    }

    /// The slots of the validator owning `band`.
    fn band_slots(band: u16) -> std::ops::Range<usize> {
        let start = band as usize * SLOTS_PER_VALIDATOR as usize;
        start..start + SLOTS_PER_VALIDATOR as usize
    }

    fn bitset(slots: impl IntoIterator<Item = usize>) -> BitSet {
        BitSet::from_iter(slots)
    }

    /// Only slots below `Policy::SLOTS` exist, so only those may carry weight.
    #[test]
    fn weight_accepts_only_slots_below_slot_count() {
        let (registry, _) = setup();
        let slots = Policy::SLOTS as usize;

        assert_eq!(registry.weight(0), Some(1));
        assert_eq!(registry.weight(slots - 1), Some(1));

        assert_eq!(registry.weight(slots), None);
        assert_eq!(registry.weight(slots + 1), None);
        assert_eq!(registry.weight(u16::MAX as usize), None);
    }

    /// Ids congruent to a valid slot modulo 2^16 truncate into range when cast to `u16`. No such
    /// slot exists, so none may carry weight.
    #[test]
    fn weight_rejects_ids_that_truncate_into_slot_range() {
        let (registry, _) = setup();
        let slots = Policy::SLOTS as usize;

        for id in [
            WRAPPED,
            WRAPPED + 1,
            WRAPPED + slots - 1,
            2 * WRAPPED,
            2 * WRAPPED + 100,
        ] {
            assert_eq!(
                registry.weight(id),
                None,
                "phantom slot {id} truncates to {} and must not carry weight",
                id as u16,
            );
        }
    }

    /// The skip block aggregation compares `signers_weight` against `TWO_F_PLUS_ONE`, so a single
    /// non existent slot must make the set unweighable rather than inflate its weight.
    #[test]
    fn signers_weight_rejects_sets_containing_phantom_slots() {
        let (registry, _) = setup();

        let signers = bitset(band_slots(0));
        assert_eq!(
            registry.signers_weight(&signers),
            Some(SLOTS_PER_VALIDATOR as usize),
        );

        for phantom in [Policy::SLOTS as usize, WRAPPED, WRAPPED + 1] {
            let mut polluted = signers.clone();
            polluted.insert(phantom);
            assert_eq!(
                registry.signers_weight(&polluted),
                None,
                "set containing phantom slot {phantom} must not be weighable",
            );
        }
    }

    /// Every existing slot resolves to the voting key of the validator owning it.
    #[test]
    fn public_key_resolves_every_valid_slot() {
        let (registry, key_pairs) = setup();

        for (band, key_pair) in key_pairs.iter().enumerate() {
            for slot in band_slots(band as u16) {
                assert_eq!(
                    registry.public_key(slot),
                    Some(key_pair.public_key),
                    "slot {slot} must resolve to the key of band {band}",
                );
            }
        }
    }

    /// Out of range slots must resolve to `None`, not panic in `Validators::get_band_from_slot`.
    #[test]
    fn public_key_rejects_out_of_range_slots() {
        let (registry, _) = setup();

        for slot in [
            Policy::SLOTS as usize,
            Policy::SLOTS as usize + 1,
            u16::MAX as usize,
        ] {
            assert_eq!(
                registry.public_key(slot),
                None,
                "out of range slot {slot} must not resolve to a key",
            );
        }
    }

    /// Truncating ids must not resolve either: a real key here verifies a forged signer index
    /// against an unrelated validator's key instead of rejecting it.
    #[test]
    fn public_key_rejects_slots_that_truncate_into_range() {
        let (registry, _) = setup();

        for slot in [
            WRAPPED,
            WRAPPED + 1,
            2 * WRAPPED,
            WRAPPED + Policy::SLOTS as usize,
        ] {
            assert_eq!(
                registry.public_key(slot),
                None,
                "phantom slot {slot} truncates to {} and must not resolve to a key",
                slot as u16,
            );
        }
    }

    /// A set of slots exactly covering one validator's band resolves to that validator.
    #[test]
    fn signers_identity_maps_full_slot_bands() {
        let (registry, _) = setup();

        for band in 0..NUM_VALIDATORS {
            let identity = registry.signers_identity(&bitset(band_slots(band)));
            assert_eq!(identity.as_vec(), vec![band]);
        }
    }

    /// Out of range slots belong to no validator, so they yield an empty identity rather than
    /// panicking in `Validators::get_band_from_slot`.
    #[test]
    fn signers_identity_rejects_out_of_range_slots() {
        let (registry, _) = setup();

        for slot in [
            Policy::SLOTS as usize,
            u16::MAX as usize,
            WRAPPED,
            WRAPPED + Policy::SLOTS as usize,
        ] {
            assert!(
                registry.signers_identity(&bitset([slot])).is_empty(),
                "out of range slot {slot} must not resolve to an identity",
            );
        }
    }

    /// The same when mixed into a valid band: the set is malformed as a whole and must not resolve
    /// to the validator owning its valid part.
    #[test]
    fn signers_identity_rejects_valid_band_polluted_with_out_of_range_slot() {
        let (registry, _) = setup();

        for phantom in [Policy::SLOTS as usize, WRAPPED, WRAPPED + 1] {
            let mut signers = bitset(band_slots(0));
            signers.insert(phantom);
            assert!(
                registry.signers_identity(&signers).is_empty(),
                "band polluted with phantom slot {phantom} must not resolve to an identity",
            );
        }
    }
}
