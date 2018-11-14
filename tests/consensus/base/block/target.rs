use num_traits::pow;
use num_bigint::BigInt;
use bigdecimal::BigDecimal;
use nimiq::consensus::base::block::*;
use nimiq::consensus::policy;

#[test]
fn it_correctly_calculates_target_from_compact() {
    assert_eq!(Target::from(TargetCompact::from(0x1f010000)), [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into());
    assert_eq!(Target::from(TargetCompact::from(0x1e010000)), [0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into());
    assert_eq!(Target::from(TargetCompact::from(0x1f000100)), [0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into());
    assert_eq!(Target::from(TargetCompact::from(0x01000001)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1].into());
    assert_eq!(Target::from(TargetCompact::from(0x0200ffff)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff].into());
    assert_eq!(Target::from(TargetCompact::from(0x037fffff)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x7f, 0xff, 0xff].into());
    assert_eq!(Target::from(TargetCompact::from(0x0380ffff)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0xff].into());
    assert_eq!(Target::from(TargetCompact::from(0x040080ff)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0].into());
}

#[test]
fn it_correctly_calculates_compact_from_target() {
    assert_eq!(TargetCompact::from(Target::from([0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])), 0x1f010000.into());
    assert_eq!(TargetCompact::from(Target::from([0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])), 0x1f008000.into());
    assert_eq!(TargetCompact::from(Target::from([0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])), 0x1e010000.into());
    assert_eq!(TargetCompact::from(Target::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])), 0x01000001.into());
    assert_eq!(TargetCompact::from(Target::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff])), 0x0200ffff.into());
    assert_eq!(TargetCompact::from(Target::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x7f, 0xff, 0xff])), 0x037fffff.into());
    assert_eq!(TargetCompact::from(Target::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0xff])), 0x040080ff.into());
}

#[test]
fn it_correctly_converts_from_big_decimal_to_target() {
    assert_eq!(Target::from(BigDecimal::new(pow(BigInt::from(2), 240), 0)), [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into());
    assert_eq!(Target::from(BigDecimal::from(1)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1].into());
    assert_eq!(Target::from(BigDecimal::from(65535.923382)), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff].into());
}

#[test]
fn it_correctly_converts_from_target_to_bigdecimal() {
    assert_eq!(BigDecimal::new(pow(BigInt::from(2), 240), 0), Target::from([0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).into());
    assert_eq!(BigDecimal::from(1), Target::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]).into());
    assert_eq!(BigDecimal::from(65535), Target::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]).into());
}

#[test]
fn it_correctly_calculates_target_from_difficulty() {
    assert_eq!(Target::from(Difficulty::from(1)), [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into());
    assert_eq!(Target::from(Difficulty::from(256)), [0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].into());
    assert_eq!(Target::from(Difficulty::from(policy::BLOCK_TARGET_MAX.clone())), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1].into());
}

#[test]
fn it_correctly_calculates_compact_from_difficulty() {
    assert_eq!(TargetCompact::from(Difficulty::from(1)), 0x1f010000.into());
    assert_eq!(TargetCompact::from(Difficulty::from(250)), 0x1e010624.into());
    assert_eq!(TargetCompact::from(Difficulty::from(256)), 0x1e010000.into());
    assert_eq!(TargetCompact::from(Difficulty::from(BigDecimal::new(pow(BigInt::from(2), 32) - 1, 0))), 0x1b010000.into());
    assert_eq!(TargetCompact::from(Difficulty::from(BigDecimal::new(pow(BigInt::from(2), 53) - 1, 0))), 0x18080000.into());
    assert_eq!(TargetCompact::from(Difficulty::from(policy::BLOCK_TARGET_MAX.clone())), 0x01000001.into());
}

#[test]
fn it_correctly_calculates_target_depth() {
    assert_eq!(Target::from(TargetCompact::from(0x1f010000)).get_depth(), 0);
    assert_eq!(Target::from(TargetCompact::from(0x1f008f00)).get_depth(), 0);
    assert_eq!(Target::from(TargetCompact::from(0x1e800000)).get_depth(), 1);
    assert_eq!(Target::from(TargetCompact::from(0x1e600000)).get_depth(), 1);
    assert_eq!(Target::from(TargetCompact::from(0x1e400000)).get_depth(), 2);
    assert_eq!(Target::from(TargetCompact::from(0x01000002)).get_depth(), 239);
    assert_eq!(Target::from(TargetCompact::from(0x01000001)).get_depth(), 240);
}

