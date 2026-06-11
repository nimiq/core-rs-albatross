#[cfg(feature = "genesis-override")]
use std::path::Path;
use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use nimiq_block::Block;
use nimiq_bls::{LazyPublicKey as BlsLazyPublicKey, PublicKey as BlsPublicKey};
#[cfg(feature = "genesis-override")]
use nimiq_database::mdbx::MdbxDatabase;
#[cfg(feature = "genesis-override")]
use nimiq_genesis_builder::{GenesisBuilder, GenesisBuilderError, GenesisInfo};
use nimiq_hash::Blake2bHash;
pub use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::trie::TrieItem;
use nimiq_serde::Deserialize;
#[cfg(feature = "genesis-override")]
use nimiq_serde::Serialize;

#[derive(Clone, Debug)]
struct GenesisData {
    block: &'static [u8],
    decompressed_keys: &'static [u8],
    hash: Blake2bHash,
    accounts: Option<&'static [u8]>,
}

/// A hardcoded, per-network reference to a recent trusted election block.
///
/// Pico clients use it as a sync anchor on the sync fallback: they fetch the block by `hash` (the
/// hash commits to the full election header, so a matching block is the trusted block) and seed
/// their chain to it instead of re-syncing from genesis. Other clients (light/full/history) do not
/// sync from it but verify it against their own chain.
///
/// This is intentionally distinct from `nimiq_consensus::messages::Checkpoint` (a transient
/// macro-sync hint): this one is a release-time hardcoded anchor, and lives in this low-level
/// crate to avoid a dependency cycle.
#[derive(Clone, Debug)]
pub struct HardcodedElection {
    /// Block height of the checkpoint election block.
    pub block_number: u32,
    /// Blake2b hash of the checkpoint election block.
    pub hash: Blake2bHash,
}

/// Set once if a hardcoded checkpoint has been found NOT to match the local chain.
/// Surfaced as the `checkpoint_mismatch` metric.
static CHECKPOINT_MISMATCH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct NetworkInfo {
    network_id: NetworkId,
    name: &'static str,
    genesis: GenesisData,
    max_supported_version: u16,
}

impl NetworkInfo {
    #[inline]
    pub fn network_id(&self) -> NetworkId {
        self.network_id
    }

    #[inline]
    pub fn name(&self) -> String {
        self.name.into()
    }

    #[inline]
    pub fn genesis_block(&self) -> Block {
        let mut block = Block::deserialize_from_vec(self.genesis.block)
            .expect("Failed to deserialize genesis block.");
        block.populate_cached_hash(self.genesis.hash.clone());
        block
    }

    #[inline]
    pub fn genesis_hash(&self) -> &Blake2bHash {
        &self.genesis.hash
    }

    /// The hardcoded checkpoint (a recent trusted election block) for this network, if any.
    #[inline]
    pub fn checkpoint(&self) -> Option<HardcodedElection> {
        checkpoint_for_network(self.network_id)
    }

    #[inline]
    pub fn genesis_accounts(&self) -> Option<Vec<TrieItem>> {
        self.genesis.accounts.as_ref().map(|accounts| {
            Deserialize::deserialize_from_vec(accounts)
                .expect("Failed to deserialize genesis accounts.")
        })
    }

    #[inline]
    pub fn max_supported_version(&self) -> u16 {
        self.max_supported_version
    }

    pub fn from_network_id(network_id: NetworkId) -> &'static Self {
        network(network_id).unwrap_or_else(|| panic!("No such network ID: {network_id}"))
    }
}

/// Returns the hardcoded checkpoint election block for a network, if one has been set.
///
/// Refreshed each release: paste the latest election block's `(block_number, hash)` from a
/// trusted, fully-synced node. Networks with ephemeral genesis (dev/unit) or no checkpoint
/// yet return `None`, in which case clients behave exactly as without checkpoints.
fn checkpoint_for_network(network_id: NetworkId) -> Option<HardcodedElection> {
    match network_id {
        // Election block #53611200 (2026-06-15), refreshed per release from a trusted node.
        NetworkId::MainAlbatross => Some(HardcodedElection {
            block_number: 53_611_200,
            hash: "f2953c1a7597422587f76dff07fd5786e41425a2c51d4837ebc3691f05eb5b43"
                .parse()
                .expect("valid checkpoint hash"),
        }),
        _ => None,
    }
}

