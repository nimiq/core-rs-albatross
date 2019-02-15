#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate bitflags;
extern crate nimiq_account as account;
extern crate nimiq_accounts as accounts;
extern crate nimiq_block as block;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
#[macro_use]
extern crate nimiq_macros as macros;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_tree_primitives as tree_primitives;
extern crate nimiq_utils as utils;

use std::io;
use std::io::Read;

use byteorder::{BigEndian, ByteOrder};
use parking_lot::RwLock;
use rand::Rng;
use rand::rngs::OsRng;

use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, SerializingError, uvar, WriteBytesExt};
use block::{Block, BlockHeader};
use block::proof::ChainProof;
use hash::{Blake2bHash, Hash};
use keys::{Address, KeyPair, PublicKey, Signature};
use network_primitives::address::{PeerAddress, PeerId};
use network_primitives::protocol::ProtocolFlags;
use network_primitives::services::ServiceFlags;
use network_primitives::subscription::Subscription;
use network_primitives::version;
use transaction::{Transaction, TransactionReceipt, TransactionsProof};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::crc::Crc32Computer;
use utils::observer::PassThroughNotifier;

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
    GetTransactionsProof = 47,
    TransactionsProof = 48,
    GetTransactionReceipts = 49,
    TransactionReceipts = 50,
    GetBlockProof = 51,
    BlockProof = 52,

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

    Signal(SignalMessage),

    GetChainProof,
    ChainProof(ChainProof),
    GetAccountsProof(GetAccountsProofMessage),
    AccountsProof(AccountsProofMessage),
    GetAccountsTreeChunk(GetAccountsTreeChunkMessage),
    AccountsTreeChunk(AccountsTreeChunkMessage),
    GetTransactionsProof(GetTransactionsProofMessage),
    TransactionsProof(TransactionsProofMessage),
    GetTransactionReceipts(GetTransactionReceiptsMessage),
    TransactionReceipts(TransactionReceiptsMessage),
    GetBlockProof(GetBlockProofMessage),
    BlockProof(BlockProofMessage),

    GetHead,
    Head(BlockHeader),

    VerAck(VerAckMessage),
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
            Message::Signal(_) => MessageType::Signal,
            Message::GetChainProof => MessageType::GetChainProof,
            Message::ChainProof(_) => MessageType::ChainProof,
            Message::GetAccountsProof(_) => MessageType::GetAccountsProof,
            Message::AccountsProof(_) => MessageType::AccountsProof,
            Message::GetAccountsTreeChunk(_) => MessageType::GetAccountsTreeChunk,
            Message::AccountsTreeChunk(_) => MessageType::AccountsTreeChunk,
            Message::GetTransactionsProof(_) => MessageType::GetTransactionsProof,
            Message::TransactionsProof(_) => MessageType::TransactionsProof,
            Message::GetTransactionReceipts(_) => MessageType::GetTransactionReceipts,
            Message::TransactionReceipts(_) => MessageType::TransactionReceipts,
            Message::GetBlockProof(_) => MessageType::GetBlockProof,
            Message::BlockProof(_) => MessageType::BlockProof,
            Message::GetHead => MessageType::GetHead,
            Message::Head(_) => MessageType::Head,
            Message::VerAck(_) => MessageType::VerAck,
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
        // Length is ignored (just like in JS implementation).
        let _length: u32 = Deserialize::deserialize(&mut crc32_reader)?;
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
            MessageType::Signal => Message::Signal(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetChainProof => Message::GetChainProof,
            MessageType::ChainProof => Message::ChainProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetAccountsProof => Message::GetAccountsProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::AccountsProof => Message::AccountsProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetAccountsTreeChunk => Message::GetAccountsTreeChunk(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::AccountsTreeChunk => Message::AccountsTreeChunk(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetTransactionsProof => Message::GetTransactionsProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::TransactionsProof => Message::TransactionsProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetTransactionReceipts => Message::GetTransactionReceipts(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::TransactionReceipts => Message::TransactionReceipts(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetBlockProof => Message::GetBlockProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::BlockProof => Message::BlockProof(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetHead => Message::GetHead,
            MessageType::Head => Message::Head(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::VerAck => Message::VerAck(Deserialize::deserialize(&mut crc32_reader)?),
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
            Message::Signal(signal_message) => signal_message.serialize(&mut v)?,
            Message::GetChainProof => 0,
            Message::ChainProof(msg) => msg.serialize(&mut v)?,
            Message::GetAccountsProof(get_accounts_proof_message) => get_accounts_proof_message.serialize(&mut v)?,
            Message::AccountsProof(accounts_proof_message) => accounts_proof_message.serialize(&mut v)?,
            Message::GetAccountsTreeChunk(get_accounts_tree_chunk_message) => get_accounts_tree_chunk_message.serialize(&mut v)?,
            Message::AccountsTreeChunk(accounts_tree_chunk_message) => accounts_tree_chunk_message.serialize(&mut v)?,
            Message::GetTransactionsProof(msg) => msg.serialize(&mut v)?,
            Message::TransactionsProof(msg) => msg.serialize(&mut v)?,
            Message::GetTransactionReceipts(msg) => msg.serialize(&mut v)?,
            Message::TransactionReceipts(msg) => msg.serialize(&mut v)?,
            Message::GetBlockProof(msg) => msg.serialize(&mut v)?,
            Message::BlockProof(msg) => msg.serialize(&mut v)?,
            Message::GetHead => 0,
            Message::Head(header) => header.serialize(&mut v)?,
            Message::VerAck(verack_message) => verack_message.serialize(&mut v)?,
        };

        // write checksum to placeholder
        let mut v_crc = Vec::with_capacity(4);
        Crc32Computer::default().update(v.as_slice()).result().serialize(&mut v_crc)?;
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
            Message::Signal(signal_message) => signal_message.serialized_size(),
            Message::GetChainProof => 0,
            Message::ChainProof(chain_proof_message) => chain_proof_message.serialized_size(),
            Message::GetAccountsProof(get_accounts_proof_message) => get_accounts_proof_message.serialized_size(),
            Message::AccountsProof(accounts_proof_message) => accounts_proof_message.serialized_size(),
            Message::GetAccountsTreeChunk(get_accounts_tree_chunk_message) => get_accounts_tree_chunk_message.serialized_size(),
            Message::AccountsTreeChunk(accounts_tree_chunk_message) => accounts_tree_chunk_message.serialized_size(),
            Message::GetTransactionsProof(msg) => msg.serialized_size(),
            Message::TransactionsProof(msg) => msg.serialized_size(),
            Message::GetTransactionReceipts(msg) => msg.serialized_size(),
            Message::TransactionReceipts(msg) => msg.serialized_size(),
            Message::GetBlockProof(msg) => msg.serialized_size(),
            Message::BlockProof(msg) => msg.serialized_size(),
            Message::GetHead => 0,
            Message::Head(header) => header.serialized_size(),
            Message::VerAck(verack_message) => verack_message.serialized_size(),
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
    pub signal: RwLock<PassThroughNotifier<'static, SignalMessage>>,
    pub get_chain_proof: RwLock<PassThroughNotifier<'static, ()>>,
    pub chain_proof: RwLock<PassThroughNotifier<'static, ChainProof>>,
    pub get_accounts_proof: RwLock<PassThroughNotifier<'static, GetAccountsProofMessage>>,
    pub get_accounts_tree_chunk: RwLock<PassThroughNotifier<'static, GetAccountsTreeChunkMessage>>,
    pub accounts_tree_chunk: RwLock<PassThroughNotifier<'static, AccountsTreeChunkMessage>>,
    pub accounts_proof: RwLock<PassThroughNotifier<'static, AccountsProofMessage>>,
    pub get_transactions_proof: RwLock<PassThroughNotifier<'static, GetTransactionsProofMessage>>,
    pub transactions_proof: RwLock<PassThroughNotifier<'static, TransactionsProofMessage>>,
    pub get_transaction_receipts: RwLock<PassThroughNotifier<'static, GetTransactionReceiptsMessage>>,
    pub transaction_receipts: RwLock<PassThroughNotifier<'static, TransactionReceiptsMessage>>,
    pub get_block_proof: RwLock<PassThroughNotifier<'static, GetBlockProofMessage>>,
    pub block_proof: RwLock<PassThroughNotifier<'static, BlockProofMessage>>,
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
            signal: RwLock::new(PassThroughNotifier::new()),
            get_chain_proof: RwLock::new(PassThroughNotifier::new()),
            chain_proof: RwLock::new(PassThroughNotifier::new()),
            get_accounts_proof: RwLock::new(PassThroughNotifier::new()),
            accounts_proof: RwLock::new(PassThroughNotifier::new()),
            get_accounts_tree_chunk: RwLock::new(PassThroughNotifier::new()),
            accounts_tree_chunk: RwLock::new(PassThroughNotifier::new()),
            get_transactions_proof: RwLock::new(PassThroughNotifier::new()),
            transactions_proof: RwLock::new(PassThroughNotifier::new()),
            get_transaction_receipts: RwLock::new(PassThroughNotifier::new()),
            transaction_receipts: RwLock::new(PassThroughNotifier::new()),
            get_block_proof: RwLock::new(PassThroughNotifier::new()),
            block_proof: RwLock::new(PassThroughNotifier::new()),
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
            Message::Signal(msg) => self.signal.read().notify(msg),
            Message::GetChainProof => self.get_chain_proof.read().notify(()),
            Message::ChainProof(proof) => self.chain_proof.read().notify(proof),
            Message::GetAccountsProof(msg) => self.get_accounts_proof.read().notify(msg),
            Message::AccountsProof(msg) => self.accounts_proof.read().notify(msg),
            Message::GetAccountsTreeChunk(msg) => self.get_accounts_tree_chunk.read().notify(msg),
            Message::AccountsTreeChunk(msg) => self.accounts_tree_chunk.read().notify(msg),
            Message::GetTransactionsProof(msg) => self.get_transactions_proof.read().notify(msg),
            Message::TransactionsProof(msg) => self.transactions_proof.read().notify(msg),
            Message::GetTransactionReceipts(msg) => self.get_transaction_receipts.read().notify(msg),
            Message::TransactionReceipts(msg) => self.transaction_receipts.read().notify(msg),
            Message::GetBlockProof(msg) => self.get_block_proof.read().notify(msg),
            Message::BlockProof(msg) => self.block_proof.read().notify(msg),
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
        cspring.fill(&mut nonce.0);
        nonce
    }
}

#[derive(Clone, Debug)]
pub struct VersionMessage {
    pub version: u32,
    pub peer_address: PeerAddress,
    pub genesis_hash: Blake2bHash,
    pub head_hash: Blake2bHash,
    pub challenge_nonce: ChallengeNonce,
    pub user_agent: Option<String>,
}

impl Deserialize for VersionMessage {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(VersionMessage {
            version: Deserialize::deserialize(reader)?,
            peer_address: Deserialize::deserialize(reader)?,
            genesis_hash: Deserialize::deserialize(reader)?,
            head_hash: Deserialize::deserialize(reader)?,
            challenge_nonce: Deserialize::deserialize(reader)?,
            user_agent: match DeserializeWithLength::deserialize::<u8, R>(reader) {
                Ok(user_agent) => Some(user_agent),
                Err(SerializingError::IoError(std::io::ErrorKind::UnexpectedEof, _)) => None,
                Err(e) => return Err(e),
            }
        })
    }
}

impl Serialize for VersionMessage {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.version, writer)?;
        size += Serialize::serialize(&self.peer_address, writer)?;
        size += Serialize::serialize(&self.genesis_hash, writer)?;
        size += Serialize::serialize(&self.head_hash, writer)?;
        size += Serialize::serialize(&self.challenge_nonce, writer)?;
        match &self.user_agent {
            Some(u) => size += SerializeWithLength::serialize::<u8, W>(u, writer)?,
            _ => ()
        };
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.version);
        size += Serialize::serialized_size(&self.peer_address);
        size += Serialize::serialized_size(&self.genesis_hash);
        size += Serialize::serialized_size(&self.head_hash);
        size += Serialize::serialized_size(&self.challenge_nonce);
        match &self.user_agent {
            Some(u) => size += SerializeWithLength::serialized_size::<u8>(u),
            _ => ()
        };
        size
    }
}

