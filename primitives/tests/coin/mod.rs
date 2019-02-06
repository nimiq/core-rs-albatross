extern crate hex;

use beserial::{Serialize, Deserialize, SerializingError};
use primitives::coin::Coin;


struct NonFailingTest {
    data: &'static str,
    coin: Coin
}

impl NonFailingTest {
    pub fn new(data: &'static str, value: u64) -> NonFailingTest {
        NonFailingTest { data, coin: Coin::from(value) }
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
                SerializingError::IoError => (),
                _ => assert!(false, "Expected to fail with IoError, but got {}", err)
            }
        }
    }
}

#[test]
fn test_deserialize_out_of_bounds() {
    let mut vec = Vec::with_capacity(8);
    let res = Serialize::serialize(&Coin::from(9007199254740992), &mut vec);
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