fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_account::StakingContract;
        use nimiq_serde::{Deserialize, Serialize};
        let res = StakingContract::deserialize_from_vec(data); // err I think is of type DeserializeError
                                                               // Now check if contract exists. If it does (aka. the original data was a valid staking contract) then try to serialize it back to a vector, then check if the original vector and the new vector are the same, if they aren't then there is a bug in the parsing logic.
                                                               // The existence of error implies that contract does not exist.
        match res {
            Ok(v) => {
                let serialized = StakingContract::serialize_to_vec(&v);
                assert!((serialized.len() <= data.len()), "The size of the serialized version was bigger than the original vector! This shouldn't happen!");
                let original_data_segment: &[u8] = data[..(serialized.len())].try_into().unwrap(); // This ugly stuff has to be done, because the serialization function ignores extra bytes at the end so we can not compare the byte vectors by themselves. Yuck!!!
                assert_eq!(original_data_segment, serialized);
            }
            Err(e) => {
                return;
            }
        }
    })
}
