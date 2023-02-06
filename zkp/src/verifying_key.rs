use std::{env, io::Cursor};

use ark_ec::mnt6::MNT6;
use ark_groth16::VerifyingKey;
use ark_mnt6_753::Parameters;
use ark_serialize::CanonicalDeserialize;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref ZKP_VERIFYING_KEY: VerifyingKey<MNT6<Parameters>> = {
        let serialized = include_bytes!(concat!(env!("OUT_DIR"), "/verifying_key.data"));

        let mut serialized_cursor = Cursor::new(serialized);
        VerifyingKey::deserialize_unchecked(&mut serialized_cursor)
            .expect("Invalid verifying key. Please rebuild the client.")
    };
}