impl VersionMessage {
    pub fn new(peer_address: PeerAddress, head_hash: Blake2bHash, genesis_hash: Blake2bHash, challenge_nonce: ChallengeNonce, user_agent: Option<String>) -> Message {
        Message::Version(Self {
            version: version::CODE,
            peer_address,
            genesis_hash,
            head_hash,
            challenge_nonce,
            user_agent
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
    pub const VECTORS_MAX_COUNT: usize = 1000;

    pub fn new(ty: InvVectorType, hash: Blake2bHash) -> Self {
        InvVector { ty, hash }
    }

    pub fn from_block(block: &Block) -> Self {
        Self::new(InvVectorType::Block, block.header.hash())
    }

    pub fn from_transaction(transaction: &Transaction) -> Self {
        Self::new(InvVectorType::Transaction, transaction.hash())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxMessage {
    pub transaction: Transaction,
    pub accounts_proof: Option<AccountsProof>,
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
        let mut size = 0;
        size += self.protocol_mask.bits().serialize(writer)?;
        size += self.service_mask.bits().serialize(writer)?;
        size += self.max_results.serialize(writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
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

#[derive(Clone, Debug)]
pub struct SignalMessage {
    pub sender_id: PeerId,
    pub recipient_id: PeerId,
    pub nonce: u32,
    pub ttl: u8,
    pub flags: SignalMessageFlags,
    pub payload: Vec<u8>,
    pub sender_public_key: Option<PublicKey>,
    pub signature: Option<Signature>,
}

bitflags! {
    #[derive(Default, Serialize, Deserialize)]
    pub struct SignalMessageFlags: u8 {
        const UNROUTABLE  = 0b00000001;
        const TTL_EXCEEDED = 0b00000010;
    }
}

impl Deserialize for SignalMessage {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let sender_id = Deserialize::deserialize(reader)?;
        let recipient_id = Deserialize::deserialize(reader)?;
        let nonce = Deserialize::deserialize(reader)?;
        let ttl = Deserialize::deserialize(reader)?;
        let flags = SignalMessageFlags::from_bits_truncate(Deserialize::deserialize(reader)?);
        let payload: Vec<u8> = DeserializeWithLength::deserialize::<u16, R>(reader)?;
        let sender_public_key = if payload.len() > 0 { Some(Deserialize::deserialize(reader)?) } else { None };
        let signature = if payload.len() > 0 { Some(Deserialize::deserialize(reader)?) } else { None };

        Ok(SignalMessage {
            sender_id,
            recipient_id,
            nonce,
            ttl,
            flags,
            payload,
            sender_public_key,
            signature,
        })
    }
}

impl Serialize for SignalMessage {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.sender_id, writer)?;
        size += Serialize::serialize(&self.recipient_id, writer)?;
        size += Serialize::serialize(&self.nonce, writer)?;
        size += Serialize::serialize(&self.ttl, writer)?;
        size += self.flags.bits().serialize(writer)?;
        size += SerializeWithLength::serialize::<u16, W>(&self.payload, writer)?;
        if self.payload.len() > 0 {
            size += Serialize::serialize(&self.sender_public_key.clone().unwrap(), writer)?;
            size += Serialize::serialize(&self.signature.clone().unwrap(), writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.sender_id);
        size += Serialize::serialized_size(&self.recipient_id);
        size += Serialize::serialized_size(&self.nonce);
        size += Serialize::serialized_size(&self.ttl);
        size += self.flags.bits().serialized_size();
        size += SerializeWithLength::serialized_size::<u16>(&self.payload);
        if self.payload.len() > 0 {
            size += Serialize::serialized_size(&self.sender_public_key.clone().unwrap());
            size += Serialize::serialized_size(&self.signature.clone().unwrap());
        }
        size
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetAccountsProofMessage {
    pub block_hash: Blake2bHash,
    #[beserial(len_type(u16))]
    pub addresses: Vec<Address>
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsProofMessage {
    pub block_hash: Blake2bHash,
    pub proof: Option<AccountsProof>,
}

impl AccountsProofMessage {
    pub fn new(block_hash: Blake2bHash, proof: Option<AccountsProof>) -> Message {
        Message::AccountsProof(AccountsProofMessage {
            block_hash,
            proof,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetAccountsTreeChunkMessage {
    pub block_hash: Blake2bHash,
    #[beserial(len_type(u8))]
    pub start_prefix: String,
}

/// We use the following enum to store cached chunks more efficiently.
/// Upon deserialization, we will always get the structured variant though.
#[derive(Clone, Debug)]
pub enum AccountsTreeChunkData {
    Serialized(Vec<u8>),
    Structured(AccountsTreeChunk),
}

impl AccountsTreeChunkData {
    pub fn to_serialized(self) -> Self {
        match self {
            data @ AccountsTreeChunkData::Serialized(_) => data,
            AccountsTreeChunkData::Structured(chunk) => AccountsTreeChunkData::Serialized(chunk.serialize_to_vec()),
        }
    }
}

impl Serialize for AccountsTreeChunkData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            AccountsTreeChunkData::Serialized(ref buf) => {
                writer.write_all(buf.as_slice())?;
                Ok(buf.len())
            },
            AccountsTreeChunkData::Structured(ref chunk) => {
                chunk.serialize(writer)
            },
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            AccountsTreeChunkData::Serialized(ref buf) => {
                buf.len()
            },
            AccountsTreeChunkData::Structured(ref chunk) => {
                chunk.serialized_size()
            },
        }
    }
}

impl Deserialize for AccountsTreeChunkData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(AccountsTreeChunkData::Structured(Deserialize::deserialize(reader)?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeChunkMessage {
    pub block_hash: Blake2bHash,
    pub accounts_tree_chunk: Option<AccountsTreeChunkData>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetTransactionsProofMessage {
    pub block_hash: Blake2bHash,
    #[beserial(len_type(u16))]
    pub addresses: Vec<Address>
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionsProofMessage {
    pub block_hash: Blake2bHash,
    pub transactions_proof: Option<TransactionsProof>,
}

impl TransactionsProofMessage {
    pub fn new(block_hash: Blake2bHash, transactions_proof: Option<TransactionsProof>) -> Message {
        Message::TransactionsProof(TransactionsProofMessage {
            block_hash,
            transactions_proof,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetTransactionReceiptsMessage {
    pub address: Address,
    pub offset: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionReceiptsMessage {
    #[beserial(len_type(u16))]
    pub receipts: Option<Vec<TransactionReceipt>>,
}

impl TransactionReceiptsMessage {
    pub const RECEIPTS_MAX_COUNT: usize = 500;

    pub fn new(receipts: Vec<TransactionReceipt>) -> Message {
        Message::TransactionReceipts(TransactionReceiptsMessage {
            receipts: Some(receipts),
        })
    }

    pub fn empty() -> Message {
        Message::TransactionReceipts(TransactionReceiptsMessage {
            receipts: None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetBlockProofMessage {
    pub block_hash_to_prove: Blake2bHash,
    pub known_block_hash: Blake2bHash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockProofMessage {
    #[beserial(len_type(u16))]
    pub proof: Option<Vec<Block>>,
}

impl BlockProofMessage {
    pub fn new(proof: Option<Vec<Block>>) -> Message {
        Message::BlockProof(BlockProofMessage {
            proof,
        })
    }

    pub fn empty() -> Message {
        Message::BlockProof(BlockProofMessage {
            proof: None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerAckMessage {
    pub public_key: PublicKey,
    pub signature: Signature,
}

impl VerAckMessage {
    pub fn new(peer_id: &PeerId, peer_challenge_nonce: &ChallengeNonce, key_pair: &KeyPair) -> Message {
        let mut data = peer_id.serialize_to_vec();
        peer_challenge_nonce.serialize(&mut data).unwrap();
        let signature = key_pair.sign(&data[..]);
        Message::VerAck(Self {
            public_key: key_pair.public.clone(),
            signature,
        })
    }
}
