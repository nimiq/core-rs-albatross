use beserial::{Deserialize, Serialize};
use hex;
use nimiq_collections::bitset::BitSet;

fn sample_bitset() -> BitSet {
    let mut set = BitSet::new();
    // Write 0xFEFF07
    set.remove(0);
    for i in 1..19 {
        set.insert(i);
    }
    // Set bit 70
    set.insert(70);
    return set;
}

#[test]
fn it_can_correctly_serialize() {
    let set = sample_bitset();
    let bin = set.serialize_to_vec();
    assert_eq!(
        hex::decode("02000000000007FFFE0000000000000040").unwrap(),
        bin
    );
}

#[test]
fn it_can_correctly_deserialize() {
    let bin = hex::decode("02000000000007FFFE0000000000000040").unwrap();
    let set = BitSet::deserialize_from_vec(&bin).unwrap();
    assert_eq!(19, set.len());
    assert_eq!(sample_bitset(), set);
}

#[test]
fn it_correctly_computes_union() {
    let set1 = sample_bitset();
    let mut set2 = BitSet::new();
    set2.insert(69);
    set2.insert(70);
    let set3 = set1 & set2;
    assert_eq!(set3.len(), 1);
    {
        let mut correct = BitSet::new();
        correct.insert(70);
        assert_eq!(correct, set3);
    }
}

#[test]
fn it_correctly_computes_intersection() {
    let set1 = sample_bitset();
    let mut set2 = BitSet::new();
    set2.insert(69);
    set2.insert(70);
    let set3 = set1 | set2;
    assert_eq!(set3.len(), 20);
}
