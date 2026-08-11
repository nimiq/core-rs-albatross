use std::collections::BTreeMap;

use nimiq_block::MultiSignature;
use nimiq_bls::{AggregateSignature, KeyPair};
use nimiq_collections::BitSet;
use nimiq_handel::{
    contribution::{AggregatableContribution, ContributionError},
    update::LevelUpdate,
};
use nimiq_hash::Blake2sHash;
use nimiq_primitives::policy::Policy;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_validator::aggregation::{
    tendermint::contribution::TendermintContribution, update::SerializableLevelUpdate,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WrappedMultiSignature(pub MultiSignature);

impl AggregatableContribution for WrappedMultiSignature {
    fn contributors(&self) -> BitSet {
        self.0.contributors()
    }

    fn combine(&mut self, other_contribution: &Self) -> Result<(), ContributionError> {
        self.0
            .combine(&other_contribution.0)
            .map_err(ContributionError::Overlapping)
    }
}

fn create_multisig() -> WrappedMultiSignature {
    let raw_key = hex::decode(
        "1b9e470e0deb06fe55774bb2cf499b411f55265c10d8d78742078381803451e058c88\
        391431799462edde4c7872649964137d8e03cd618dd4a25690c56ffd7f42fb7ae8049d29f38d569598b38d4\
        39f69107cc0b6f4ecd00a250c74409510100",
    )
    .unwrap();
    let key_pair = KeyPair::deserialize_from_vec(&raw_key).unwrap();
    let signature = key_pair.sign(&"foobar");
    let signature = AggregateSignature::from_signatures(&[signature]);
    let mut signers = BitSet::default();
    signers.insert(1);
    WrappedMultiSignature(MultiSignature { signature, signers })
}

#[test]
fn test_serialize_deserialize_level_update() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> =
        LevelUpdate::new(create_multisig(), None, 2, 3).into();
    let data = update.serialize_to_vec();
    let update_2: SerializableLevelUpdate<WrappedMultiSignature> =
        Deserialize::deserialize_from_vec(&data).unwrap();

    assert_eq!(data.len(), update.serialized_size());
    assert_eq!(update.serialized_size(), 99);

    let update_2 = update_2.into_level_update(2);
    assert!(update_2.individual.is_none());
    assert_eq!(update_2.level, 2);
}

#[test]
fn test_serialize_deserialize_with_message() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> =
        LevelUpdate::new(create_multisig(), None, 2, 3).into();
    assert_eq!(update.serialized_size(), 99);
}

// The tests below cover the `Checked` bound on `SerializableLevelUpdate`. It is the only bounds
// check between a peer supplied bitset and `ValidatorRegistry`, which indexes slots into the
// validator set without re-checking them.
//
// The payloads are built as a `LevelUpdate` and serialized: `Checked` only runs on deserialization
// and `From<LevelUpdate>` does not inspect its input, so this is what a peer can put on the wire.

/// Contribution whose contributor bitset is exactly `signers`. The signature is irrelevant,
/// `Checked` runs before any signature is looked at.
fn multisig_with_signers(signers: impl IntoIterator<Item = usize>) -> WrappedMultiSignature {
    let mut multisig = create_multisig();
    multisig.0.signers = BitSet::from_iter(signers);
    multisig
}

fn roundtrip(
    update: SerializableLevelUpdate<WrappedMultiSignature>,
) -> Result<SerializableLevelUpdate<WrappedMultiSignature>, nimiq_serde::DeserializeError> {
    Deserialize::deserialize_from_vec(&update.serialize_to_vec())
}

/// Control: the highest existing slot is accepted, so the rejections below are not an off by one.
#[test]
fn accepts_highest_valid_slot() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> = LevelUpdate::new(
        multisig_with_signers([Policy::SLOTS as usize - 1]),
        None,
        2,
        3,
    )
    .into();

    assert!(roundtrip(update).is_ok());
}

