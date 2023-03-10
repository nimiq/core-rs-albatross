use serde::ser::SerializeStruct;
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use beserial::Serialize;
use nimiq_primitives::policy::Policy;

/// JSON-compatible and human-readable format of blocks.
#[derive(Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainBlock {
    /// The block's unique hash, used as its identifier, in HEX format.
    pub hash: String,
    /// The block's on-chain size, in bytes.
    pub size: u32,
    /// The block's block height, also called block number.
    pub height: u32,
    /// The batch number that the block is in.
    pub batch: u32,
    /// The epoch number that the block is in.
    pub epoch: u32,
    /// The timestamp of the block. It follows the Unix time and has millisecond precision.
    pub timestamp: u64,

    /// The protocol version that this block is valid for.
    pub version: u16,
    /// The hash of the header of the immediately preceding block (either micro or macro), in HEX format.
    pub prev_hash: String,
    /// The seed of the block. This is the BLS signature of the seed of the immediately preceding
    /// block (either micro or macro) using the validator key of the block producer.
    pub seed: String,
    /// The extra data of the block, in HEX format. Up to 32 raw bytes.
    ///
    /// In the genesis block, it encodes the initial supply as a big-endian `u64`.
    ///
    /// No planned use otherwise.
    pub extra_data: String,
    /// The root of the Merkle tree of the blockchain state, in HEX format. It acts as a commitment to the state.
    pub state_hash: String,
    /// The root of the Merkle tree of the body, in HEX format. It acts as a commitment to the body.
    pub body_hash: String,
    /// A Merkle root over all of the transactions that happened in the current epoch, in HEX format.
    pub history_hash: String,

    #[serde(flatten)]
    pub additional_fields: PlainBlockAdditionalFields,
}

// Manually implement serde::Serialize trait to ensure struct is serialized into a JS Object and not a Map.
//
// Unfortunately, serde cannot serialize a struct that includes a #[serde(flatten)] annotation into an Object,
// and the Github issue for it is closed as "wontfix": https://github.com/serde-rs/serde/issues/1346
impl serde::Serialize for PlainBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let common_fields_length = 13 + /* type */ 1;

        let length = match &self.additional_fields {
            PlainBlockAdditionalFields::Macro { .. } => common_fields_length + 3,
            PlainBlockAdditionalFields::Micro { .. } => common_fields_length, // + 0
        };

        let mut plain = serializer.serialize_struct("PlainBlock", length)?;
        plain.serialize_field("hash", &self.hash)?;
        plain.serialize_field("size", &self.size)?;
        plain.serialize_field("height", &self.height)?;
        plain.serialize_field("batch", &self.batch)?;
        plain.serialize_field("epoch", &self.epoch)?;
        plain.serialize_field("timestamp", &self.timestamp)?;

        plain.serialize_field("version", &self.version)?;
        plain.serialize_field("prevHash", &self.prev_hash)?;
        plain.serialize_field("seed", &self.seed)?;
        plain.serialize_field("extraData", &self.extra_data)?;
        plain.serialize_field("stateHash", &self.state_hash)?;
        plain.serialize_field("bodyHash", &self.body_hash)?;
        plain.serialize_field("historyHash", &self.history_hash)?;

        match &self.additional_fields {
            PlainBlockAdditionalFields::Macro {
                is_election_block,
                round,
                prev_election_hash,
            } => {
                plain.serialize_field("type", "macro")?;
                plain.serialize_field("isElectionBlock", is_election_block)?;
                plain.serialize_field("round", round)?;
                plain.serialize_field("prevElectionHash", prev_election_hash)?;
            }
            PlainBlockAdditionalFields::Micro {} => {
                plain.serialize_field("type", "micro")?;
            }
        }

        plain.end()
    }
}

impl PlainBlock {
    /// Creates a PlainBlock struct that can be serialized to JS from a native [nimiq_block::Block].
    pub fn from_block(block: &nimiq_block::Block) -> Self {
        let block_number = block.block_number();

        Self {
            hash: block.hash().to_hex(),
            size: block.serialized_size() as u32,
            height: block_number,
            batch: Policy::batch_at(block_number),
            epoch: Policy::epoch_at(block_number),
            timestamp: block.timestamp(),

            version: block.version(),
            prev_hash: block.parent_hash().to_hex(),
            seed: block.seed().to_string(),
            extra_data: hex::encode(block.extra_data()),
            state_hash: block.state_root().to_hex(),
            body_hash: block.body_root().to_hex(),
            history_hash: block.history_root().to_hex(),

            additional_fields: PlainBlockAdditionalFields::from_block(block),
        }
    }
}

#[derive(Tsify)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PlainBlockAdditionalFields {
    #[serde(rename_all = "camelCase")]
    Macro {
        /// If true, this macro block is an election block finalizing an epoch.
        is_election_block: bool,
        /// The round number this block was proposed in.
        round: u32,
        /// The hash of the header of the preceding election macro block, in HEX format.
        prev_election_hash: String,
    },
    #[serde(rename_all = "camelCase")]
    Micro {},
}

impl PlainBlockAdditionalFields {
    pub fn from_block(block: &nimiq_block::Block) -> Self {
        match block {
            nimiq_block::Block::Macro(block) => PlainBlockAdditionalFields::Macro {
                is_election_block: block.is_election_block(),
                round: block.round(),
                prev_election_hash: block.header.parent_election_hash.to_hex(),
            },
            nimiq_block::Block::Micro(_block) => PlainBlockAdditionalFields::Micro {},
        }
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainBlock")]
    pub type PlainBlockType;
}
