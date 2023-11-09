use std::{
    convert::{TryFrom, TryInto},
    str::FromStr,
};

use nimiq_primitives::coin::Coin;
#[cfg(feature = "serde-derive")]
use nimiq_serde::Deserialize;
use nimiq_test_log::test;

struct NonFailingTest {
    data: &'static str,
    value: u64,
}

impl NonFailingTest {
    pub const fn new(data: &'static str, value: u64) -> NonFailingTest {
        NonFailingTest { data, value }
    }
}

static NON_FAILING_TESTS: [NonFailingTest; 7] = [
    NonFailingTest::new("0000000000000000", 0u64),
    NonFailingTest::new("0000000000000001", 1u64),
    NonFailingTest::new("0000000000000005", 5u64),
    NonFailingTest::new("0000000100000005", 4294967301),
    NonFailingTest::new("000000000001e240", 123456u64),
    NonFailingTest::new("0000001234561234", 78187467316u64),
    NonFailingTest::new("001fffffffffffff", Coin::MAX_SAFE_VALUE),
];

#[test]
#[cfg(feature = "serde-derive")]
fn test_non_failing() {
    for test_case in NON_FAILING_TESTS.iter() {
        let expected: Coin = test_case.value.try_into().unwrap();
        let vec = hex::decode(test_case.data).unwrap();
        let coin: Coin = Deserialize::deserialize_from_vec(&vec[..]).unwrap();
        assert_eq!(expected, coin);
    }
}

#[test]
#[cfg(feature = "serde-derive")]
fn test_deserialize_out_of_bounds() {
    use nimiq_serde::DeserializeError;

    let vec = hex::decode("0020000000000000").unwrap();
    let res: Result<Coin, DeserializeError> = Deserialize::deserialize_from_vec(&vec[..]);
    match res {
        Ok(coin) => panic!("Instead of failing, got {}", coin),
        Err(err) => assert_eq!(err, DeserializeError::serde_custom()),
    }
}

#[test]
fn test_format() {
    let s = format!("{}", Coin::try_from(123456789u64).unwrap());
    assert_eq!("1234.56789", s);
}

#[test]
fn test_format_int_zero() {
    let s = format!("{}", Coin::try_from(56789u64).unwrap());
    assert_eq!("0.56789", s);
}

#[test]
fn test_format_frac_zero() {
    let s = format!("{}", Coin::try_from(123400000u64).unwrap());
    assert_eq!("1234", s);
}

#[test]
fn test_format_zero() {
    let s = format!("{}", Coin::try_from(0u64).unwrap());
    assert_eq!("0", s);
}

#[test]
fn test_parse_valid_one_part() {
    let coin = Coin::from_str("1234").unwrap();
    assert_eq!(Coin::try_from(123400000u64).unwrap(), coin);
}

#[test]
fn test_parse_valid_two_parts() {
    let coin = Coin::from_str("1234.56789").unwrap();
    assert_eq!(Coin::try_from(123456789u64).unwrap(), coin);
}

#[test]
fn test_parse_zero1() {
    let coin = Coin::from_str("0").unwrap();
    assert_eq!(Coin::try_from(0).unwrap(), coin);
}

#[test]
fn test_parse_zero2() {
    let coin = Coin::from_str("0.0").unwrap();
    assert_eq!(Coin::try_from(0).unwrap(), coin);
}

#[test]
fn test_parse_zero3() {
    let coin = Coin::from_str("0000.0000").unwrap();
    assert_eq!(Coin::try_from(0).unwrap(), coin);
}

#[test]
fn test_parse_empty_string() {
    Coin::from_str("").expect_err("Should error");
}

#[test]
fn test_parse_too_many_dots() {
    Coin::from_str("1234.456.789").expect_err("Should error");
}

#[test]
fn test_parse_too_many_frac_digits() {
    Coin::from_str("123.456789").expect_err("Should error");
}

#[test]
fn test_parse_frac_more_zeros() {
    let coin = Coin::from_str("1234.56789000").unwrap();
    assert_eq!(Coin::try_from(123456789).unwrap(), coin)
}

#[test]
fn test_u64_overflow() {
    Coin::try_from(Coin::MAX_SAFE_VALUE).expect("Should work");
    Coin::try_from(Coin::MAX_SAFE_VALUE + 1).expect_err("Should error");
}

#[test]
fn test_int_part_overflow() {
    Coin::from_str("900719925474").expect_err("Should error");
}

// Max safe value in fractional format: 90071992547.40991
#[test]
fn test_frac_part_overflow() {
    Coin::from_str("90071992547.40992").expect_err("Should error");
}

#[test]
fn test_max_value() {
    let coin = Coin::from_str("90071992547.40991").unwrap();
    assert_eq!(Coin::try_from(9007199254740991u64).unwrap(), coin)
}

#[test]
fn test_empty_int_part() {
    Coin::from_str(".56789").expect_err("Should error");
}

#[test]
fn test_empty_frac_part() {
    Coin::from_str("1234.").expect_err("Should error");
}

#[test]
fn test_integrity_frac_digits_and_lunas_per_coin() {
    assert_eq!(10u64.pow(Coin::FRAC_DIGITS), Coin::LUNAS_PER_COIN);
}