/// Whether a hardcoded checkpoint has been found NOT to match the local chain.
pub fn checkpoint_mismatch() -> bool {
    CHECKPOINT_MISMATCH.load(Ordering::Relaxed)
}

/// Outcome of comparing a just-committed election block against the hardcoded checkpoint.
#[derive(Debug, PartialEq, Eq)]
enum CheckpointCheck {
    /// Block is at the checkpoint height and the hash matches.
    Match,
    /// Block is at the checkpoint height but the hash differs.
    Mismatch,
    /// Block is not at the checkpoint height; nothing to check.
    NotApplicable,
}

/// Pure comparison of a committed election block against a checkpoint (no logging or metric
/// side effects, so it can be unit-tested directly).
fn check_against_checkpoint(
    checkpoint: &HardcodedElection,
    block_number: u32,
    hash: &Blake2bHash,
) -> CheckpointCheck {
    if block_number != checkpoint.block_number {
        CheckpointCheck::NotApplicable
    } else if *hash == checkpoint.hash {
        CheckpointCheck::Match
    } else {
        CheckpointCheck::Mismatch
    }
}

/// Verifies a just-committed election block against the network's hardcoded checkpoint, if any.
///
/// Non-pico clients call this from the blockchain election-commit paths. When the committed
/// block is at the checkpoint height but its hash differs, it logs an error and flags the
/// `checkpoint_mismatch` metric — meaning either this client is on a different chain or the
/// bundled checkpoint is wrong. It never panics; a missing checkpoint or a non-matching height
/// is a no-op.
pub fn verify_election_checkpoint(network_id: NetworkId, block_number: u32, hash: &Blake2bHash) {
    let Some(checkpoint) = checkpoint_for_network(network_id) else {
        return;
    };
    match check_against_checkpoint(&checkpoint, block_number, hash) {
        CheckpointCheck::NotApplicable => {}
        CheckpointCheck::Match => {
            log::info!(
                "Hardcoded checkpoint at block #{block_number} verified against the local chain"
            );
        }
        CheckpointCheck::Mismatch => {
            log::error!(
                "Hardcoded checkpoint at block #{} does NOT match the local chain (expected {}, \
                 got {}). This client may be on a different chain, or the bundled checkpoint is \
                 incorrect.",
                block_number,
                checkpoint.hash,
                hash,
            );
            CHECKPOINT_MISMATCH.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "genesis-override")]
fn read_genesis_config(config: &Path) -> Result<GenesisData, GenesisBuilderError> {
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    let GenesisInfo {
        block,
        hash,
        accounts,
    } = GenesisBuilder::from_config_file(config)?.generate(env)?;

    let block = block.serialize_to_vec();
    let accounts = accounts.map(|accounts| accounts.serialize_to_vec());

    Ok(GenesisData {
        block: Box::leak(block.into_boxed_slice()),
        decompressed_keys: &[],
        hash,
        accounts: accounts.map(|accounts| Box::leak(accounts.into_boxed_slice()) as &'static _),
    })
}

static KEYS_DEV: OnceLock<Box<[BlsLazyPublicKey]>> = OnceLock::new();
static KEYS_TEST: OnceLock<Box<[BlsLazyPublicKey]>> = OnceLock::new();
static KEYS_UNIT: OnceLock<Box<[BlsLazyPublicKey]>> = OnceLock::new();
static KEYS_MAIN: OnceLock<Box<[BlsLazyPublicKey]>> = OnceLock::new();

fn network(network_id: NetworkId) -> Option<&'static NetworkInfo> {
    let result = network_impl(network_id);
    if let Some(info) = result {
        assert_eq!(network_id, info.network_id);
        assert_eq!(network_id, info.genesis_block().network());
        let keys = match network_id {
            NetworkId::DevAlbatross => &KEYS_DEV,
            NetworkId::TestAlbatross => &KEYS_TEST,
            NetworkId::UnitAlbatross => &KEYS_UNIT,
            NetworkId::MainAlbatross => &KEYS_MAIN,
            _ => unreachable!(),
        };
        keys.get_or_init(|| {
            info.genesis
                .decompressed_keys
                .chunks(BlsPublicKey::TRUSTED_SERIALIZATION_SIZE)
                .map(|chunk| {
                    BlsLazyPublicKey::from(BlsPublicKey::trusted_deserialize(
                        &chunk.try_into().unwrap(),
                    ))
                })
                .collect()
        });
    }
    result
}

fn network_impl(network_id: NetworkId) -> Option<&'static NetworkInfo> {
    Some(match network_id {
        NetworkId::DevAlbatross => {
            #[cfg(feature = "genesis-override")]
            {
                use std::sync::OnceLock;
                static OVERRIDE: OnceLock<Option<NetworkInfo>> = OnceLock::new();
                if let Some(info) = OVERRIDE.get_or_init(|| {
                    let override_path = env::var_os("NIMIQ_OVERRIDE_DEVNET_CONFIG");
                    override_path.map(|p| NetworkInfo {
                        network_id: NetworkId::DevAlbatross,
                        name: "dev-albatross",
                        genesis: read_genesis_config(Path::new(&p))
                            .expect("failure reading provided NIMIQ_OVERRIDE_DEVNET_CONFIG"),
                        max_supported_version: 1,
                    })
                }) {
                    return Some(info);
                }
            }
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::DevAlbatross,
                name: "dev-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/dev-albatross/genesis.rs",
                )),
                max_supported_version: 1,
            };
            &INFO
        }
        NetworkId::TestAlbatross => {
            #[cfg(feature = "genesis-override")]
            {
                use std::sync::OnceLock;
                static OVERRIDE: OnceLock<Option<NetworkInfo>> = OnceLock::new();
                if let Some(info) = OVERRIDE.get_or_init(|| {
                    let override_path = env::var_os("NIMIQ_OVERRIDE_TESTNET_CONFIG");
                    override_path.map(|p| NetworkInfo {
                        network_id: NetworkId::TestAlbatross,
                        name: "test-albatross",
                        genesis: read_genesis_config(Path::new(&p))
                            .expect("failure reading provided NIMIQ_OVERRIDE_TESTNET_CONFIG"),
                        max_supported_version: 1,
                    })
                }) {
                    return Some(info);
                }
            }
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::TestAlbatross,
                name: "test-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/test-albatross/genesis.rs"
                )),
                max_supported_version: 1,
            };
            &INFO
        }
        NetworkId::UnitAlbatross => {
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::UnitAlbatross,
                name: "unit-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/unit-albatross/genesis.rs"
                )),
                max_supported_version: 5,
            };
            &INFO
        }
        NetworkId::MainAlbatross => {
            #[cfg(feature = "genesis-override")]
            {
                use std::sync::OnceLock;
                static OVERRIDE: OnceLock<Option<NetworkInfo>> = OnceLock::new();
                if let Some(info) = OVERRIDE.get_or_init(|| {
                    let override_path = env::var_os("NIMIQ_OVERRIDE_MAINNET_CONFIG");
                    override_path.map(|p| NetworkInfo {
                        network_id: NetworkId::MainAlbatross,
                        name: "main-albatross",
                        genesis: read_genesis_config(Path::new(&p))
                            .expect("failure reading provided NIMIQ_OVERRIDE_MAINNET_CONFIG"),
                        max_supported_version: 1,
                    })
                }) {
                    return Some(info);
                }
            }
            static INFO: NetworkInfo = NetworkInfo {
                network_id: NetworkId::MainAlbatross,
                name: "main-albatross",
                genesis: include!(concat!(
                    env!("OUT_DIR"),
                    "/genesis/main-albatross/genesis.rs"
                )),
                max_supported_version: 1,
            };
            &INFO
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint() -> HardcodedElection {
        HardcodedElection {
            block_number: 100,
            hash: Blake2bHash::from([1u8; 32]),
        }
    }

    #[test]
    fn detects_matching_checkpoint() {
        let cp = checkpoint();
        assert_eq!(
            check_against_checkpoint(&cp, cp.block_number, &cp.hash),
            CheckpointCheck::Match
        );
    }

    #[test]
    fn detects_mismatched_checkpoint() {
        let cp = checkpoint();
        let other = Blake2bHash::from([2u8; 32]);
        assert_eq!(
            check_against_checkpoint(&cp, cp.block_number, &other),
            CheckpointCheck::Mismatch
        );
    }

    #[test]
    fn ignores_other_heights() {
        let cp = checkpoint();
        // A different height is a no-op even if the hash happens to match.
        assert_eq!(
            check_against_checkpoint(&cp, cp.block_number + 1, &cp.hash),
            CheckpointCheck::NotApplicable
        );
    }
}
