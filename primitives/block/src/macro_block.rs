use std::{fmt, io};

use ark_ec::PrimeGroup;
use nimiq_bls::{G2Projective, PublicKey as BlsPublicKey};
use nimiq_collections::bitset::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput, Hasher, SerializeContent};
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    networks::NetworkId,
    policy::{upgrades, Policy},
    slots_allocation::{Validators, ValidatorsBuilder},
    Message, PREFIX_TENDERMINT_PROPOSAL,
};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize, SerializedSize};
use nimiq_transaction::reward::RewardTransaction;
use nimiq_vrf::VrfSeed;
use thiserror::Error;

use crate::{tendermint::TendermintProof, BlockError};

/// The struct representing a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct MacroBlock {
    /// The header, contains some basic information and commitments to the body and the state.
    pub header: MacroHeader,
    /// The body of the block.
    pub body: Option<MacroBody>,
    /// The justification, contains all the information needed to verify that the header was signed
    /// by the correct producers.
    pub justification: Option<TendermintProof>,
}

impl MacroBlock {
    /// Returns the network ID of this macro block.
    pub fn network(&self) -> NetworkId {
        self.header.network
    }

    /// Returns the Blake2b hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }

    /// Returns the Blake2s hash of the block header.
    pub fn hash_blake2s(&self) -> Blake2sHash {
        Hash::hash(&self.header)
    }

    /// Computes the next interlink from self.header.interlink
    pub fn get_next_interlink(&self) -> Result<Vec<Blake2bHash>, BlockError> {
        if !self.is_election() {
            return Err(BlockError::InvalidBlockType);
        }
        let mut interlink = self
            .header
            .interlink
            .clone()
            .expect("Election blocks have interlinks");
        let number_hashes_to_update = if self.block_number() == Policy::genesis_block_number() {
            // 0.trailing_zeros() would be 32, thus we need an exception for it
            0
        } else {
            ((self.block_number() - Policy::genesis_block_number()) / Policy::blocks_per_epoch())
                .trailing_zeros() as usize
        };
        if number_hashes_to_update > interlink.len() {
            interlink.push(self.hash());
        }
        assert!(
            interlink.len() >= number_hashes_to_update,
            "{} {}",
            interlink.len(),
            number_hashes_to_update,
        );
        #[allow(clippy::needless_range_loop)]
        for i in 0..number_hashes_to_update {
            interlink[i] = self.hash();
        }
        Ok(interlink)
    }

    /// Returns whether this macro block is an election block.
    pub fn is_election(&self) -> bool {
        self.header.is_election()
    }

    /// Returns a copy of the validator slots. Only returns Some if it is an election block.
    pub fn get_validators(&self) -> Option<Validators> {
        self.header.validators.clone()
    }

    /// Returns the block number of this macro block.
    pub fn block_number(&self) -> u32 {
        self.header.block_number
    }

    /// Returns the batch number of this macro block.
    pub fn batch_number(&self) -> u32 {
        Policy::batch_at(self.header.block_number)
    }

    /// Returns the epoch number of this macro block.
    pub fn epoch_number(&self) -> u32 {
        Policy::epoch_at(self.header.block_number)
    }

    /// Returns the block number of this macro block.
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }

    /// Return the round of this macro block.
    pub fn round(&self) -> u32 {
        self.header.round
    }

    /// Verifies that the block is valid for the given validators.
    pub(crate) fn verify_validators(&self, validators: &Validators) -> Result<(), BlockError> {
        // Verify the Tendermint proof.
        if !TendermintProof::verify(self, validators) {
            warn!(
                block = %self,
                reason = "Macro block with bad justification",
                "Rejecting block"
            );
            return Err(BlockError::InvalidJustification);
        }

        Ok(())
    }

    /// Creates a default block that has body and justification.
    pub fn non_empty_default() -> Self {
        let mut validators = ValidatorsBuilder::new();
        for _ in 0..Policy::SLOTS {
            validators.push(
                Address::default(),
                BlsPublicKey::new(G2Projective::generator()).compress(),
                SchnorrPublicKey::default(),
            );
        }

        let validators = Some(validators.build());
        let body = MacroBody {
            ..Default::default()
        };
        let body_root = body.hash();
        MacroBlock {
            header: MacroHeader {
                body_root,
                validators,
                ..Default::default()
            },
            body: Some(body),
            justification: Some(TendermintProof {
                round: 0,
                sig: Default::default(),
            }),
        }
    }
}

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.header, f)
    }
}

