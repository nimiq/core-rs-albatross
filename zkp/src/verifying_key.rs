use std::{env, io::Cursor};

use ark_ec::mnt6::MNT6;
use ark_groth16::VerifyingKey;
use ark_mnt6_753::Config;
use ark_serialize::CanonicalDeserialize;
use nimiq_primitives::networks::NetworkId;
use nimiq_serde::Deserialize;
use nimiq_zkp_circuits::metadata::VerifyingKeyMetadata;
use once_cell::sync::OnceCell;

pub struct ZKPVerifyingKey {
    cell: OnceCell<VerifyingKey<MNT6<Config>>>,
}

impl ZKPVerifyingKey {
    pub const fn new() -> Self {
        ZKPVerifyingKey {
            cell: OnceCell::new(),
        }
    }

    pub fn init_with_network_id(&self, network_id: NetworkId) {
        self.init_with_key(Self::init_verifying_key(network_id))
    }

    pub fn init_with_key(&self, verifying_key: VerifyingKey<MNT6<Config>>) {
        assert!(self.cell.set(verifying_key).is_ok())
    }

    fn init_verifying_key(network_id: NetworkId) -> VerifyingKey<MNT6<Config>> {
        let (key_bytes, metadata_bytes) = match network_id {
            NetworkId::DevAlbatross => (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../.zkp/verifying_keys/merger_wrapper.bin"
                )),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../.zkp/meta_data.bin"
                )),
            ),
            NetworkId::TestAlbatross => (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../.zkp_testnet/verifying_keys/merger_wrapper.bin"
                )),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../.zkp_testnet/meta_data.bin"
                )),
            ),
            NetworkId::UnitAlbatross => (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../.zkp_tests/verifying_keys/merger_wrapper.bin"
                )),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../.zkp_tests/meta_data.bin"
                )),
            ),
            _ => panic!("Network id {:?} does not have a verifying key!", network_id),
        };

        let metadata = VerifyingKeyMetadata::deserialize_from_vec(metadata_bytes)
            .expect("Invalid metadata. Please rebuild the ZKP keys.");

        assert!(
            metadata.matches(network_id),
            "ZKP metadata does not match current network. Please rebuild the ZKP keys."
        );

        let mut serialized_cursor = Cursor::new(key_bytes);
        VerifyingKey::deserialize_uncompressed_unchecked(&mut serialized_cursor)
            .expect("Invalid verifying key. Please rebuild the client.")
    }
}

impl std::ops::Deref for ZKPVerifyingKey {
    type Target = VerifyingKey<MNT6<Config>>;
    fn deref(&self) -> &VerifyingKey<MNT6<Config>> {
        self.cell
            .get_or_init(|| Self::init_verifying_key(NetworkId::UnitAlbatross))
    }
}

pub static ZKP_VERIFYING_KEY: ZKPVerifyingKey = ZKPVerifyingKey::new();
