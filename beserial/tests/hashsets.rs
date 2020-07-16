use beserial::{Deserialize, DeserializeWithLength, Serialize, SerializeWithLength};
use std::collections::HashSet;

#[test]
fn it_correctly_serializes_and_deserializes_hashsets() {
    fn reserialize<T>(s: HashSet<T>) -> HashSet<T>
    where
        T: Serialize + Deserialize + std::cmp::Eq + std::hash::Hash + std::cmp::Ord,
    {
        let mut v = Vec::with_capacity(64);
        SerializeWithLength::serialize::<u16, Vec<u8>>(&s, &mut v).unwrap();
        let s2: HashSet<T> = DeserializeWithLength::deserialize::<u16, &[u8]>(&mut &v[..]).unwrap();
        return s2;
    }

    let mut hashset = HashSet::new();
    hashset.insert(18446744073709551615u64);
    hashset.insert(9223372036854775807);
    hashset.insert(381073568934759237);
    hashset.insert(2131907967678687);
    hashset.insert(255);
    hashset.insert(16);

    let reference_hashset = hashset.clone();
    let processed_hashset = reserialize(hashset);

    assert_eq!(reference_hashset, processed_hashset);

    assert!(processed_hashset.contains(&18446744073709551615));
    assert!(processed_hashset.contains(&9223372036854775807));
    assert!(processed_hashset.contains(&381073568934759237));
    assert!(processed_hashset.contains(&2131907967678687));
    assert!(processed_hashset.contains(&255));
    assert!(processed_hashset.contains(&16));

    assert!(!processed_hashset.contains(&0));
}
