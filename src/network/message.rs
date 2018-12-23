use std::io;
use std::io::Read;

use rand::OsRng;
use rand::Rng;

use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, SerializingError, uvar, WriteBytesExt};
use byteorder::{BigEndian, ByteOrder};
use parking_lot::RwLock;

use crate::consensus::base::account::tree::AccountsProof;
use crate::consensus::base::block::{Block, BlockHeader};
use crate::consensus::base::primitive::crypto::{PublicKey, Signature, KeyPair};
use crate::consensus::base::primitive::hash::Blake2bHash;
use crate::consensus::base::Subscription;
use crate::consensus::base::transaction::Transaction;
use crate::network::address::{PeerAddress, PeerId};
use crate::utils::crc::Crc32Computer;
use crate::utils::services::ServiceFlags;
use crate::network::ProtocolFlags;
use crate::utils::version;
use crate::utils::observer::PassThroughNotifier;
use crate::consensus::base::primitive::hash::Hash;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u64)]
#[beserial(uvar)]
pub enum MessageType {
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

    VerAck = 90,
}

#[derive(Clone, Debug)]
pub enum Message {
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

    VerAck(VerAckMessage),

    GetHead,
    Head(BlockHeader),
}

impl Message {
    pub fn ty(&self) -> MessageType {
        match self {
            Message::Version(_) => MessageType::Version,
            Message::Inv(_) => MessageType::Inv,
            Message::GetData(_) => MessageType::GetData,
            Message::GetHeader(_) => MessageType::GetHeader,
            Message::NotFound(_) => MessageType::NotFound,
            Message::Block(_) => MessageType::Block,
            Message::Header(_) => MessageType::Header,
            Message::Tx(_) => MessageType::Tx,
            Message::GetBlocks(_) => MessageType::GetBlocks,
            Message::Mempool => MessageType::Mempool,
            Message::Reject(_) => MessageType::Reject,
            Message::Subscribe(_) => MessageType::Subscribe,
            Message::Addr(_) => MessageType::Addr,
            Message::GetAddr(_) => MessageType::GetAddr,
            Message::Ping(_) => MessageType::Ping,
            Message::Pong(_) => MessageType::Pong,
            Message::VerAck(_) => MessageType::VerAck,
            Message::GetHead => MessageType::GetHead,
            Message::Head(_) => MessageType::Head,
        }
    }

    pub fn peek_length(buffer: &[u8]) -> usize {
        // FIXME: support for message types > 253 is pending (it changes the length position in the chunk).
        // The magic number is 4 bytes and the type is 1 byte, so we want to start at the 6th byte (index 5), and the length field is 4 bytes.
        BigEndian::read_u32(&buffer[5..9]) as usize
    }
}

const MAGIC: u32 = 0x42042042;

impl Deserialize for Message {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        pub struct ReaderComputeCrc32<'a, T: 'a + ReadBytesExt> {
            reader: &'a mut T,
            crc32: Crc32Computer,
            nth_element: u32
        }

        impl<'a, T: ReadBytesExt> ReaderComputeCrc32<'a, T> {
            pub fn new(reader: &'a mut T) -> ReaderComputeCrc32<T> {
                ReaderComputeCrc32 {
                    reader,
                    crc32: Crc32Computer::default(),
                    nth_element: 0
                }
            }
        }