/// The struct representing the header of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MacroHeader {
    /// Network of the block.
    pub network: NetworkId,
    /// The version number of the block. Changing this always results in a hard fork.
    pub version: u16,
    /// The number of the block.
    pub block_number: u32,
    /// The round number this block was proposed in.
    pub round: u32,
    /// The timestamp of the block. It follows the Unix time and has millisecond precision.
    pub timestamp: u64,
    /// The hash of the header of the immediately preceding block (either micro or macro).
    pub parent_hash: Blake2bHash,
    /// The hash of the header of the preceding election macro block.
    pub parent_election_hash: Blake2bHash,
    /// Hashes of the last blocks divisible by 2^x
    pub interlink: Option<Vec<Blake2bHash>>,
    /// The seed of the block. This is the BLS signature of the seed of the immediately preceding
    /// block (either micro or macro) using the validator key of the block proposer.
    pub seed: VrfSeed,
    /// The extra data of the block. It is simply up to 32 raw bytes.
    ///
    /// It encodes the initial supply in the genesis block, as a big-endian `u64`.
    ///
    /// No planned use otherwise.
    pub extra_data: Vec<u8>,
    /// The root of the Merkle tree of the blockchain state. It just acts as a commitment to the
    /// state.
    pub state_root: Blake2bHash,
    /// The hash of the body. It just acts as a commitment to the body.
    pub body_root: Blake2sHash,
    /// The root of the trie diff tree proof.
    pub diff_root: Blake2bHash,
    /// A merkle root over all the transactions that happened in the current epoch.
    pub history_root: Blake2bHash,
    /// Contains all the information regarding the next validator set, i.e. their validator
    /// public key, their reward address and their assigned validator slots.
    /// Is only Some when the macro block is an election block.
    pub validators: Option<Validators>,
    /// A bitset representing which validator slots will be prohibited from producing micro blocks or
    /// proposing macro blocks in the batch following this macro block.
    /// This set is needed for nodes that do not have the state as it is normally computed
    /// inside the staking contract.
    pub next_batch_initial_punished_set: BitSet,
    /// The cached hash of this header. This is NOT sent over the wire.
    #[serde(skip)]
    pub cached_hash: Option<Blake2bHash>,
}

impl MacroHeader {
    /// Returns the Blake2b hash of this header.
    pub fn hash(&self) -> Blake2bHash {
        if let Some(hash) = &self.cached_hash {
            return hash.clone();
        }
        Hash::hash(&self)
    }

    /// Returns the Blake2b hash of this header and caches the result internally.
    pub fn hash_cached(&mut self) -> Blake2bHash {
        if self.cached_hash.is_none() {
            self.cached_hash = Some(Hash::hash(self));
        }
        self.cached_hash.as_ref().unwrap().clone()
    }

    /// Returns whether this macro block is an election block.
    pub fn is_election(&self) -> bool {
        Policy::is_election_block_at(self.block_number)
    }

    pub(crate) fn verify(&self) -> Result<(), BlockError> {
        // Check that validators are only set on election blocks.
        if self.is_election() != self.validators.is_some() {
            return Err(BlockError::InvalidValidators);
        }
        // Validate that all BLS voting keys can be decompressed and structural invariants hold.
        if let Some(ref validators) = self.validators {
            validators
                .validate_keys()
                .map_err(|_| BlockError::InvalidValidators)?;
        }
        // The punished set may only reference in-range validator slots. An out-of-range bit
        // would let a malformed set disable every in-range slot while evading the all-disabled
        // fast path in `compute_slot_number`, which would otherwise divide by zero on light/pico
        // clients that adopt this header without recomputing the set from the staking contract
        if self
            .next_batch_initial_punished_set
            .iter()
            .any(|slot| slot >= Policy::SLOTS as usize)
        {
            return Err(BlockError::InvalidPunishedSet);
        }
        Ok(())
    }

    /// Blake2s root over the payload, folded into the header hash by `serialize_content`.
    /// See `serialize_payload_commitment` for the contents.
    fn payload_commitment_root(&self) -> Blake2sHash {
        let mut h = <Blake2sHash as HashOutput>::Builder::default();
        self.serialize_payload_commitment(&mut h).unwrap();
        h.finish()
    }

