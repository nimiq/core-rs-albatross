fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_primitives::key_nibbles::KeyNibbles;
        use nimiq_serde::Deserialize as _;

        let _ = KeyNibbles::deserialize_from_vec(data);
    })
}