        impl<'a, T: ReadBytesExt> io::Read for ReaderComputeCrc32<'a, T> {
            fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
                let size = self.reader.read(buf)?;
                if size > 0 {
                    self.nth_element += 1;

                    // checksum is the 4th element. Use 0's instead for checksum computation.
                    if self.nth_element != 4 {
                        self.crc32.update(&buf[..size]);
                    } else {
                        self.crc32.update(&[0, 0, 0, 0]);
                    }
                }
                return Ok(size);
            }
        }


        let mut crc32_reader = ReaderComputeCrc32::new(reader);
        let magic: u32 = Deserialize::deserialize(&mut crc32_reader)?;
        if magic != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Wrong magic byte").into());
        }

        let ty: MessageType = Deserialize::deserialize(&mut crc32_reader)?;
        let length: u32 = Deserialize::deserialize(&mut crc32_reader)?;
        let checksum: u32 = Deserialize::deserialize(&mut crc32_reader)?;

        let message: Message = match ty {
            MessageType::Version => Message::Version(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Inv => Message::Inv(DeserializeWithLength::deserialize::<u16, ReaderComputeCrc32<R>>(&mut crc32_reader)?),
            MessageType::GetData => Message::GetData(DeserializeWithLength::deserialize::<u16, ReaderComputeCrc32<R>>(&mut crc32_reader)?),
            MessageType::GetHeader => Message::GetHeader(DeserializeWithLength::deserialize::<u16, ReaderComputeCrc32<R>>(&mut crc32_reader)?),
            MessageType::NotFound => Message::NotFound(DeserializeWithLength::deserialize::<u16, ReaderComputeCrc32<R>>(&mut crc32_reader)?),
            MessageType::Block => Message::Block(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Header => Message::Header(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Tx => Message::Tx(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetBlocks => Message::GetBlocks(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Mempool => Message::Mempool,
            MessageType::Reject => Message::Reject(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Subscribe => Message::Subscribe(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Addr => Message::Addr(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetAddr => Message::GetAddr(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Ping => Message::Ping(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Pong => Message::Pong(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::VerAck => Message::VerAck(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetHead => Message::GetHead,
            MessageType::Head => Message::Head(Deserialize::deserialize(&mut crc32_reader)?),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Message deserialization: Unimplemented message type").into())  // FIXME remove default case
        };

        // XXX Consume any leftover bytes in the message before computing the checksum.
        // This is consistent with the JS implementation.
        crc32_reader.read_to_end(&mut Vec::new()).unwrap();

        let crc_comp = crc32_reader.crc32.result();
        if crc_comp != checksum {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Message deserialization: Bad checksum").into());
        }

        return Ok(message);
    }
}

impl Serialize for Message {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size= 0;
        let serialized_size: u32 = self.serialized_size() as u32;
        let mut v = Vec::with_capacity(serialized_size as usize);
        size += MAGIC.serialize(&mut v)?;
        size += self.ty().serialize(&mut v)?;
        size += serialized_size.serialize(&mut v)?;
        let checksum_start = v.len();
        size += 0u32.serialize(&mut v)?; // crc32 placeholder

        size += match self {
            Message::Version(version_message) => version_message.serialize(&mut v)?,
            Message::Inv(inv_vector) => inv_vector.serialize::<u16, Vec<u8>>(&mut v)?,
            Message::GetData(inv_vector) => inv_vector.serialize::<u16, Vec<u8>>(&mut v)?,
            Message::GetHeader(inv_vector) => inv_vector.serialize::<u16, Vec<u8>>(&mut v)?,
            Message::NotFound(inv_vector) => inv_vector.serialize::<u16, Vec<u8>>(&mut v)?,
            Message::Block(block_message) => block_message.serialize(&mut v)?,
            Message::Header(header_message) => header_message.serialize(&mut v)?,
            Message::Tx(tx_message) => tx_message.serialize(&mut v)?,
            Message::GetBlocks(get_blocks_message) => get_blocks_message.serialize(&mut v)?,
            Message::Mempool => 0,
            Message::Reject(reject_message) => reject_message.serialize(&mut v)?,
            Message::Subscribe(subscribe_message) => subscribe_message.serialize(&mut v)?,
            Message::Addr(addr_message) => addr_message.serialize(&mut v)?,
            Message::GetAddr(get_addr_message) => get_addr_message.serialize(&mut v)?,
            Message::Ping(nonce) => nonce.serialize(&mut v)?,
            Message::Pong(nonce) => nonce.serialize(&mut v)?,
            Message::VerAck(verack_message) => verack_message.serialize(&mut v)?,
            Message::GetHead => 0,
            Message::Head(header) => header.serialize(&mut v)?,
        };

        // write checksum to placeholder
        let mut v_crc = Vec::with_capacity(4);
        let crc32 = Crc32Computer::default().update(v.as_slice()).result().serialize(&mut v_crc);
        for i in 0..4 {
            v[checksum_start + i] = v_crc[i];
        }

        writer.write(v.as_slice())?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 4 + 4 + 4; // magic + serialized_size + checksum
        size += self.ty().serialized_size();
        size += match self {
            Message::Version(version_message) => version_message.serialized_size(),
            Message::Inv(inv_vector) => inv_vector.serialized_size::<u16>(),
            Message::GetData(inv_vector) => inv_vector.serialized_size::<u16>(),
            Message::GetHeader(inv_vector) => inv_vector.serialized_size::<u16>(),
            Message::NotFound(inv_vector) => inv_vector.serialized_size::<u16>(),
            Message::Block(block_message) => block_message.serialized_size(),
            Message::Header(header_message) => header_message.serialized_size(),
            Message::Tx(tx_message) => tx_message.serialized_size(),
            Message::GetBlocks(get_blocks_message) => get_blocks_message.serialized_size(),
            Message::Mempool => 0,
            Message::Reject(reject_message) => reject_message.serialized_size(),
            Message::Subscribe(subscribe_message) => subscribe_message.serialized_size(),
            Message::Addr(addr_message) => addr_message.serialized_size(),
            Message::GetAddr(get_addr_message) => get_addr_message.serialized_size(),
            Message::Ping(nonce) => nonce.serialized_size(),
            Message::Pong(nonce) => nonce.serialized_size(),
            Message::VerAck(verack_message) => verack_message.serialized_size(),
            Message::GetHead => 0,
            Message::Head(header) => header.serialized_size(),
        };
        return size;
    }
}

pub struct MessageNotifier {
    pub version: RwLock<PassThroughNotifier<'static, VersionMessage>>,
    pub ver_ack: RwLock<PassThroughNotifier<'static, VerAckMessage>>,
    pub inv: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub get_data: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub get_header: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub not_found: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub block: RwLock<PassThroughNotifier<'static, Block>>,
    pub header: RwLock<PassThroughNotifier<'static, BlockHeader>>,
    pub tx: RwLock<PassThroughNotifier<'static, TxMessage>>,
    pub get_blocks: RwLock<PassThroughNotifier<'static, GetBlocksMessage>>,
    pub mempool: RwLock<PassThroughNotifier<'static, ()>>,
    pub reject: RwLock<PassThroughNotifier<'static, RejectMessage>>,
    pub subscribe: RwLock<PassThroughNotifier<'static, Subscription>>,
    pub addr: RwLock<PassThroughNotifier<'static, AddrMessage>>,
    pub get_addr: RwLock<PassThroughNotifier<'static, GetAddrMessage>>,
    pub ping: RwLock<PassThroughNotifier<'static, /*nonce*/ u32>>,
    pub pong: RwLock<PassThroughNotifier<'static, /*nonce*/ u32>>,
    pub get_head: RwLock<PassThroughNotifier<'static, ()>>,
    pub head: RwLock<PassThroughNotifier<'static, BlockHeader>>,
}

impl MessageNotifier {
    pub fn new() -> Self {
        MessageNotifier {
            version: RwLock::new(PassThroughNotifier::new()),
            ver_ack: RwLock::new(PassThroughNotifier::new()),
            inv: RwLock::new(PassThroughNotifier::new()),
            get_data: RwLock::new(PassThroughNotifier::new()),
            get_header: RwLock::new(PassThroughNotifier::new()),
            not_found: RwLock::new(PassThroughNotifier::new()),
            block: RwLock::new(PassThroughNotifier::new()),
            header: RwLock::new(PassThroughNotifier::new()),
            tx: RwLock::new(PassThroughNotifier::new()),
            get_blocks: RwLock::new(PassThroughNotifier::new()),
            mempool: RwLock::new(PassThroughNotifier::new()),
            reject: RwLock::new(PassThroughNotifier::new()),
            subscribe: RwLock::new(PassThroughNotifier::new()),
            addr: RwLock::new(PassThroughNotifier::new()),
            get_addr: RwLock::new(PassThroughNotifier::new()),
            ping: RwLock::new(PassThroughNotifier::new()),
            pong: RwLock::new(PassThroughNotifier::new()),
            get_head: RwLock::new(PassThroughNotifier::new()),
            head: RwLock::new(PassThroughNotifier::new()),
        }
    }

    pub fn notify(&self, msg: Message) {
        match msg {
            Message::Version(msg) => self.version.read().notify(msg),
            Message::VerAck(msg) => self.ver_ack.read().notify(msg),
            Message::Inv(vector) => self.inv.read().notify(vector),
            Message::GetData(vector) => self.get_data.read().notify(vector),
            Message::GetHeader(vector) => self.get_header.read().notify(vector),
            Message::NotFound(vector) => self.not_found.read().notify(vector),
            Message::Block(block) => self.block.read().notify(block),
            Message::Header(header) => self.header.read().notify(header),
            Message::Tx(msg) => self.tx.read().notify(msg),
            Message::GetBlocks(msg) => self.get_blocks.read().notify(msg),
            Message::Mempool => self.mempool.read().notify(()),
            Message::Reject(msg) => self.reject.read().notify(msg),
            Message::Subscribe(msg) => self.subscribe.read().notify(msg),
            Message::Addr(msg) => self.addr.read().notify(msg),
            Message::GetAddr(msg) => self.get_addr.read().notify(msg),
            Message::Ping(nonce) => self.ping.read().notify(nonce),
            Message::Pong(nonce) => self.pong.read().notify(nonce),
            Message::GetHead => self.get_head.read().notify(()),
            Message::Head(header) => self.head.read().notify(header),
        }
    }
}


create_typed_array!(ChallengeNonce, u8, 32);

impl ChallengeNonce {
    pub fn generate() -> Self {
        let mut nonce = Self::default();
        let mut cspring: OsRng = OsRng::new().unwrap();
        cspring.fill_bytes(&mut nonce.0);
        nonce
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionMessage {
    pub version: u32,
    pub peer_address: PeerAddress,
    pub genesis_hash: Blake2bHash,
    pub head_hash: Blake2bHash,
    pub challenge_nonce: ChallengeNonce,
    #[beserial(len_type(u8))]
    pub user_agent: String, // FIXME: older versions don't have this field at all, need to take care of that scenario
}

impl VersionMessage {
    pub fn new(peer_address: PeerAddress, head_hash: Blake2bHash, genesis_hash: Blake2bHash, challenge_nonce: ChallengeNonce, user_agent: Option<String>) -> Message {
        Message::Version(Self {
            version: version::CODE,
            peer_address,
            genesis_hash,
            head_hash,
            challenge_nonce,
            user_agent: user_agent.unwrap_or_else(|| String::new())
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum InvVectorType {
    Error = 0,
    Transaction = 1,
    Block = 2,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct InvVector {
    pub ty: InvVectorType,
    pub hash: Blake2bHash,
}
impl InvVector {
    pub fn new(ty: InvVectorType, hash: Blake2bHash) -> Self {
        InvVector { ty, hash }
    }

    pub fn from_block(block: &Block) -> Self {
        Self::new(InvVectorType::Block, block.header.hash())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxMessage {
    transaction: Transaction,
    accounts_proof: Option<AccountsProof>,
}
impl TxMessage {
    pub fn new(transaction: Transaction) -> Message {
        Message::Tx(Self {
            transaction,
            accounts_proof: None
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum GetBlocksDirection {
    Forward = 1,
    Backward = 2,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetBlocksMessage {
    #[beserial(len_type(u16))]
    pub locators: Vec<Blake2bHash>,
    pub max_inv_size: u16,
    pub direction: GetBlocksDirection,
}
impl GetBlocksMessage {
    pub const LOCATORS_MAX_COUNT: usize = 128;

    pub fn new(locators: Vec<Blake2bHash>, max_inv_size: u16, direction: GetBlocksDirection) -> Message {
        Message::GetBlocks(Self {
            locators,
            max_inv_size,
            direction,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum RejectMessageCode {
    Malformed = 0x01,
    Invalid = 0x10,
    Obsolete = 0x11,
    Double = 0x12,
    Dust = 0x41,
    InsufficientFee = 0x42,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectMessage {
    message_type: MessageType,
    code: RejectMessageCode,
    #[beserial(len_type(u8))]
    reason: String,
    #[beserial(len_type(u16))]
    extra_data: Vec<u8>,
}

impl RejectMessage {
    pub fn new(message_type: MessageType, code: RejectMessageCode, reason: String, extra_data: Option<Vec<u8>>) -> Message {
        Message::Reject(Self {
            message_type,
            code,
            reason,
            extra_data: extra_data.unwrap_or_else(|| Vec::new()),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddrMessage {
    #[beserial(len_type(u16))]
    pub addresses: Vec<PeerAddress>
}

impl AddrMessage {
    pub fn new(addresses: Vec<PeerAddress>) -> Message {
        Message::Addr(Self {
            addresses
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsProofMessage {
    block_hash: Blake2bHash,
    accounts_proof: Option<AccountsProof>,
}

#[derive(Clone, Debug)]
pub struct GetAddrMessage {
    pub protocol_mask: ProtocolFlags,
    pub service_mask: ServiceFlags,
    pub max_results: u16, // TODO this is optional right now but is always set
}

impl GetAddrMessage {
    pub fn new(protocol_mask: ProtocolFlags, service_mask: ServiceFlags, max_results: u16) -> Message {
        Message::GetAddr(Self {
            protocol_mask,
            service_mask,
            max_results
        })
    }
}

impl Serialize for GetAddrMessage {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size= 0;
        size += self.protocol_mask.bits().serialize(writer)?;
        size += self.service_mask.bits().serialize(writer)?;
        size += self.max_results.serialize(writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size= 0;
        size += self.protocol_mask.bits().serialized_size();
        size += self.service_mask.bits().serialized_size();
        size += self.max_results.serialized_size();
        size
    }
}

impl Deserialize for GetAddrMessage {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let protocol_mask = ProtocolFlags::from_bits_truncate(Deserialize::deserialize(reader)?);
        let service_mask = ServiceFlags::from_bits_truncate(Deserialize::deserialize(reader)?);
        let max_results: u16 = Deserialize::deserialize(reader)?;
        Ok(GetAddrMessage {
            protocol_mask,
            service_mask,
            max_results,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerAckMessage {
    pub public_key: PublicKey,
    pub signature: Signature,
}

impl VerAckMessage {
    pub fn new(peer_id: &PeerId, peer_challence_nonce: &ChallengeNonce, key_pair: &KeyPair) -> Message {
        let mut data = peer_id.serialize_to_vec();
        peer_challence_nonce.serialize(&mut data).unwrap();
        let signature = key_pair.sign(&data[..]);
        Message::VerAck(Self {
            public_key: key_pair.public.clone(),
            signature,
        })
    }
}
