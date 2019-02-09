extern crate fixed_unsigned;

use std::str::FromStr;
use std::string::ToString;

use fixed_unsigned::FixedUnsigned;
use fixed_unsigned::types::{FixedUnsigned4, FixedUnsigned10};



// TODO: Test serialization

#[test]
fn test_parse_zero() {
    assert_eq!(FixedUnsigned4::from_str("0").unwrap(), FixedUnsigned4::from(0u32));
}

#[test]
fn test_to_string() {
    assert_eq!(String::from("1234.0000"), FixedUnsigned4::from(1234u32).to_string());
}

#[test]
fn test_parse() {
    assert_eq!(FixedUnsigned4::from(1234u32), FixedUnsigned4::from_str("1234.0000").unwrap());
}

#[test]
fn test_from_to_string() {
    let f = FixedUnsigned4::from_str("123.4567").expect("Parse failed");
    assert_eq!(f.to_string(), String::from("123.4567"));
}

#[test]
fn test_from_to_string_scale_up() {
    let f = FixedUnsigned4::from_str("123.45").expect("Parse failed");
    assert_eq!(f.to_string(), String::from("123.4500"));
}

/*
  NOTE obsolete, since `from_str` drop digits, no rounding done

#[test]
fn test_from_to_string_scale_up_round_down() {
    let f = FixedUnsigned4::from_str("123.456749").expect("Parse failed");
    assert_eq!(f.to_string(), String::from("123.4567"));
}

#[test]
fn test_from_to_string_scale_up_round_up() {
    let f = FixedUnsigned4::from_str("123.456789").expect("Parse failed");
    assert_eq!(f.to_string(), String::from("123.4568"));
}
*/

#[test]
fn test_add() {
    let a = FixedUnsigned4::from_str("123.4567").unwrap();
    let b = FixedUnsigned4::from_str("135.7910").unwrap();
    let expected = FixedUnsigned4::from_str("259.2477").unwrap();
    assert_eq!(a + b, expected);
}

#[test]
fn test_sub() {
    let a = FixedUnsigned4::from_str("123.4567").unwrap();
    let b = FixedUnsigned4::from_str("135.7910").unwrap();
    let expected = FixedUnsigned4::from_str("12.3343").unwrap();
    assert_eq!(b - a, expected);
}

#[test]
fn test_mul() {
    let a = FixedUnsigned4::from_str("123.4567").unwrap();
    let b = FixedUnsigned4::from_str("135.7910").unwrap();
    let expected = FixedUnsigned4::from_str("16764.3087").unwrap();
    assert_eq!(a * b, expected);
}

#[test]
fn test_div_round_down() {
    let a = FixedUnsigned4::from_str("1.4444").unwrap();
    let b = FixedUnsigned4::from_str("3.1234").unwrap();
    let res = a / b;
    let expected = FixedUnsigned4::from_str("0.4624").unwrap();
    assert_eq!(res, expected);
}

#[test]
fn test_div_round_up() {
    let a = FixedUnsigned4::from_str("123.4567").unwrap();
    let b = FixedUnsigned4::from_str("135.7910").unwrap();
    let expected = FixedUnsigned4::from_str("0.9091").unwrap();
    assert_eq!(a / b, expected);
}

#[test]
fn test_parse_emtpy_string() {
    let res = FixedUnsigned4::from_str("");
    match res {
        Ok(v) => assert!(false, "Expected error, not value: {}", v),
        Err(_) => ()
    }
}

#[test]
fn test_parse_dot() {
    let res = FixedUnsigned4::from_str(".");
    match res {
        Ok(v) => assert!(false, "Expected error, not value: {}", v),
        Err(_) => ()
    }
}

#[test]
fn test_parse_zero_dot() {
    let res = FixedUnsigned4::from_str("0.");
    match res {
        Ok(v) => assert_eq!(v, FixedUnsigned4::from(0u32)),
        Err(_) => assert!(false, "Didn't expect an error")
    }
}

#[test]
fn test_parse_dot_zero() {
    let res = FixedUnsigned4::from_str(".0");
    match res {
        Ok(v) => assert_eq!(v, FixedUnsigned4::from(0u32)),
        Err(_) => assert!(false, "Didn't expect an error")
    }
}

#[test]
fn test_parse_one_dot() {
    let res = FixedUnsigned4::from_str("1.");
    match res {
        Ok(v) => assert_eq!(v, FixedUnsigned4::from(1u32)),
        Err(_) => assert!(false, "Didn't expect an error")
    }
}

#[test]
fn test_parse_dot_one() {
    let res = FixedUnsigned4::from_str(".1");
    match res {
        Ok(v) => assert_eq!(v, FixedUnsigned4::from_str("0.1").unwrap()),
        Err(_) => assert!(false, "Didn't expect an error")
    }
}

#[test]
fn test_bytes_length_matches() {
    let fixed = FixedUnsigned4::from_str("1234.5678").unwrap();
    let bytes = fixed.to_bytes_be();
    assert_eq!(bytes.len(), fixed.bytes());
}

#[test]
fn test_from_f64() {
    let float = 1234.56789f64;
    let fixed = FixedUnsigned4::from(float);
    assert_eq!(fixed, FixedUnsigned::from_str("1234.5678").unwrap());
}

#[test]
fn test_mul_borrow() {
    let res = &FixedUnsigned4::from_str("12.34").unwrap() * &FixedUnsigned4::from_str("56.78").unwrap();
    assert_eq!(res, FixedUnsigned4::from_str("700.6652").unwrap());
}

#[test]
fn test_div_borrow() {
    let res = &FixedUnsigned4::from_str("12.34").unwrap() / &FixedUnsigned4::from_str("56.78").unwrap();
    assert_eq!(res, FixedUnsigned4::from_str("0.2173").unwrap());
}
