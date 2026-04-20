//! Contains the message types and request/response handlers used for peer-to-peer communication during consensus and sync.
//!
//! This module groups together:
//! - Request types that a node can send to peers.
//! - Response types for the corresponding requests.
//! - Handler implementations for processing incoming requests.

use std::{
    fmt::{Debug, Display, Formatter},
    io::Write,
};

use nimiq_block::{
    Block, BlockBody, BlockInclusionProof, BlockType, MacroBlock, MacroHeader, MicroBlock,
    MicroHeader, MicroJustification, TendermintProof,
};
#[cfg(feature = "full")]
use nimiq_blockchain::HistoryTreeChunk;
use nimiq_blockchain_interface::Direction;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput, SerializeContent};
use nimiq_keys::Address;
use nimiq_macros::test_max_req_size;
use nimiq_mmr::mmr::proof::SizeProof;
use nimiq_network_interface::{
    network::Topic,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_proof::TrieProof};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};
use nimiq_transaction::{
    historic_transaction::HistoricTransaction, history_proof::HistoryTreeProof,
};
use thiserror::Error;

use crate::error::SubscribeToAddressesError;

mod handlers;

/// Message containing the header and justification of either a macro or micro block.
/// Used when broadcasting block headers over the network.
#[derive(Debug, Serialize, Deserialize)]
pub enum BlockHeaderMessage {
    Macro {
        header: MacroHeader,
        justification: TendermintProof,
    },
    Micro {
        header: MicroHeader,
        justification: MicroJustification,
    },
}

impl BlockHeaderMessage {
    pub fn ty(&self) -> BlockType {
        match self {
            BlockHeaderMessage::Macro { .. } => BlockType::Macro,
            BlockHeaderMessage::Micro { .. } => BlockType::Micro,
        }
    }

    pub fn body_root(&self) -> &Blake2sHash {
        match self {
            BlockHeaderMessage::Macro { header, .. } => &header.body_root,
            BlockHeaderMessage::Micro { header, .. } => &header.body_root,
        }
    }

    /// Splits a block into its header and body messages for transmission.
    pub fn split_block(block: Block) -> (BlockHeaderMessage, BlockBodyMessage) {
        match block {
            Block::Macro(block) => {
                let header_message = BlockHeaderMessage::Macro {
                    header: block.header,
                    justification: block.justification.expect("Justification is required"),
                };
                let body_message = BlockBodyMessage {
                    header_message_hash: header_message.hash(),
                    body: BlockBody::Macro(block.body.expect("Body is required")),
                };
                (header_message, body_message)
            }
            Block::Micro(block) => {
                let header_message = BlockHeaderMessage::Micro {
                    header: block.header,
                    justification: block.justification.expect("Justification is required"),
                };
                let body_message = BlockBodyMessage {
                    header_message_hash: header_message.hash(),
                    body: BlockBody::Micro(block.body.expect("Body is required")),
                };
                (header_message, body_message)
            }
        }
    }
}

impl Display for BlockHeaderMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockHeaderMessage::Macro { header, .. } => Display::fmt(header, f),
            BlockHeaderMessage::Micro { header, .. } => Display::fmt(header, f),
        }
    }
}

impl SerializeContent for BlockHeaderMessage {
    fn serialize_content<W: Write, H: HashOutput>(&self, writer: &mut W) -> std::io::Result<()> {
        self.serialize(writer).map(|_| ())
    }
}

impl From<BlockHeaderMessage> for Block {
    fn from(value: BlockHeaderMessage) -> Block {
        match value {
            BlockHeaderMessage::Macro {
                header,
                justification,
            } => Block::Macro(MacroBlock {
                header,
                body: None,
                justification: Some(justification),
            }),
            BlockHeaderMessage::Micro {
                header,
                justification,
            } => Block::Micro(MicroBlock {
                header,
                body: None,
                justification: Some(justification),
            }),
        }
    }
}

/// Message containing the body of a block, paired with the hash of its header message.
#[derive(Debug, Serialize, Deserialize)]
pub struct BlockBodyMessage {
    /// Hash of the corresponding [`BlockHeaderMessage`].
    ///
    /// This is used instead of the hash of the contained
    /// [`MacroHeader`]/[`MicroHeader`] since the header hash wouldn't capture
    /// the justification.
    pub header_message_hash: Blake2bHash,
    /// The block body of either a micro or macro block.
    pub body: BlockBody,
}

