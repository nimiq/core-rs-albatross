use nimiq_blockchain::SuperBlockCounts;


fn assert_range(sbc: &SuperBlockCounts, left: u8, right: u8, expected: u64) {
    for i in left..=right {
        assert_eq!(sbc.get(i as u8), expected);
    }
}

#[test]
pub fn test_zero() {
    assert_range(&SuperBlockCounts::zero(), 0, (SuperBlockCounts::NUM_COUNTS - 1) as u8, 0);
}

#[test]
pub fn test_add() {
    let mut sbc = SuperBlockCounts::zero();
    sbc.add(5);
    assert_range(&sbc, 0, 5, 1);
}


#[test]
pub fn test_add2() {
    // 00 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15
    //  3  3  3  3  3  3  2  2  1  1  1  1  1  1  0  0
    let mut sbc = SuperBlockCounts::zero();
    sbc.add(13);
    sbc.add(7);
    sbc.add(5);
    assert_range(&sbc, 0, 5, 3);
    assert_range(&sbc, 6, 7, 2);
    assert_range(&sbc, 8, 13, 1);
}


#[test]
pub fn test_substract() {
    let mut sbc = SuperBlockCounts::zero();
    sbc.add(15);
    sbc.substract(5);
    assert_range(&sbc, 0, 5, 0);
    assert_range(&sbc, 6, 15, 1);
}


#[test]
pub fn test_substract2() {
    let mut sbc = SuperBlockCounts::zero();
    sbc.add(15);
    sbc.add(15);
    sbc.add(15);
    sbc.substract(13);
    sbc.substract(7);
    sbc.substract(5);
    assert_range(&sbc, 0, 5, 0);
    assert_range(&sbc, 6, 7, 1);
    assert_range(&sbc, 8, 13, 2);
    assert_range(&sbc, 14, 15, 3);
}


#[test]
pub fn test_get_candidate_depth() {
    // 00 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 .. 255
    //  3  3  3  3  3  3  2  2  1  1  1  1  1  1  0  0      0
    let mut sbc = SuperBlockCounts::zero();
    sbc.add(13);
    sbc.add(7);
    sbc.add(5);
    assert_eq!(sbc.get_candidate_depth(0), 255);
    assert_eq!(sbc.get_candidate_depth(1), 13);
    assert_eq!(sbc.get_candidate_depth(2), 7);
    assert_eq!(sbc.get_candidate_depth(3), 5);
}