    /// Serializes the payload commitment embedded in the header hash: the elected
    /// validator set (election blocks only), the next batch's initial punished set, and
    /// the body root.
    ///
    /// The validator set uses `Validators::hash` (voting keys only) before v2, and
    /// `Validators::commitment_hash` (full set) from v2 on, which additionally binds the
    /// signing keys, addresses and slot ranges so they can't be swapped under an
    /// unchanged hash.
    pub fn serialize_payload_commitment<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if let Some(ref validators) = self.validators {
            // v1 hashing is left untouched so mainnet/testnet header hashes stay
            // byte-identical until the v2 upgrade activates.
            let validators_root =
                if self.version >= upgrades::v2::ELECTION_VALIDATOR_METADATA_COMMITMENT {
                    validators.commitment_hash::<Blake2sHash>()
                } else {
                    validators.hash::<Blake2sHash>()
                };
            validators_root.serialize_to_writer(writer)?;
        } else {
            0u8.serialize_to_writer(writer)?;
        }

        let punished_set_hash = self
            .next_batch_initial_punished_set
            .serialize_to_vec()
            .hash::<Blake2sHash>();
        punished_set_hash.serialize_to_writer(writer)?;

        self.body_root.serialize_to_writer(writer)?;

        Ok(())
    }
}

impl SerializedMaxSize for MacroHeader {
    #[allow(clippy::identity_op)]
    const MAX_SIZE: usize = 0
        + /*network*/ NetworkId::SIZE
        + /*version*/ u16::MAX_SIZE
        + /*block_number*/ u32::MAX_SIZE
        + /*round*/ u32::MAX_SIZE
        + /*timestamp*/ u64::MAX_SIZE
        + /*parent_hash*/ Blake2bHash::SIZE
        + /*parent_election_hash*/ Blake2bHash::SIZE
        + /*interlink*/ nimiq_serde::option_max_size(nimiq_serde::seq_max_size(Blake2bHash::SIZE, 32))
        + /*seed*/ VrfSeed::SIZE
        + /*extra_data*/ nimiq_serde::seq_max_size(u8::SIZE, 32)
        + /*state_root*/ Blake2bHash::SIZE
        + /*body_root*/ Blake2sHash::SIZE
        + /*diff_root*/ Blake2bHash::SIZE
        + /*history_root*/ Blake2bHash::SIZE
        + /*validators*/ nimiq_serde::option_max_size(Validators::MAX_SIZE)
        + /*next_batch_punished_set*/ BitSet::max_size(Policy::SLOTS as usize);
}

// We can't derive this because we want to ignore the `cached_hash` field.
impl PartialEq for MacroHeader {
    fn eq(&self, other: &Self) -> bool {
        self.network == other.network
            && self.version == other.version
            && self.block_number == other.block_number
            && self.round == other.round
            && self.timestamp == other.timestamp
            && self.parent_hash == other.parent_hash
            && self.parent_election_hash == other.parent_election_hash
            && self.interlink == other.interlink
            && self.seed == other.seed
            && self.extra_data == other.extra_data
            && self.state_root == other.state_root
            && self.body_root == other.body_root
            && self.diff_root == other.diff_root
            && self.history_root == other.history_root
            && self.validators == other.validators
            && self.next_batch_initial_punished_set == other.next_batch_initial_punished_set
    }
}

impl Eq for MacroHeader {}

impl Message for MacroHeader {
    const PREFIX: u8 = PREFIX_TENDERMINT_PROPOSAL;
}

impl fmt::Display for MacroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "#{}:MA:{}",
            self.block_number,
            self.hash().to_short_str(),
        )
    }
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.network.serialize_to_writer(writer)?;
        self.version.to_be_bytes().serialize_to_writer(writer)?;
        self.block_number.to_be_bytes().serialize(writer)?;
        self.round.to_be_bytes().serialize_to_writer(writer)?;

        self.timestamp.to_be_bytes().serialize_to_writer(writer)?;
        self.parent_hash.serialize_to_writer(writer)?;
        self.parent_election_hash.serialize_to_writer(writer)?;

        let interlink_hash = H::Builder::default()
            .chain(&self.interlink.serialize_to_vec())
            .finish();
        interlink_hash.serialize_to_writer(writer)?;

        self.seed.serialize_to_writer(writer)?;

        let extra_data_hash = H::Builder::default()
            .chain(&self.extra_data.serialize_to_vec())
            .finish();
        extra_data_hash.serialize_to_writer(writer)?;

        self.state_root.serialize_to_writer(writer)?;
        // Binds the validator set, the next batch's initial punished set and the body root.
        self.payload_commitment_root().serialize_to_writer(writer)?;
        self.history_root.serialize_to_writer(writer)?;

        // From `DIFF_ROOT_COMMITMENT` on, the diff root is part of the (signed) header hash.
        if self.version >= upgrades::v2::DIFF_ROOT_COMMITMENT {
            self.diff_root.serialize_to_writer(writer)?;
        }

        Ok(())
    }
}

