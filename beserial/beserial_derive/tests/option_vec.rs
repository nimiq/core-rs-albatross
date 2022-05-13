use beserial::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct TestStruct {
    #[beserial(len_type(u16))]
    test: Option<Vec<u8>>,
}

#[test]
fn it_correctly_serializes_and_deserializes_option_of_vec() {
    fn reserialize(s: &TestStruct) -> TestStruct {
        let mut v = Vec::with_capacity(s.serialized_size());
        Serialize::serialize(&s, &mut v).unwrap();
        let s2: TestStruct = Deserialize::deserialize(&mut &v[..]).unwrap();
        s2
    }
    let mut test = TestStruct { test: None };
    assert_eq!(test.serialized_size(), 1);
    assert_eq!(reserialize(&test), test);
    test = TestStruct {
        test: Some(Vec::new()),
    };
    assert_eq!(test.serialized_size(), 3);
    assert_eq!(reserialize(&test), test);
    test = TestStruct {
        test: Some(vec![1, 2, 3]),
    };
    assert_eq!(test.serialized_size(), 6);
    assert_eq!(reserialize(&test), test);
}
