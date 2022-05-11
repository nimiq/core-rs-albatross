use nimiq_test_log::test;
use nimiq_utils::crc::*;

#[test]
fn compute_crc8() {
    let mut crc8_comp = Crc8Computer::default();
    crc8_comp.update(&[0u8]);
    assert_eq!(crc8_comp.result(), 0);

    crc8_comp = Crc8Computer::default();
    let a: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    crc8_comp.update(&a);
    assert_eq!(crc8_comp.result(), 255);

    crc8_comp = Crc8Computer::default();
    let a: [u8; 32] = [
        169, 203, 76, 129, 160, 230, 129, 141, 117, 240, 195, 239, 197, 18, 196, 30, 26, 52, 253,
        1, 21, 81, 249, 22, 234, 115, 246, 14, 62, 197, 228, 223,
    ];
    crc8_comp.update(&a);
    assert_eq!(crc8_comp.result(), 165);
}