/// The struct representing the body of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct MacroBody {
    /// The reward related transactions of this block.
    #[serialize_size(seq_max_elems = Policy::SLOTS as usize)]
    pub transactions: Vec<RewardTransaction>,
}

impl SerializeContent for MacroBody {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        let transactions_hash = self.transactions.serialize_to_vec().hash::<H>();
        transactions_hash.serialize_to_writer(writer)?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum IntoSlotsError {
    #[error("Body missing in macro block")]
    MissingBody,
    #[error("Not an election macro block")]
    NoElection,
}

#[cfg(test)]
mod test {
    use nimiq_bls::CompressedPublicKey as BlsCompressedPublicKey;
    use nimiq_keys::Ed25519PublicKey as SchnorrPublicKey;
    use nimiq_primitives::{
        policy::{upgrades, Policy},
        slots_allocation::{Validator, Validators},
    };

    use super::{MacroBlock, MacroHeader};

    #[test]
    fn size_well_below_msg_limit() {
        use nimiq_serde::SerializedMaxSize;
        assert!(
            2 * dbg!(MacroBlock::MAX_SIZE) + 16384
                <= dbg!(nimiq_network_interface::network::MIN_SUPPORTED_MSG_SIZE)
        );
    }

    /// Builds a single-band election header carrying `validators`, at the given protocol
    /// version. Only the fields that influence the header hash are set.
    fn election_header(validators: Validators, version: u16) -> MacroHeader {
        MacroHeader {
            version,
            validators: Some(validators),
            ..Default::default()
        }
    }

    /// Builds a valid single-band validator set owning all slots, with the given Ed25519
    /// signing key and a matching reward address. The BLS voting key is the same for every
    /// set so that only the metadata (signing key / address) differs between them.
    fn validators_with_signing_key(signing_key: SchnorrPublicKey) -> Validators {
        let validator = Validator::new(
            nimiq_keys::Address::from(&signing_key),
            BlsCompressedPublicKey::default(),
            signing_key,
            0..Policy::SLOTS,
        );
        Validators::new(vec![validator])
    }

    /// From protocol version 2 on, the macro-header hash must bind the validators'
    /// Ed25519 signing keys and reward addresses, so a peer cannot swap them on an
    /// election block while keeping the same hash (and a valid justification).
    /// Before version 2 the binding is dormant and the hash is unchanged.
    ///
    /// Regression test for the validator-metadata poisoning issue: the BLS voting-key
    /// tree (`Validators::hash`) does not commit signing keys or addresses.
    #[test]
    fn election_hash_binds_signing_key_from_version_2() {
        use nimiq_hash::{Blake2sHash, Hash};

        let honest = validators_with_signing_key(SchnorrPublicKey::from([0x11; 32]));
        let forged = validators_with_signing_key(SchnorrPublicKey::from([0x22; 32]));

        // The forge keeps the BLS voting-key tree identical; only the metadata differs.
        assert_eq!(
            honest.hash::<Blake2sHash>(),
            forged.hash::<Blake2sHash>(),
            "voting-key tree must be unaffected by the signing-key swap"
        );
        assert_ne!(
            honest.commitment_hash::<Blake2sHash>(),
            forged.commitment_hash::<Blake2sHash>(),
            "v2 commitment must distinguish the swapped signing key"
        );

        let activation = upgrades::v2::ELECTION_VALIDATOR_METADATA_COMMITMENT;

        // Before activation: binding dormant, the swap is invisible to the header hash (the bug).
        assert_eq!(
            election_header(honest.clone(), activation - 1).hash(),
            election_header(forged.clone(), activation - 1).hash(),
            "pre-activation the header hash must be unchanged (legacy hashing)"
        );

        // From activation on: the swap changes the authenticated header hash (the fix).
        assert_ne!(
            election_header(honest, activation).hash(),
            election_header(forged, activation).hash(),
            "from activation the header hash must bind the signing key / address"
        );
    }

