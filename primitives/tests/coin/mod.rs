use beserial::{Serialize, Deserialize, SerializingError};
use primitives::coin::{Coin, CoinParseError};
use std::str::FromStr;


struct NonFailingTest {
    data: &'static str,
    coin: Coin
}

impl NonFailingTest {
    pub fn new(data: &'static str, value: u64) -> NonFailingTest {
        NonFailingTest { data, coin: Coin::from_u64(value).unwrap() }
    }
}

lazy_static! {
    static ref NON_FAILING_TESTS: Vec<NonFailingTest> = vec![
        NonFailingTest::new("0000000000000000", 0u64),
        NonFailingTest::new("0000000000000001", 1u64),
        NonFailingTest::new("0000000000000005", 5u64),
        NonFailingTest::new("0000000100000005", 4294967301),
        NonFailingTest::new("000000000001e240", 123456u64),
        NonFailingTest::new("0000001234561234", 78187467316u64),
        NonFailingTest::new("001fffffffffffff", Coin::MAX_SAFE_VALUE),
    ];
}


#[test]
fn test_non_failing() {
    for test_case in NON_FAILING_TESTS.iter() {
        let vec = hex::decode(test_case.data).unwrap();
        let coin: Coin = Deserialize::deserialize(&mut &vec[..]).unwrap();
        assert_eq!(test_case.coin, coin);
    }
}

#[test]
fn test_serialize_out_of_bounds() {
    let vec = hex::decode("0020000000000000").unwrap();
    let res:Result<Coin, SerializingError> = Deserialize::deserialize(&mut &vec[..]);
    match res {
        Ok(coin) => assert!(false, "Instead of failing, got {}", coin),
        Err(err) => {
            match err {
                SerializingError::IoError(_, _) => (),
                _ => assert!(false, "Expected to fail with IoError, but got {}", err)
            }
        }
    }
}

#[test]
fn test_deserialize_out_of_bounds() {
    let mut vec = Vec::with_capacity(8);
    let res = Serialize::serialize(&Coin::from_u64_unchecked(9007199254740992), &mut vec);
    match res {
        Ok(_) => assert!(false, "Didn't fail"),
        Err(err) => {
            match err {
                SerializingError::Overflow => (),
                _ => assert!(false, "Expected to fail with Overflow, but got {}", err)
            }
        }
    }
}

#[test]
fn test_format() {
    let s = format!("{}", Coin::from_u64(123456789u64).unwrap());
    assert_eq!("1234.56789", s);
}

#[test]
fn test_format_int_zero() {
    let s = format!("{}", Coin::from_u64(56789u64).unwrap());
    assert_eq!("0.56789", s);
}

#[test]
fn test_format_frac_zero() {
    let s = format!("{}", Coin::from_u64(123400000u64).unwrap());
    assert_eq!("1234", s);
}

#[test]
fn test_format_zero() {
    let s = format!("{}", Coin::from_u64(0u64).unwrap());
    assert_eq!("0", s);
}




#[test]
fn test_parse_valid_one_part() {
    let coin = Coin::from_str("1234").unwrap();
    assert_eq!(Coin::from_u64(123400000).unwrap(), coin);
}

#[test]
fn test_parse_valid_two_parts() {
    let coin = Coin::from_str("1234.56789").unwrap();
    assert_eq!(Coin::from_u64(123456789).unwrap(), coin);
}

#[test]
fn test_parse_zero1() {
    let coin = Coin::from_str("0").unwrap();
    assert_eq!(Coin::from_u64(0).unwrap(), coin);
}

#[test]
fn test_parse_zero2() {
    let coin = Coin::from_str("0.0").unwrap();
    assert_eq!(Coin::from_u64(0).unwrap(), coin);
}

#[test]
fn test_parse_zero3() {
    let coin = Coin::from_str("0000.0000").unwrap();
    assert_eq!(Coin::from_u64(0).unwrap(), coin);
}

#[test]
fn test_parse_empty_string() {
    let coin = Coin::from_str("");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => match e {
            CoinParseError::InvalidString => (),
            _ => assert!(false, "Expected CoinParseError::InvalidString")
        }
    }
}

#[test]
fn test_parse_too_many_dots() {
    let coin = Coin::from_str("1234.456.789");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => match e {
            CoinParseError::InvalidString => (),
            _ => assert!(false, "Expected CoinParseError::InvalidString")
        }
    }
}

#[test]
fn test_parse_too_many_frac_digits() {
    let coin = Coin::from_str("123.456789");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => match e {
            CoinParseError::TooManyFractionalDigits => (),
            _ => assert!(false, "Expected CoinParseError::TooManyFractionalDigits")
        }
    }
}

#[test]
// NOTE: This is open for discussion, if `1234.56789000` is a value amount of NIM
// I don't think so - Janosch
fn test_parse_frac_more_zeros() {
    let coin = Coin::from_str("1234.56789000");
    match &coin {
        Err(e) => { println!("{:#}", e); },
        _ => ()
    };

    assert!(coin.is_err());
}

#[test]
fn test_u64_overflow() {
    let coin = Coin::from_u64(Coin::MAX_SAFE_VALUE);
    match coin {
        Ok(_) => (),
        Err(_) => assert!(false, "Did not expect an error here"),
    }

    let coin = Coin::from_u64(Coin::MAX_SAFE_VALUE + 1);
    match coin {
        Err(CoinParseError::Overflow) => (),
        _ => assert!(false, "Did not expect an error here"),
    }
}

#[test]
fn test_int_part_overflow() {
    let coin = Coin::from_str("900719925474");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => match e {
            CoinParseError::Overflow => (),
            _ => assert!(false, "Expected CoinParseError::Overflow")
        }
    }
}


// Max safe value in fractional format: 90071992547.40991
#[test]
fn test_frac_part_overflow() {
    let coin = Coin::from_str("90071992547.40992");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => {
            match e {
                CoinParseError::Overflow => (),
                _ => assert!(false, "Expected CoinParseError::Overflow")
            }
        }
    }
}

#[test]
fn test_max_value() {
    let coin = Coin::from_str("90071992547.40991").unwrap();
    assert_eq!(Coin::from_u64(9007199254740991u64).unwrap(), coin)
}

#[test]
fn test_empty_int_part() {
    let coin = Coin::from_str(".56789");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => match e {
            CoinParseError::InvalidString => (),
            _ => assert!(false, "Expected CoinParseError::InvalidString")
        }
    }
}

#[test]
fn test_empty_frac_part() {
    let coin = Coin::from_str("1234.");
    match coin {
        Ok(_) => assert!(false, "Expected error"),
        Err(e) => match e {
            CoinParseError::InvalidString => (),
            _ => assert!(false, "Expected CoinParseError::InvalidString")
        }
    }
}

#[test]
fn test_integrity_frac_digits_and_lunas_per_coin() {
    assert_eq!(10u64.pow(Coin::FRAC_DIGITS), Coin::LUNAS_PER_COIN);
}
