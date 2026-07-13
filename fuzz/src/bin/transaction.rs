fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_serde::Deserialize as _;
        use nimiq_transaction::Transaction;
        let _ = Transaction::deserialize_from_vec(data);
    })
}