/// GossipSub topics to publish block headers and block bodies.
#[derive(Clone, Debug, Default)]
pub struct BlockHeaderTopic;

impl Topic for BlockHeaderTopic {
    type Item = BlockHeaderMessage;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block-header";
    const VALIDATE: bool = true;
    const MAX_MESSAGES: u32 = 20;
}

/// GossipSub topic for broadcasting block body messages.
#[derive(Clone, Debug, Default)]
pub struct BlockBodyTopic;

impl Topic for BlockBodyTopic {
    type Item = BlockBodyMessage;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block-body";
    const VALIDATE: bool = true;
    const MAX_MESSAGES: u32 = BlockHeaderTopic::MAX_MESSAGES;
}

/*
The consensus module uses the following messages:
200 RequestResponseMessage<RequestBlockHashes>
201 RequestResponseMessage<BlockHashes>
202 RequestResponseMessage<RequestEpoch>
203 RequestResponseMessage<Epoch>
*/

/// Part of [`MacroChain`].
///
/// Metadata of the last checkpoint block in the response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Block height of the checkpoint block.
    pub block_number: u32,
    /// Hash of the checkpoint block.
    pub hash: Blake2bHash,
}

/// Response to [`RequestMacroChain`].
#[derive(Clone, Serialize, Deserialize)]
pub struct MacroChain {
    /// The hashes of the macro blocks starting at one of the locators of the request.
    pub epochs: Vec<Blake2bHash>,
    /// Under certain circumstances, the metadata about the last checkpoint block.
    pub checkpoint: Option<Checkpoint>,
}

/// Error response to [`RequestMacroChain`]
#[derive(Clone, Debug, Deserialize, Error, Serialize, SerializedMaxSize)]
pub enum MacroChainError {
    /// The locators of the request could not be found.
    #[error("unknown locators")]
    UnknownLocators,
    /// Request contains too many locators.
    #[error("too many locators")]
    TooManyLocators,
    /// Error not understood by the recipient, is never sent explicitly.
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

impl Debug for MacroChain {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("MacroChain");
        dbg.field("num_epochs", &self.epochs.len());
        if !self.epochs.is_empty() {
            let first = self.epochs.first().unwrap();
            let last = self.epochs.last().unwrap();
            dbg.field("first_epoch", &first);
            dbg.field("last_epoch", &last);
        }
        dbg.field("checkpoint", &self.checkpoint);
        dbg.finish()
    }
}

/// Request macro block hashes starting from a set of locators.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMacroChain {
    /// Blocks known by the requester.
    pub locators: Vec<Blake2bHash>,
    /// Limit of epochs to send in response.
    pub max_epochs: u16,
}

impl RequestCommon for RequestMacroChain {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 200;
    type Response = Result<MacroChain, MacroChainError>;
    const MAX_REQUESTS: u32 = 20;
}

/// Request a batch set for the given macro block hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBatchSet {
    /// The hash of the macro block.
    pub hash: Blake2bHash,
}

impl RequestCommon for RequestBatchSet {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 202;
    type Response = Result<BatchSetInfo, BatchSetError>;
    const MAX_REQUESTS: u32 = 100;
}

/// A batch set containing a macro block and the total history length at its height.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchSet {
    /// Verifying macro block
    pub macro_block: MacroBlock,
    /// Total history length at the height of the specified macro block
    pub history_len: SizeProof<Blake2bHash, HistoricTransaction>,
}

/// Errors that can occur when processing a [`RequestBatchSet`].
#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum BatchSetError {
    #[error("target hash not found")]
    TargetHashNotFound,
    #[error("the hash of a micro block was given")]
    MicroBlockGiven,
    #[error("couldn't produce history size proof")]
    CouldntProduceHistorySizeProof,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

/// This message contains a macro block and the number of historic transactions (transitions)
/// within this epoch.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct BatchSetInfo {
    pub election_macro_block: Option<MacroBlock>,
    pub batch_sets: Vec<BatchSet>,
}

