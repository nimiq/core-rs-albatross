use beserial::{Deserialize, Serialize};
use nimiq_collections::grouped_list::{Group, GroupedList};

const SAMPLE_LIST: &str = "0003004801000102003703";

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
    let list: GroupedList<u8> = vec.iter().cloned().collect();
    assert_eq!(list.len(), 128);
    assert_eq!(list.num_groups(), 3);
    assert_eq!(list.is_empty(), false);
    let bin = list.serialize_to_vec();
    assert_eq!(SAMPLE_LIST, hex::encode(bin));
}

#[test]
fn it_can_deserialize_and_iterate() {
    let list = GroupedList::<u8>::deserialize_from_vec(&hex::decode(SAMPLE_LIST).unwrap()).unwrap();
    assert_eq!(list.len(), 128);
    assert_eq!(list.num_groups(), 3);
    assert_eq!(list.is_empty(), false);

    let mut iter = list.iter();
    for _ in 0..72 {
        assert_eq!(iter.next(), Some(1));
    }
    assert_eq!(iter.next(), Some(2));
    for _ in 73..128 {
        assert_eq!(iter.next(), Some(3));
    }
    assert_eq!(iter.next(), None);

    let mut iter_groups = list.groups().iter();
    assert_eq!(iter_groups.next(), Some(&Group(72, 1)));
    assert_eq!(iter_groups.next(), Some(&Group(1, 2)));
    assert_eq!(iter_groups.next(), Some(&Group(55, 3)));
    assert_eq!(iter_groups.next(), None);
}