/// The first non existent slot. Unchecked it reaches `get_band_from_slot` untruncated and trips
/// its assertion, crashing the node.
#[test]
fn rejects_aggregate_with_slot_at_slot_count() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> =
        LevelUpdate::new(multisig_with_signers([Policy::SLOTS as usize]), None, 2, 3).into();

    assert!(roundtrip(update).is_err());
}

/// Slots at or above 2^16 truncate back into range when cast to `u16`, which is what makes
/// `WeightRegistry::weight` hand out weight for slots that do not exist.
#[test]
fn rejects_aggregate_with_slots_beyond_u16() {
    let wrapped = u16::MAX as usize + 1;

    for slot in [
        wrapped,
        wrapped + 1,
        2 * wrapped,
        wrapped + Policy::SLOTS as usize,
    ] {
        let update: SerializableLevelUpdate<WrappedMultiSignature> =
            LevelUpdate::new(multisig_with_signers([slot]), None, 2, 3).into();

        assert!(
            roundtrip(update).is_err(),
            "slot {slot} truncates to {} and must be rejected",
            slot as u16,
        );
    }
}

/// `WeightedVote::verify` feeds both fields to the registry, so a valid aggregate does not excuse
/// an out of range individual contribution.
#[test]
fn rejects_individual_with_out_of_range_slot() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> = LevelUpdate::new(
        multisig_with_signers([0]),
        Some(multisig_with_signers([Policy::SLOTS as usize])),
        2,
        3,
    )
    .into();

    assert!(roundtrip(update).is_err());
}

/// The registry walks every bit, so a bad slot hidden among valid ones must reject the whole
/// update rather than be silently dropped.
#[test]
fn rejects_aggregate_mixing_valid_and_out_of_range_slots() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> = LevelUpdate::new(
        multisig_with_signers([0, 1, 2, Policy::SLOTS as usize, u16::MAX as usize + 1]),
        None,
        2,
        3,
    )
    .into();

    assert!(roundtrip(update).is_err());
}

fn tendermint_contribution(
    buckets: impl IntoIterator<Item = (Option<Blake2sHash>, Vec<usize>)>,
) -> TendermintContribution {
    let signature = create_multisig().0.signature;
    let contributions = buckets
        .into_iter()
        .map(|(hash, signers)| {
            (
                hash,
                MultiSignature::new(signature, BitSet::from_iter(signers)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    TendermintContribution { contributions }
}

/// `TendermintContribution` splits its signers into one bucket per proposal hash. The check must
/// see every bucket, otherwise a bad slot can hide behind a well formed leading one.
#[test]
fn rejects_tendermint_contribution_with_out_of_range_slot_in_any_bucket() {
    let proposal = Some(Blake2sHash::from([1u8; 32]));
    let out_of_range = vec![Policy::SLOTS as usize];
    let valid = vec![0];

    // `None` sorts before `Some`, so these put the offending bucket first and last respectively.
    let cases = [
        vec![
            (None, out_of_range.clone()),
            (proposal.clone(), valid.clone()),
        ],
        vec![
            (None, valid.clone()),
            (proposal.clone(), out_of_range.clone()),
        ],
    ];

    for buckets in cases {
        let update: SerializableLevelUpdate<TendermintContribution> =
            LevelUpdate::new(tendermint_contribution(buckets), None, 2, 3).into();
        let data = update.serialize_to_vec();

        assert!(
            SerializableLevelUpdate::<TendermintContribution>::deserialize_from_vec(&data).is_err(),
        );
    }

    // Control: the same shape with only valid slots.
    let update: SerializableLevelUpdate<TendermintContribution> = LevelUpdate::new(
        tendermint_contribution([(None, valid), (proposal, vec![1])]),
        None,
        2,
        3,
    )
    .into();
    let data = update.serialize_to_vec();

    assert!(SerializableLevelUpdate::<TendermintContribution>::deserialize_from_vec(&data).is_ok());
}