impl BatchSetInfo {
    pub fn total_history_len(&self) -> u64 {
        self.batch_sets
            .last()
            .map(|batch_set| batch_set.history_len.size())
            .unwrap_or(0)
    }

    pub fn final_macro_block(&self) -> &MacroBlock {
        self.election_macro_block.as_ref().unwrap_or_else(|| {
            &self
                .batch_sets
                .last()
                .expect("BatchSet must not be empty")
                .macro_block
        })
    }
}

impl Debug for BatchSetInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("BatchSetInfo");
        if let Some(block) = &self.election_macro_block {
            debug_struct
                .field("election_epoch_number", &block.epoch_number())
                .field("election_block_number", &block.block_number());
        }
        debug_struct.field("total_history_len", &self.total_history_len());
        debug_struct.field("batch_sets_len", &self.batch_sets.len());
        debug_struct.finish()
    }
}

#[cfg(feature = "full")]
/// Request to get a specific chunk of the history for a given block and epoch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryChunk {
    pub epoch_number: u32,
    pub block_number: u32,
    pub chunk_index: u64,
}

#[cfg(feature = "full")]
impl RequestCommon for RequestHistoryChunk {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 204;
    type Response = Result<HistoryChunk, HistoryChunkError>;
    const MAX_REQUESTS: u32 = 500;
}

#[cfg(feature = "full")]
/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: HistoryTreeChunk,
}

/// Error returned when [`RequestHistoryChunk`] fails.
#[cfg(feature = "full")]
#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum HistoryChunkError {
    #[error("couldn't produce proof")]
    CouldntProduceProof,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

/// Request a specific block by hash.
///
/// Can optionally omit the body if it's a micro block.
#[derive(Clone, Debug, Deserialize, Serialize, SerializedMaxSize)]
pub struct RequestBlock {
    /// The hash of the block that is requested.
    pub hash: Blake2bHash,
    /// Whether to include the body.
    pub include_body: bool,
}

/// Error response for [`RequestBlock`].
#[derive(Clone, Debug, Deserialize, Error, Serialize, SerializedMaxSize)]
pub enum BlockError {
    /// Block hash unknown to the responder.
    #[error("target hash not found")]
    TargetHashNotFound,
    /// Block hash response does not correspond to the requested hash.
    #[error("target hash response hash mismatch")]
    ResponseHashMismatch,
    /// Error not understood by the recipient, is never sent explicitly.
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

impl RequestCommon for RequestBlock {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 207;
    type Response = Result<Block, BlockError>;
    const MAX_REQUESTS: u32 = 200;
}
test_max_req_size!(
    RequestBlock,
    request_block_req_size,
    request_block_resp_size,
);

/// Response to [`RequestMissingBlocks`].
#[derive(Clone, Deserialize, Serialize)]
pub struct ResponseBlocks {
    // TODO: Set to sensible limit (2 * BATCH_SIZE for example).
    pub blocks: Vec<Block>,
}

/// Error response to [`RequestMissingBlocks`].
#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseBlocksError {
    /// The target block is not on the responder's main chain, it cannot
    /// fulfill the request.
    #[error("target block not on main chain")]
    TargetBlockNotOnMainChain,
    /// The target block is unknown to the responder.
    #[error("target hash not found")]
    TargetHashNotFound,
    /// The locators are unknown to the responder.
    #[error("unknown locators")]
    UnknownLocators,
    /// Request contains too many locators.
    #[error("too many locators")]
    TooManyLocators,
    /// The responder couldn't get the intermediate blocks due to another
    /// error.
    #[error("failed to get blocks")]
    FailedToGetBlocks,
    /// Error not understood by the recipient, is never sent explicitly.
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

impl Debug for ResponseBlocks {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("ResponseBlocks");
        dbg.field("length", &self.blocks.len());
        if !self.blocks.is_empty() {
            let first = self.blocks.first().unwrap();
            let last = self.blocks.last().unwrap();
            dbg.field("first_block", &first.block_number());
            dbg.field("last_block", &last.block_number());
        }
        dbg.finish()
    }
}

