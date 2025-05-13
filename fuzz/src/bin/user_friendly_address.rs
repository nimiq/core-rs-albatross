fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use std::str;

        use nimiq_keys::Address;

        if let Ok(data) = str::from_utf8(data) {
            let _ = Address::from_user_friendly_address(data);
        }
    })
}
