use std::{env, io::Cursor};

use ark_ec::mnt6::MNT6;
use ark_groth16::VerifyingKey;
use ark_mnt6_753::Parameters;
use ark_serialize::CanonicalDeserialize;
use lazy_static::lazy_static;
use parking_lot::RwLock;

lazy_static! {
    pub static ref ZKP_VERIFYING_KEY: RwLock<Option<VerifyingKey<MNT6<Parameters>>>> =
        RwLock::new(init_verifying_key());
}

fn init_verifying_key() -> Option<VerifyingKey<MNT6<Parameters>>> {
    let serialized = include_bytes!(concat!(env!("OUT_DIR"), "/verifying_keys.data"));

    let mut serialized_cursor = Cursor::new(serialized);
    VerifyingKey::deserialize_unchecked(&mut serialized_cursor).ok()
}

pub fn set_verifying_key(key: VerifyingKey<MNT6<Parameters>>) {
    *ZKP_VERIFYING_KEY.write() = Some(key);
}