/// Request "missing" blocks, from a set of locators to a target hash.
/// Requests of the sort must choose a direction to facilitate requesting of inferior chain blocks as well.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMissingBlocks {
    /// Target block hash.
    pub target_hash: Blake2bHash,
    /// Whether to include block bodies.
    pub include_body: bool,
    /// Set of hashes of blocks known to the requester, possible starts of the
    /// answer chain.
    ///
    /// The locator ordering should be from newest to oldest block.
    ///
    /// For `Direction::Forward`, locators must be ordered from newest to oldest.
    /// Otherwise, the responder may select an older locator as the starting point, returning unnecessary blocks.
    /// The order does not matter for `Direction::Backward`.
    pub locators: Vec<Blake2bHash>,

    /// The direction the responder should take to search.
    ///  - Direction::Forward searches from the best locator to the target on the main chain.
    ///
    ///    That also implies that any block request using forward direction can only return main chain blocks.
    ///
    ///  - Direction::Backward searches from the target hash backwards until a locator (or macro block) is reached.
    ///
    ///    Using Backward makes retrieving inferior chains possible, but is more expensive for bigger
    ///    sets of anticipated returned blocks.
    pub direction: Direction,
}

impl RequestCommon for RequestMissingBlocks {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 209;
    type Response = Result<ResponseBlocks, ResponseBlocksError>;
    const MAX_REQUESTS: u32 = 200;
}

/// Request the current blockchain head block hash.
#[derive(Clone, Debug, Deserialize, Serialize, SerializedMaxSize)]
pub struct RequestHead {}

/// Indicates the responder’s latest known state.
#[derive(Clone, Debug, Deserialize, Serialize, SerializedMaxSize)]
pub struct ResponseHead {
    pub block_number: u32,
    pub block_hash: Blake2bHash,
    pub macro_hash: Blake2bHash,
    pub election_hash: Blake2bHash,
}

impl RequestCommon for RequestHead {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 210;
    type Response = ResponseHead;
    const MAX_REQUESTS: u32 = 50;
}
test_max_req_size!(RequestHead, request_head_req_size, request_head_resp_size);

/// Response containing a proof of transaction inclusion.
#[derive(Serialize, Deserialize)]
pub struct ResponseTransactionsProof {
    /// The history MMR proof
    pub proof: HistoryTreeProof,
    /// Block used to generate the history tree proof.
    ///
    /// The block is determined by internal logic (see the `prove_txns_with_block_number`
    /// function in the implementation of [`RequestTransactionsProof`])
    pub block: Block,
}

/// Errors returned when generating a transaction inclusion proof in response to a [`RequestTransactionsProof`].
#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseTransactionProofError {
    #[error("empty list of transactions given")]
    NoTransactionsProvided,
    #[error("too many transactions given")]
    TooManyTransactionsProvided,
    #[error("requested txn proof from future block {0}, current head is {1}")]
    RequestedTxnProofFromFuture(u32, u32),
    #[error("requested txn proof that corresponds to a finalized epoch (block number {0}), should use the election block instead")]
    RequestedTxnProofFromFinalizedEpoch(u32),
    #[error("requested txn proof from finalized batch (block number {0}), should use a checkpoint block instead")]
    RequestedTxnProofFromFinalizedBatch(u32),
    #[error("Block not found: {0}")]
    BlockNotFound(u32),
    #[error("couldn't prove inclusion")]
    CouldntProveInclusion,
    #[error("transaction not found")]
    TransactionNotFound,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

/// Request a proof of inclusion for one or more transactions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTransactionsProof {
    pub hashes: Vec<Blake2bHash>,
    pub block_number: Option<u32>,
}

impl RequestCommon for RequestTransactionsProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 213;
    type Response = Result<ResponseTransactionsProof, ResponseTransactionProofError>;
    const MAX_REQUESTS: u32 = 1000;
}

/// Returns the latest transactions for a given address. All the transactions
/// where the given address is listed as a recipient or as a sender are considered. Reward
/// transactions are also returned. It has an option to specify the maximum number of transactions
/// to fetch. It has also an option to retrieve transactions before a given transaction hash.
/// If this hash is not found or does not belong to this address, it will return an empty list.
/// The transactions are returned in descending order, meaning the latest transaction is the first.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTransactionReceiptsByAddress {
    pub address: Address,
    pub max: Option<u16>,
    pub start_at: Option<Blake2bHash>,
}

