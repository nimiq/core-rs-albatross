fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_collections::BitSet;
        use nimiq_serde::Deserialize as _;

        let _ = BitSet::deserialize_from_vec(data);
    })
}
