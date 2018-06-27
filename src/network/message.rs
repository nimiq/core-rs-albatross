use beserial::{Serialize, SerializeWithLength, Deserialize, DeserializeWithLength, ReadBytesExt, WriteBytesExt};
use std::io;
use consensus::base::{Subscription};
use consensus::base::account::tree::{AccountsProof};
use consensus::base::block::{Block, BlockHeader};
use consensus::base::transaction::{Transaction};
use consensus::base::primitive::crypto::{PublicKey, Signature};
use consensus::base::primitive::hash::Blake2bHash;
use network::address::{PeerAddress, PeerId};

#[derive(Clone,Copy,Debug,Eq,PartialEq,Ord,PartialOrd,Serialize,Deserialize)]
#[repr(u8)]
enum MessageType {
    Version = 0,
    Inv = 1,
    GetData = 2,
    GetHeader = 3,
    NotFound = 4,
    GetBlocks = 5,
    Block = 6,
    Header = 7,
    Tx = 8,
    Mempool = 9,
    Reject = 10,
    Subscribe = 11,

    Addr = 20,
    GetAddr = 21,
    Ping = 22,
    Pong = 23,

    Signal = 30,

    GetChainProof = 40,
    ChainProof = 41,
    GetAccountsProof = 42,
    AccountsProof = 43,
    GetAccountsTreeChunk = 44,
    AccountsTreeChunk = 45,
    GetTransactionsProof = 46,
    TransactionsProof = 47,
    GetTransactionReceipts = 48,
    TransactionReceipts = 49,
    GetBlockProof = 50,
    BlockProof = 51,

    GetHead = 60,
    Head = 61,

    VerAck = 90
}

enum Message {
    Version(VersionMessage),
    Inv(Vec<InvVector>),
    GetData(Vec<InvVector>),
    GetHeader(Vec<InvVector>),
    NotFound(Vec<InvVector>),
    Block(Block),
    Header(BlockHeader),
    Tx(TxMessage),
    GetBlocks(GetBlocksMessage),
    Mempool,
    Reject(RejectMessage),
    Subscribe(Subscription),

    Addr(AddrMessage),
    GetAddr(GetAddrMessage),
    Ping(/*nonce*/ u32),
    Pong(/*nonce*/ u32),

    VerAck(VerAckMessage)
}

const MAGIC: u32 = 0x42042042;

impl Deserialize for Message {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let magic: u32 = Deserialize::deserialize(reader)?;
        if magic != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Wrong magic byte"));
        }
        let ty: u8 = Deserialize::deserialize(reader)?;
        let checksum: u32 = Deserialize::deserialize(reader)?;
        let message: Message = match ty {
            ty if ty == MessageType::Version as u8 => {
                Message::Version(Deserialize::deserialize(reader)?)
            },
            _ => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message type"));
            }
        };
        return Ok(message);
    }
}

#[derive(Serialize,Deserialize)]
struct VersionMessage {
    version: u32,
    peer_address: PeerAddress,
    genesis_hash: Blake2bHash,
    head_hash: Blake2bHash,
    challenge_nonce: u32
}

#[derive(Clone,Copy,Debug,Eq,PartialEq,Ord,PartialOrd,Serialize,Deserialize)]
#[repr(u32)]
enum InvVectorType {
    Error = 0,
    Transaction = 1,
    Block = 2
}

#[derive(Serialize,Deserialize)]
struct InvVector {
    ty: InvVectorType,
    hash: Blake2bHash
}

#[derive(Serialize,Deserialize)]
struct TxMessage {
    transaction: Transaction,
    accounts_proof: Option<AccountsProof>
}

#[derive(Clone,Copy,Debug,Eq,PartialEq,Ord,PartialOrd,Serialize,Deserialize)]
#[repr(u8)]
enum GetBlocksDirection {
    Forward = 0,
    Backward = 1
}

#[derive(Serialize,Deserialize)]
struct GetBlocksMessage {
    #[beserial(len_type(u16))]
    locators: Vec<Blake2bHash>,
    max_inv_size: u16,
    direction: GetBlocksDirection
}

#[derive(Clone,Copy,Debug,Eq,PartialEq,Ord,PartialOrd,Serialize,Deserialize)]
#[repr(u8)]
enum RejectMessageCode {
    Malformed = 0x01,
    Invalid = 0x10,
    Obsolete = 0x11,
    Double = 0x12,
    Dust = 0x41,
    InsufficientFee = 0x42
}

struct RejectMessage {
    message_type: MessageType,
    code: RejectMessageCode,
    reason: String, // TODO
//    #[beserial(len_type(u16))]
    extra_data: Vec<u8>
}

#[derive(Serialize,Deserialize)]
struct AddrMessage {
    #[beserial(len_type(u16))]
    addresses: Vec<PeerAddress>
}

#[derive(Serialize,Deserialize)]
struct AccountsProofMessage {
    block_hash: Blake2bHash,
    accounts_proof: Option<AccountsProof>
}

#[derive(Serialize,Deserialize)]
struct GetAddrMessage {
    protocol_mask: u8,
    service_mask: u32,
    max_results: u16 // TODO this is optional right now but is always set
}

#[derive(Serialize,Deserialize)]
struct VerAckMessage {
    public_key: PublicKey,
    signature: Signature
}