impl RequestCommon for RequestTransactionReceiptsByAddress {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 214;
    type Response = ResponseTransactionReceiptsByAddress;
    const MAX_REQUESTS: u32 = 20;
}

/// Response containing a list of transactions related to a specific address.
#[derive(Serialize, Deserialize)]
pub struct ResponseTransactionReceiptsByAddress {
    /// Tuples of `(transaction_hash, block_number)`
    pub receipts: Vec<(Blake2bHash, u32)>,
}

/// Request a proof for the values corresponding to some keys or their absence from the accounts trie.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTrieProof {
    /// Addresses for which the accounts trie proof is requested for
    pub keys: Vec<KeyNibbles>, //-> Accounts
}

impl RequestCommon for RequestTrieProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 215;
    type Response = Result<ResponseTrieProof, ResponseTrieProofError>;
    const MAX_REQUESTS: u32 = 50;
}

/// Response to [`RequestTrieProof`].
#[derive(Serialize, Deserialize)]
pub struct ResponseTrieProof {
    // The accounts proof
    pub proof: TrieProof,
    // The hash of the block that was used to create the proof
    pub block_hash: Blake2bHash,
}

/// Errors returned in response to a [`RequestTrieProof`].
#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseTrieProofError {
    #[error("incomplete trie")]
    IncompleteTrie,
    #[error("too many keys")]
    TooManyKeys,
    #[error("duplicate keys")]
    DuplicateKeys,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

/// Request a block inclusion proof for a set of block numbers relative to a given election head.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlocksProof {
    pub election_head: u32,
    pub blocks: Vec<u32>,
}

/// Response containing a proof of inclusion for a set of blocks.
#[derive(Serialize, Deserialize)]
pub struct ResponseBlocksProof {
    pub proof: BlockInclusionProof,
}

/// Errors returned when generating a blocks proof in response to a [`RequestBlocksProof`].
#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseBlocksProofError {
    #[error("bad block number {0}")]
    BadBlockNumber(u32),
    #[error("too many blocks")]
    TooManyBlocks,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

impl RequestCommon for RequestBlocksProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 216;
    type Response = Result<ResponseBlocksProof, ResponseBlocksProofError>;
    const MAX_REQUESTS: u32 = 50;
}

/// Operations supported for the transaction address subscription
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum AddressSubscriptionOperation {
    /// Subscribe to some interesting addresses, to start receiving notifications about those addresses.
    Subscribe,
    /// Unsubscribe from some specific addresses, to stop receiving notifications from those addresses
    Unsubscribe,
}

/// This request is used to subscribe or unsubscribe from specific addresses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestSubscribeToAddress {
    /// The type of operation that is needed by the peer
    pub operation: AddressSubscriptionOperation,
    /// The addresses which are interesting to the peer
    pub addresses: Vec<Address>,
}

impl RequestCommon for RequestSubscribeToAddress {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 217;
    type Response = Result<(), SubscribeToAddressesError>;
    const MAX_REQUESTS: u32 = 10;
}

/// Different kind of events that could generate notifications
#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
#[repr(u8)]
pub enum NotificationEvent {
    /// A new block was pushed into the chain.
    BlockchainExtend,
}

/// Interesting Addresses Notifications:
/// A collection of transaction receipts that might be interesting for some peer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddressNotification {
    /// The Event that generated this notification
    pub event: NotificationEvent,
    /// Tuples of `(transaction_hash, block_number)`
    pub receipts: Vec<(Blake2bHash, u32)>,
}

/// Topic used to notify peers about transaction addresses they are subscribed to
/// The final notification is sent over a subtopic derived from this one, which is specific to each peer
#[derive(Clone, Debug, Default)]
pub struct AddressSubscriptionTopic;

impl Topic for AddressSubscriptionTopic {
    type Item = AddressNotification;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "address-subscription";
    const VALIDATE: bool = false;
    const MAX_MESSAGES: u32 = 10_000;
}
