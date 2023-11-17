use std::{env, io::Cursor};

use ark_groth16::VerifyingKey;
use ark_serialize::CanonicalDeserialize;
use nimiq_primitives::networks::NetworkId;
use nimiq_serde::Deserialize;
use nimiq_zkp_circuits::metadata::VerifyingKeyMetadata;
use nimiq_zkp_primitives::VerifyingData;
use once_cell::sync::OnceCell;

pub struct ZKPVerifyingKey {
    cell: OnceCell<VerifyingData>,
}

impl ZKPVerifyingKey {
    pub const fn new() -> Self {
        ZKPVerifyingKey {
            cell: OnceCell::new(),
        }
    }

    pub fn init_with_network_id(&self, network_id: NetworkId) {
        self.init_with_data(Self::init_verifying_key(network_id))
    }

    pub fn init_with_data(&self, verifying_data: VerifyingData) {
        assert!(self.cell.set(verifying_data).is_ok())
    }

    fn init_verifying_key(network_id: NetworkId) -> VerifyingData {
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
        VerifyingData {
            merger_wrapper_vk: VerifyingKey::deserialize_uncompressed_unchecked(
                &mut serialized_cursor,
            )
            .expect("Invalid verifying key. Please rebuild the client."),
            keys_commitment: metadata.vks_commitment(),
        }
    }
}

impl std::ops::Deref for ZKPVerifyingKey {
    type Target = VerifyingData;
    fn deref(&self) -> &VerifyingData {
        self.cell
            .get_or_init(|| Self::init_verifying_key(NetworkId::UnitAlbatross))
    }
}

pub static ZKP_VERIFYING_DATA: ZKPVerifyingKey = ZKPVerifyingKey::new();
