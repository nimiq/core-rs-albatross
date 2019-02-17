use beserial::{Deserialize, Serialize, uvar};

#[test]
fn it_correctly_serializes_and_deserializes_uvar() {
    fn reserialize(u: u64) -> u64 {
        let uu: uvar = u.into();
        let mut v = Vec::with_capacity(9);
        Serialize::serialize(&uu, &mut v).unwrap();
        let uv: uvar = Deserialize::deserialize(&mut &v[..]).unwrap();
        return uv.into();
    }
    assert_eq!(reserialize(0), 0);
    assert_eq!(reserialize(1), 1);
    assert_eq!(reserialize(127), 127);
    assert_eq!(reserialize(128), 128);
    assert_eq!(reserialize(4223), 4223);
    assert_eq!(reserialize(4224), 4224);
    assert_eq!(reserialize(16511), 16511);
    assert_eq!(reserialize(16512), 16512);
    assert_eq!(reserialize(16513), 16513);
    assert_eq!(reserialize(2113663), 2113663);
    assert_eq!(reserialize(2113664), 2113664);
    assert_eq!(reserialize(2113665), 2113665);
    assert_eq!(reserialize(270549119), 270549119);
    assert_eq!(reserialize(270549120), 270549120);
    assert_eq!(reserialize(270549121), 270549121);
    assert_eq!(reserialize(2164392959), 2164392959);
    assert_eq!(reserialize(2164392960), 2164392960);
    assert_eq!(reserialize(2164392961), 2164392961);
    assert_eq!(reserialize(10), 10);
    assert_eq!(reserialize(100), 100);
    assert_eq!(reserialize(1000), 1000);
    assert_eq!(reserialize(10000), 10000);
    assert_eq!(reserialize(100000), 100000);
    assert_eq!(reserialize(1000000), 1000000);
    assert_eq!(reserialize(10000000), 10000000);
    assert_eq!(reserialize(100000000), 100000000);
    assert_eq!(reserialize(1000000000), 1000000000);
    assert_eq!(reserialize(10000000000), 10000000000);
    assert_eq!(reserialize(100000000000), 100000000000);
    assert_eq!(reserialize(1000000000000), 1000000000000);
    assert_eq!(reserialize(10000000000000), 10000000000000);
    assert_eq!(reserialize(100000000000000), 100000000000000);
    assert_eq!(reserialize(1000000000000000), 1000000000000000);
    assert_eq!(reserialize(10000000000000000), 10000000000000000);
    assert_eq!(reserialize(100000000000000000), 100000000000000000);
    assert_eq!(reserialize(9223372036854775807), 9223372036854775807);
    assert_eq!(reserialize(18446744073709551615), 18446744073709551615);
}
