use nimiq_block::MultiSignature;
use nimiq_bls::{AggregateSignature, KeyPair};
use nimiq_collections::BitSet;
use nimiq_handel::{
    contribution::{AggregatableContribution, ContributionError},
    update::LevelUpdate,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_validator::aggregation::update::SerializableLevelUpdate;

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
    assert!(update_2.individual.is_none());
    assert_eq!(update_2.level, 2);
}

#[test]
fn test_serialize_deserialize_with_message() {
    let update: SerializableLevelUpdate<WrappedMultiSignature> =
        LevelUpdate::new(create_multisig(), None, 2, 3).into();
    assert_eq!(update.serialized_size(), 99);
}
