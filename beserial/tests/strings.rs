use beserial::{DeserializeWithLength, SerializeWithLength};

#[test]
fn it_correctly_serializes_and_deserializes_string() {
    fn reserialize(s: String) -> String {
        let mut v = Vec::with_capacity(50);
        SerializeWithLength::serialize::<u16, Vec<u8>>(&s, &mut v).unwrap();
        let s2: String = DeserializeWithLength::deserialize::<u16, &[u8]>(&mut &v[..]).unwrap();
        return s2;
    }
    assert!(reserialize("a".into()) == "a");
    assert!(reserialize("kjsSDFsdf345SD@$%^&".into()) == "kjsSDFsdf345SD@$%^&");
    assert!(reserialize("a".into()) != "b");
}