    /// Validator sets with non-contiguous or overlapping slot bands are rejected, so the
    /// `get_band_from_slot` lookup can never panic on a crafted set (which would otherwise
    /// crash light nodes that adopt the set verbatim).
    #[test]
    fn non_contiguous_slot_bands_rejected() {
        let band = |start, end| {
            Validator::new(
                nimiq_keys::Address::default(),
                BlsCompressedPublicKey::default(),
                SchnorrPublicKey::default(),
                start..end,
            )
        };

        // Gap between the two bands (0..1 then 2..Policy::SLOTS leaves slot 1 uncovered).
        let gapped = Validators::new(vec![band(0, 1), band(2, Policy::SLOTS)]);
        assert!(gapped.validate_keys().is_err());

        // Overlapping bands.
        let overlapping = Validators::new(vec![band(0, 2), band(1, Policy::SLOTS)]);
        assert!(overlapping.validate_keys().is_err());
    }

    /// Verifies that invalid BLS voting keys are caught by verify() and
    /// voting_keys(), preventing the panic that would occur in hash_cached().
    ///
    /// Regression test for: untrusted peer announces an election macro block
    /// with an invalid BLS voting key. Before the fix, `hash_cached()` would
    /// panic via `voting_keys()` → `uncompress().unwrap()`.
    #[test]
    fn invalid_bls_voting_key_rejected_by_verify() {
        let invalid_compressed = BlsCompressedPublicKey {
            public_key: [0xFF; BlsCompressedPublicKey::SIZE],
        };

        let validator = Validator::new(
            nimiq_keys::Address::default(),
            invalid_compressed,
            SchnorrPublicKey::default(),
            0..Policy::SLOTS,
        );

        let validators = Validators::new(vec![validator]);

        // validate_keys() catches the invalid key.
        assert!(validators.validate_keys().is_err());

        // voting_keys() returns an error instead of panicking.
        assert!(validators.voting_keys().is_err());

        // MacroHeader::verify() rejects the block.
        let header = MacroHeader {
            validators: Some(validators),
            ..Default::default()
        };
        assert!(header.verify().is_err());
    }

    /// Verifies that Validators::hash() does not panic when called with invalid
    /// BLS voting keys. This is the root cause fix for the sync-path crash:
    /// hash() now uses raw compressed bytes directly instead of decompressing.
    #[test]
    fn hash_with_invalid_bls_key_does_not_panic() {
        use nimiq_hash::{Blake2sHash, Hash};

        let invalid_compressed = BlsCompressedPublicKey {
            public_key: [0xFF; BlsCompressedPublicKey::SIZE],
        };

        let validator = Validator::new(
            nimiq_keys::Address::default(),
            invalid_compressed,
            SchnorrPublicKey::default(),
            0..Policy::SLOTS,
        );

        let validators = Validators::new(vec![validator]);

        // This must not panic. Previously it would via voting_keys_g2().expect().
        let _hash: Blake2sHash = validators.hash();
    }

    /// Verifies that the raw-bytes hash produces the same output as the old
    /// decompress-reserialize path for valid keys. This ensures the macro block
    /// hash (which is consensus-critical and used for BLS signing) is unchanged.
    #[test]
    fn hash_output_unchanged_for_valid_keys() {
        use ark_ec::CurveGroup;
        use ark_serialize::CanonicalSerialize;
        use nimiq_hash::{Blake2sHash, Hash};
        use nimiq_keys::SecureGenerate;
        use nimiq_primitives::{
            merkle_tree::merkle_tree_construct, slots_allocation::PK_TREE_BREADTH,
        };

        let key_pair = nimiq_bls::KeyPair::generate_default_csprng();
        let compressed = key_pair.public_key.compress();

        let validator = Validator::new(
            nimiq_keys::Address::default(),
            compressed,
            SchnorrPublicKey::default(),
            0..Policy::SLOTS,
        );

        let validators = Validators::new(vec![validator]);

        // Compute hash via the new code path (raw bytes).
        let new_hash: Blake2sHash = validators.hash();

        // Compute hash via the old code path (decompress + reserialize).
        let public_keys = validators.voting_keys_g2().unwrap();
        let bytes: Vec<u8> = public_keys
            .iter()
            .flat_map(|pk| {
                let mut buffer = [0u8; 285];
                CanonicalSerialize::serialize_compressed(&pk.into_affine(), &mut buffer[..])
                    .unwrap();
                buffer.to_vec()
            })
            .collect();
        let mut inputs = Vec::new();
        for i in 0..PK_TREE_BREADTH {
            inputs.push(
                bytes[i * bytes.len() / PK_TREE_BREADTH..(i + 1) * bytes.len() / PK_TREE_BREADTH]
                    .to_vec(),
            );
        }
        let old_hash: Blake2sHash = merkle_tree_construct(inputs);

        assert_eq!(new_hash, old_hash);
    }

