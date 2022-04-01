use beserial::{uvar, Deserialize, DeserializeWithLength, Serialize, SerializeWithLength};

#[test]
fn it_correctly_serializes_and_deserializes_uvar() {
    fn reserialize(u: u64) -> u64 {
        let uu: uvar = u.into();
        let mut v = Vec::with_capacity(9);
        Serialize::serialize(&uu, &mut v).unwrap();
        let uv: uvar = Deserialize::deserialize(&mut &v[..]).unwrap();
        uv.into()
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

#[test]
fn it_serializes_and_deserializes_box() {
    let b = Box::new(1337);
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized: Box<i32> = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);
}

#[test]
fn it_serializes_and_deserializes_option() {
    let b = Some(1337);
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized: Option<i32> = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let c: Option<i32> = None;
    let serialized = Serialize::serialize_to_vec(&c);
    let deserialized: Option<i32> = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, c);
}

#[test]
fn it_serializes_and_deserializes_result() {
    let b = Ok(1337);
    let serialized = Serialize::serialize_to_vec(&b);
    let deserialized: Result<i32, ()> = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, b);

    let c: Result<i32, ()> = Err(());
    let serialized = Serialize::serialize_to_vec(&c);
    let deserialized: Result<i32, ()> = Deserialize::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(deserialized, c);
}

#[test]
fn it_serializes_and_deserializes_vec() {
    let vec = vec![1, 4, 7, 4, 3, 6, 9, 9, 4];
    let serialized = SerializeWithLength::serialize_to_vec::<u8>(&vec);
    let deserialized: Vec<i32> =
        DeserializeWithLength::deserialize_from_vec::<u8>(&serialized).unwrap();
    assert_eq!(deserialized, vec);
}

#[test]
fn it_correctly_serializes_and_deserializes_string() {
    fn reserialize(s: String) -> String {
        let mut v = Vec::with_capacity(50);
        SerializeWithLength::serialize::<u16, Vec<u8>>(&s, &mut v).unwrap();
        let s2: String = DeserializeWithLength::deserialize::<u16, &[u8]>(&mut &v[..]).unwrap();
        s2
    }
    assert!(reserialize("a".into()) == "a");
    assert!(reserialize("kjsSDFsdf345SD@$%^&".into()) == "kjsSDFsdf345SD@$%^&");
    assert!(reserialize("a".into()) != "b");
}
