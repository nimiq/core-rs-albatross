use std::fmt::Debug;

use beserial::{Deserialize, Serialize};

use crate::multisig::{IndividualSignature, MultiSignature};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelUpdate {
    /// The updated multi-signature for this level
    pub(crate) multisig: MultiSignature,

    /// The individual signature of the sender, or `None`
    pub(crate) individual: Option<IndividualSignature>,

    /// The level to which this multi-signature belongs to
    pub(crate) level: u8,

    /// The validator ID of the sender (a.k.a. `pk_idx`)
    ///
    /// NOTE: It's save to just send your own validator ID, since everything critical is authenticated
    /// by signatures anyway.
    pub(crate) origin: u16,
}

impl LevelUpdate {
    pub fn new(
        multisig: MultiSignature,
        individual: Option<IndividualSignature>,
        level: usize,
        origin: usize,
    ) -> Self {
        Self {
            multisig,
            individual,
            level: level as u8,
            origin: origin as u16,
        }
    }

    pub fn with_tag<T: Clone + Debug + Serialize + Deserialize>(
        self,
        tag: T,
    ) -> LevelUpdateMessage<T> {
        LevelUpdateMessage { update: self, tag }
    }

    pub fn origin(&self) -> usize {
        self.origin as usize
    }

    pub fn level(&self) -> usize {
        self.level as usize
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelUpdateMessage<T: Clone + Debug + Serialize + Deserialize> {
    /// The update for that level
    pub update: LevelUpdate,

    /// The message this aggregation is running over. This is needed to differentiate to which
    /// aggregation this belongs to.
    pub tag: T,
}

#[cfg(test)]
mod test {
    use beserial::{Deserialize, Serialize};
    use bls;

    use crate::multisig::{IndividualSignature, MultiSignature};

    use super::*;

    fn create_multisig() -> MultiSignature {
        let raw_key = hex::decode(
            "1b9e470e0deb06fe55774bb2cf499b411f55265c10d8d78742078381803451e058c88\
            391431799462edde4c7872649964137d8e03cd618dd4a25690c56ffd7f42fb7ae8049d29f38d569598b38d4\
            39f69107cc0b6f4ecd00a250c74409510100",
        )
        .unwrap();
        let key_pair = bls::KeyPair::deserialize_from_vec(&raw_key).unwrap();
        let signature = key_pair.sign(&"foobar");
        IndividualSignature::new(signature, 1).as_multisig()
    }

    #[test]
    fn test_serialize_deserialize_level_update() {
        let update = LevelUpdate::new(create_multisig(), None, 2, 3);
        let data = update.serialize_to_vec();
        let update_2: LevelUpdate = Deserialize::deserialize_from_vec(&data).unwrap();

        assert_eq!(data.len(), update.serialized_size());
        assert_eq!(update.serialized_size(), 109);
        assert!(update_2.individual.is_none());
        assert_eq!(update_2.level, 2);
        assert_eq!(update_2.origin, 3);
    }

    #[test]
    fn test_serialize_deserialize_with_message() {
        let update = LevelUpdate::new(create_multisig(), None, 2, 3).with_tag(42u64);
        assert_eq!(update.serialized_size(), 109 + 8);
    }
}
