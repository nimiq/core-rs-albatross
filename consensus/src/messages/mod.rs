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

    pub fn split_block(block: Block) -> (BlockHeaderMessage, BlockBodyMessage) {
        match block {
            Block::Macro(block) => {
                let header = BlockHeaderMessage::Macro {
                    header: block.header,
                    justification: block.justification.expect("Justification is required"),
                };
                let body = BlockBodyMessage {
                    header_hash: header.hash(),
                    body: BlockBody::Macro(block.body.expect("Body is required")),
                };
                (header, body)
            }
            Block::Micro(block) => {
                let header = BlockHeaderMessage::Micro {
                    header: block.header,
                    justification: block.justification.expect("Justification is required"),
                };
                let body = BlockBodyMessage {
                    header_hash: header.hash(),
                    body: BlockBody::Micro(block.body.expect("Body is required")),
                };
                (header, body)
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

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockBodyMessage {
    pub header_hash: Blake2bHash,
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
}

#[derive(Clone, Debug, Default)]
pub struct BlockBodyTopic;

impl Topic for BlockBodyTopic {
    type Item = BlockBodyMessage;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "block-body";
    const VALIDATE: bool = true;
}

/*
The consensus module uses the following messages:
200 RequestResponseMessage<RequestBlockHashes>
201 RequestResponseMessage<BlockHashes>
202 RequestResponseMessage<RequestEpoch>
203 RequestResponseMessage<Epoch>
*/

/// The max number of MacroChain requests per peer.
pub const MAX_REQUEST_RESPONSE_MACRO_CHAIN: u32 = 1000;
/// The max number of BatchSet requests per peer.
pub const MAX_REQUEST_RESPONSE_BATCH_SET: u32 = 1000;
/// The max number of HistoryChunk requests per peer.
pub const MAX_REQUEST_RESPONSE_HISTORY_CHUNK: u32 = 1000;
/// The max number of RequestBlock requests per peer.
pub const MAX_REQUEST_RESPONSE_BLOCK: u32 = 1000;
/// The max number of MissingBlocks requests per peer.
pub const MAX_REQUEST_RESPONSE_MISSING_BLOCKS: u32 = 1000;
/// The max number of RequestHead requests per peer.
pub const MAX_REQUEST_RESPONSE_HEAD: u32 = 1000;
/// The max number of Transactions proof requests per peer.
pub const MAX_REQUEST_TRANSACTIONS_PROOF: u32 = 1000;
/// The max number of Transactions proof requests per peer.
pub const MAX_REQUEST_TRANSACTIONS_BY_ADDRESS: u32 = 1000;
// The max number of Trie proof requests per peer.
pub const MAX_REQUEST_TRIE_PROOF: u32 = 1000;
/// The max number of Block proof requests per peer.
pub const MAX_REQUEST_BLOCKS_PROOF: u32 = 1000;
/// The max number of Subscribe to address requests per peer.
pub const MAX_REQUEST_SUBSCRIBE_BY_ADDRESS: u32 = 10;
/// The max number of Address notifications per peer.
pub const MAX_ADDRESS_NOTIFICATIONS: u32 = 100;

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
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_MACRO_CHAIN;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBatchSet {
    /// The hash of the macro block.
    pub hash: Blake2bHash,
}

impl RequestCommon for RequestBatchSet {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 202;
    type Response = Result<BatchSetInfo, BatchSetError>;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_BATCH_SET;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchSet {
    /// Verifying macro block
    pub macro_block: MacroBlock,
    /// Total history length at the height of the specified macro block
    pub history_len: SizeProof<Blake2bHash, HistoricTransaction>,
}

#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum BatchSetError {
    #[error("target hash not found")]
    TargetHashNotFound,
    #[error("the hash of a micro block was given")]
    MicroBlockGiven,
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
/// This message contains a chunk of the history.
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
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_HISTORY_CHUNK;
}

#[cfg(feature = "full")]
/// This message contains a chunk of the history.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryChunk {
    pub chunk: HistoryTreeChunk,
}

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
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_BLOCK;
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
    /// For requests with `Direction::Forward`, if not ordered that way any response may include blocks which are
    /// not necessary as they answer with respect to an older locator than they could have.
    /// For `Direction::Backward` it does have no effect.
    pub locators: Vec<Blake2bHash>,

    /// The direction the responder should take to search.
    ///  - Direction::Forward searches from the best locator to the target on the main chain.
    ///
    ///     That also implies that any block request using forward direction can only return main chain blocks.
    ///
    ///  - Direction::Backward searches from the target hash backwards until a locator (or macro block) is reached.
    ///
    ///     Using Backward makes retrieving inferior chains possible, but is more expensive for bigger
    ///     sets of anticipated returned blocks.
    pub direction: Direction,
}

impl RequestCommon for RequestMissingBlocks {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 209;
    type Response = Result<ResponseBlocks, ResponseBlocksError>;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_MISSING_BLOCKS;
}

/// Request the current blockchain head block hash.
#[derive(Clone, Debug, Deserialize, Serialize, SerializedMaxSize)]
pub struct RequestHead {}

#[derive(Clone, Debug, Deserialize, Serialize, SerializedMaxSize)]
pub struct ResponseHead {
    pub block_number: u32,
    pub block_hash: Blake2bHash,
}

impl RequestCommon for RequestHead {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 210;
    type Response = ResponseHead;
    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_HEAD;
}
test_max_req_size!(RequestHead, request_head_req_size, request_head_resp_size);

#[derive(Serialize, Deserialize)]
pub struct ResponseTransactionsProof {
    pub proof: HistoryTreeProof,
    pub block: Block,
}

#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseTransactionProofError {
    #[error("empty list of transactions given")]
    NoTransactionsProvided,
    #[error("requested txn proof from future block {0}, current head is {1}")]
    RequestedTxnProofFromFuture(u32, u32),
    #[error("requested txn proof that corresponds to a finalized epoch (block number {0}), should use the election block instead")]
    RequestedTxnProofFromFinalizedEpoch(u32),
    #[error("requested txn proof from finalized batch (block number {0}), should use a checkpoint block instead")]
    RequestedTxnProofFromFinalizedBatch(u32),
    #[error("block not found")]
    BlockNotFound,
    #[error("couldn't prove inclusion")]
    CouldntProveInclusion,
    #[error("transaction not found")]
    TransactionNotFound,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTransactionsProof {
    pub hashes: Vec<Blake2bHash>,
    pub block_number: Option<u32>,
}

impl RequestCommon for RequestTransactionsProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 213;
    type Response = Result<ResponseTransactionsProof, ResponseTransactionProofError>;
    const MAX_REQUESTS: u32 = MAX_REQUEST_TRANSACTIONS_PROOF;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTransactionReceiptsByAddress {
    pub address: Address,
    pub max: Option<u16>,
}

impl RequestCommon for RequestTransactionReceiptsByAddress {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 214;
    type Response = ResponseTransactionReceiptsByAddress;
    const MAX_REQUESTS: u32 = MAX_REQUEST_TRANSACTIONS_BY_ADDRESS;
}

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
    const MAX_REQUESTS: u32 = MAX_REQUEST_TRIE_PROOF;
}

/// Response to [`RequestTrieProof`].
#[derive(Serialize, Deserialize)]
pub struct ResponseTrieProof {
    // The accounts proof
    pub proof: TrieProof,
    // The hash of the block that was used to create the proof
    pub block_hash: Blake2bHash,
}

#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseTrieProofError {
    #[error("incomplete trie")]
    IncompleteTrie,
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestBlocksProof {
    pub election_head: u32,
    pub blocks: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct ResponseBlocksProof {
    pub proof: BlockInclusionProof,
}

#[derive(Clone, Debug, Deserialize, Error, Serialize)]
pub enum ResponseBlocksProofError {
    #[error("bad block number {0}")]
    BadBlockNumber(u32),
    #[error("unknown error")]
    #[serde(other)]
    Other,
}

impl RequestCommon for RequestBlocksProof {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 216;
    type Response = Result<ResponseBlocksProof, ResponseBlocksProofError>;
    const MAX_REQUESTS: u32 = MAX_REQUEST_BLOCKS_PROOF;
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
    const MAX_REQUESTS: u32 = MAX_REQUEST_SUBSCRIBE_BY_ADDRESS;
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
}
