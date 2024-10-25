fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_bls::KeyPair;
        use nimiq_serde::Deserialize as _;
        let _ = KeyPair::deserialize_from_vec(data);
    })
}
