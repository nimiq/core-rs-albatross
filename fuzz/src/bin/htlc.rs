fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_account::HashedTimeLockedContract;
        use nimiq_serde::Deserialize as _;
        let _ = HashedTimeLockedContract::deserialize_from_vec(data);
    })
}
