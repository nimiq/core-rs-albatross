#![feature(try_from)]
extern crate beserial;
#[macro_use]
extern crate beserial_derive;

use beserial::{Deserialize, Serialize, uvar};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
enum TestU8 {
    A = 1,
    B,
    C = 5,
    D,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u64)]
enum TestU64 {
    A = 1,
    B,
    C = 9223372036854775807,
    D,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u64)]
#[beserial(uvar)]
enum TestUVar {
    A = 1,
    B = 2164392960,
    C = 9223372036854775807,
    D,
}

#[test]
fn it_can_handle_value_enums_with_repr_u8() {
    fn reserialize(test: TestU8) -> TestU8 {
        let v = Serialize::serialize_to_vec(&test);
        return Deserialize::deserialize(&mut &v[..]).unwrap();
    }
    assert_eq!(reserialize(TestU8::A), TestU8::A);
    assert_eq!(reserialize(TestU8::B), TestU8::B);
    assert_eq!(reserialize(TestU8::C), TestU8::C);
    assert_eq!(reserialize(TestU8::D), TestU8::D);
    assert_eq!(Serialize::serialize_to_vec(&TestU8::A)[0], 1);
    assert_eq!(Serialize::serialize_to_vec(&TestU8::B)[0], 2);
    assert_eq!(Serialize::serialize_to_vec(&TestU8::C)[0], 5);
    assert_eq!(Serialize::serialize_to_vec(&TestU8::D)[0], 6);
}

#[test]
fn it_can_handle_value_enums_with_repr_u64() {
    fn reserialize(test: TestU64) -> TestU64 {
        let v = Serialize::serialize_to_vec(&test);
        return Deserialize::deserialize(&mut &v[..]).unwrap();
    }
    fn reserialize_to_num(test: TestU64) -> u64 {
        let v = Serialize::serialize_to_vec(&test);
        return Deserialize::deserialize(&mut &v[..]).unwrap();
    }
    assert_eq!(reserialize(TestU64::A), TestU64::A);
    assert_eq!(reserialize(TestU64::B), TestU64::B);
    assert_eq!(reserialize(TestU64::C), TestU64::C);
    assert_eq!(reserialize(TestU64::D), TestU64::D);
    assert_eq!(reserialize_to_num(TestU64::A), 1);
    assert_eq!(reserialize_to_num(TestU64::B), 2);
    assert_eq!(reserialize_to_num(TestU64::C), 9223372036854775807);
    assert_eq!(reserialize_to_num(TestU64::D), 9223372036854775808);
}

#[test]
fn it_can_handle_value_enums_with_repr_uvar() {
    fn reserialize(test: TestUVar) -> TestUVar {
        let v = Serialize::serialize_to_vec(&test);
        return Deserialize::deserialize(&mut &v[..]).unwrap();
    }
    fn reserialize_to_num(test: TestUVar) -> uvar {
        let v = Serialize::serialize_to_vec(&test);
        return Deserialize::deserialize(&mut &v[..]).unwrap();
    }
    assert_eq!(reserialize(TestUVar::A), TestUVar::A);
    assert_eq!(reserialize(TestUVar::B), TestUVar::B);
    assert_eq!(reserialize(TestUVar::C), TestUVar::C);
    assert_eq!(reserialize(TestUVar::D), TestUVar::D);
    assert_eq!(reserialize_to_num(TestUVar::A), 1.into());
    assert_eq!(reserialize_to_num(TestUVar::B), 2164392960.into());
    assert_eq!(reserialize_to_num(TestUVar::C), 9223372036854775807.into());
    assert_eq!(reserialize_to_num(TestUVar::D), 9223372036854775808.into());
}

