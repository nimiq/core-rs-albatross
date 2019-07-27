use std::iter::empty;
use beserial::{Deserialize, Serialize};
use nimiq_collections::compressed_list::CompressedList;

const SAMPLE_LIST: &str = "008000030102030200000000000000010000000000000300";

#[test]
fn it_can_collect_and_serialize() {
    let mut vec = Vec::<u8>::with_capacity(128);
    for _ in 0..72 {
        vec.push(1);
    }
    vec.push(2);
    for _ in 73..128 {
        vec.push(3);
    }
    let list: CompressedList<u8> = vec.iter().cloned().collect();
    assert!(list.verify());
    assert_eq!(list.len(), 128);
    let bin = list.serialize_to_vec();
    assert_eq!(SAMPLE_LIST, hex::encode(bin));
}

#[test]
fn it_can_deserialize_and_iterate() {
    let list = CompressedList::<u8>::deserialize_from_vec(&hex::decode(SAMPLE_LIST).unwrap()).unwrap();
    assert!(list.verify());
    assert_eq!(list.len(), 128);
    let mut iter = list.into_iter();
    for _ in 0..72 {
        assert_eq!(iter.next(), Some(&1));
    }
    assert_eq!(iter.next(), Some(&2));
    for _ in 73..128 {
        assert_eq!(iter.next(), Some(&3));
    }
    assert_eq!(iter.next(), None);
}

#[test]
fn it_can_be_empty() {
    const EMPTY_LIST: &str = "0000000000";

    let list: CompressedList<u8> = empty().collect();
    assert!(list.verify());
    assert_eq!(list.len(), 0);
    assert_eq!(EMPTY_LIST, hex::encode(list.serialize_to_vec()));

    let list = CompressedList::<u8>::deserialize_from_vec(&hex::decode(EMPTY_LIST).unwrap()).unwrap();
    let mut iter = list.into_iter();
    assert_eq!(iter.next(), None);
}

#[test]
fn it_rejects_bad_lists() {
    // first bit zero but not empty
    const UNDEFINED_START_LIST: &str = "0080000201020200000000000000000000000000000300";
    // not enough distinct items
    const UNDERFLOW_ITEMS_LIST: &str = "0080000201020200000000000000010000000000000300";
    // too many distinct items
    const OVERLONG_ITEMS_LIST:  &str = "00800004010203050200000000000000010000000000000300";

    assert_list_bad(UNDEFINED_START_LIST);
    assert_list_bad(UNDERFLOW_ITEMS_LIST);
    assert_list_bad(OVERLONG_ITEMS_LIST);
}

fn assert_list_bad(hexdump: &str) {
    let list = CompressedList::<u8>::deserialize_from_vec(&hex::decode(hexdump).unwrap()).unwrap();
    assert!(!list.verify());
}
