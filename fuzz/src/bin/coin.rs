fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_primitives::coin::Coin;
        use nimiq_serde::Deserialize as _;
        let _ = Coin::deserialize_from_vec(data);
    })
}