    /// Verifies that `Validators::hash()` does not panic when the validator set has
    /// an invalid total slot count, and that `validate_keys()` rejects it.
    ///
    /// Regression test for the election-macro-block crash (finding C1): on the
    /// sync/request paths (`Blockchain::push`, `LightBlockchain::push`,
    /// `push_history_sync`, `push_macro`) the block hash is computed *before*
    /// verification, so an unconditional assertion in `hash()` would crash the node
    /// on a single malformed block. Before the fix, `hash()` asserted
    /// `total_slots == Policy::SLOTS` and would panic here.
    #[test]
    fn hash_with_wrong_slot_total_does_not_panic() {
        use nimiq_hash::{Blake2sHash, Hash};
        use nimiq_keys::SecureGenerate;

        let compressed = nimiq_bls::KeyPair::generate_default_csprng()
            .public_key
            .compress();

        // Wrong total slot count: Policy::SLOTS + 1 is neither equal to
        // Policy::SLOTS nor a multiple of PK_TREE_BREADTH
        let validator = Validator::new(
            nimiq_keys::Address::default(),
            compressed,
            SchnorrPublicKey::default(),
            0..(Policy::SLOTS + 1),
        );
        let validators = Validators::new(vec![validator]);

        // Must not panic even though the slot total is invalid
        let _hash: Blake2sHash = validators.hash();

        // And the malformed set must be rejected during verification
        assert!(validators.validate_keys().is_err());
    }

    /// Verifies that `validate_keys()` rejects a validator set whose slot total
    /// would wrap around `u16` to exactly `Policy::SLOTS`.
    ///
    /// Regression test for the u16 accumulation overflow (finding C1, poc3): with
    /// overflow checks disabled (release profile), a plain `u16 += ` accumulator could
    /// wrap to `Policy::SLOTS` and let a malformed set pass validation while still
    /// mismatching the `usize` total computed in `hash()`, re-opening the crash on the
    /// gossip path. `validate_keys` now uses `checked_add`, which rejects on overflow.
    #[test]
    fn validate_keys_rejects_u16_wrapping_slot_total() {
        use nimiq_keys::SecureGenerate;

        let compressed = nimiq_bls::KeyPair::generate_default_csprng()
            .public_key
            .compress();

        // Two validators whose true total slot count is 65535 + 513 = 66048, which
        // wraps to exactly Policy::SLOTS (512) in u16 arithmetic
        let validator_a = Validator::new(
            nimiq_keys::Address::default(),
            compressed.clone(),
            SchnorrPublicKey::default(),
            0..u16::MAX, // 65535 slots
        );
        let validator_b = Validator::new(
            nimiq_keys::Address::default(),
            compressed,
            SchnorrPublicKey::default(),
            0..(Policy::SLOTS + 1), // 513 slots; 65535 + 513 = 66048 ≡ 512 (mod 2^16)
        );
        let validators = Validators::new(vec![validator_a, validator_b]);

        // Sanity check: this is exactly the case that wraps a u16 to Policy::SLOTS
        let true_total: usize = validators.iter().map(|v| v.num_slots() as usize).sum();
        assert_eq!(true_total, Policy::SLOTS as usize + (1 << 16));
        assert_eq!(true_total as u16, Policy::SLOTS);

        // `checked_add` must reject it (a plain `u16 +=` accumulator would have wrapped
        // to Policy::SLOTS and passed)
        assert!(validators.validate_keys().is_err());
    }
}
