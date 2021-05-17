// TODO: Perhaps move constructors into Message
#![allow(clippy::new_ret_no_self)]

#[macro_use]
extern crate beserial_derive;

use std::fmt::Display;
use std::io;
use std::io::{Cursor, ErrorKind, Read, Seek, SeekFrom};

use bitflags::bitflags;
use parking_lot::RwLock;
use rand::rngs::OsRng;
use rand::Rng;

use beserial::{
    uvar, Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_account::Account;
use nimiq_block::{Block, BlockHeader, ForkProof, MultiSignature, ViewChange, ViewChangeProof};
use nimiq_handel::update::LevelUpdateMessage;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, KeyPair, PublicKey, Signature};
use nimiq_macros::{add_hex_io_fns_typed_arr, create_typed_array};
use nimiq_network_interface::message::Message as MessageInterface;
use nimiq_peer_address::address::{PeerAddress, PeerId};
use nimiq_peer_address::protocol::ProtocolFlags;
use nimiq_peer_address::services::ServiceFlags;
use nimiq_peer_address::version;
use nimiq_subscription::Subscription;
use nimiq_transaction::{Transaction, TransactionReceipt, TransactionsProof};
use nimiq_tree::accounts_proof::AccountsProof;
use nimiq_tree::accounts_tree_chunk::AccountsTreeChunk;
use nimiq_utils::crc::Crc32Computer;
use nimiq_utils::merkle::partial::Blake2bPartialMerkleProof;
use nimiq_utils::observer::{PassThroughListener, PassThroughNotifier};

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
    Tx = 8,
    Mempool = 9,
    Reject = 10,
    Subscribe = 11,

    Addr = 20,
    GetAddr = 21,
    Ping = 22,
    Pong = 23,

    Signal = 30,

    GetAccountsProof = 42,
    AccountsProof = 43,
    GetAccountsTreeChunk = 44,
    AccountsTreeChunk = 45,
    GetTransactionsProof = 47,
    TransactionsProof = 48,
    GetTransactionReceipts = 49,
    TransactionReceipts = 50,

    GetHead = 60,

    VerAck = 90,

    // Albatross
    BlockAlbatross = 100,
    HeaderAlbatross = 101,
    ViewChange = 105,
    ViewChangeProof = 106,
    ForkProof = 107,
    GetMacroBlocks = 123,
    GetEpochTransactions = 124,
    EpochTransactions = 125,
}

impl Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version => write!(f, "version"),
            Self::Inv => write!(f, "inv"),
            Self::GetData => write!(f, "get-data"),
            Self::GetHeader => write!(f, "get-header"),
            Self::NotFound => write!(f, "not-found"),
            Self::GetBlocks => write!(f, "get-blocks"),
            Self::Tx => write!(f, "tx"),
            Self::Mempool => write!(f, "mempool"),
            Self::Reject => write!(f, "reject"),
            Self::Subscribe => write!(f, "subscribe"),

            Self::Addr => write!(f, "addr"),
            Self::GetAddr => write!(f, "get-addr"),
            Self::Ping => write!(f, "ping"),
            Self::Pong => write!(f, "pong"),

            Self::Signal => write!(f, "signal"),

            Self::GetAccountsProof => write!(f, "get-accounts-proof"),
            Self::AccountsProof => write!(f, "accounts-proof"),
            Self::GetAccountsTreeChunk => write!(f, "get-accounts-tree-chunk"),
            Self::AccountsTreeChunk => write!(f, "accounts-tree-chunk"),
            Self::GetTransactionsProof => write!(f, "get-transactions-proof"),
            Self::TransactionsProof => write!(f, "transactions-proof"),
            Self::GetTransactionReceipts => write!(f, "get-transaction-receipts"),
            Self::TransactionReceipts => write!(f, "transaction-receipts"),

            Self::GetHead => write!(f, "get-head"),
            Self::VerAck => write!(f, "verack"),

            // Albatross
            Self::BlockAlbatross => write!(f, "block-albatross"),
            Self::HeaderAlbatross => write!(f, "header-albatross"),
            Self::ViewChange => write!(f, "view-change"),
            Self::ViewChangeProof => write!(f, "view-change-proof"),
            Self::ForkProof => write!(f, "fork-proof"),
            Self::GetMacroBlocks => write!(f, "get-macro-blocks"),
            Self::GetEpochTransactions => write!(f, "get-epoch-transactions"),
            Self::EpochTransactions => write!(f, "epoch-transactions"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Version(Box<VersionMessage>),
    Inv(Vec<InvVector>),
    GetData(Vec<InvVector>),
    GetHeader(Vec<InvVector>),
    NotFound(Vec<InvVector>),
    Tx(Box<TxMessage>),
    GetBlocks(Box<GetBlocksMessage>),
    Mempool,
    Reject(Box<RejectMessage>),
    Subscribe(Box<Subscription>),

    Addr(Box<AddrMessage>),
    GetAddr(Box<GetAddrMessage>),
    Ping(/*nonce*/ u32),
    Pong(/*nonce*/ u32),

    Signal(Box<SignalMessage>),

    GetAccountsProof(Box<GetAccountsProofMessage>),
    AccountsProof(Box<AccountsProofMessage>),
    GetAccountsTreeChunk(Box<GetAccountsTreeChunkMessage>),
    AccountsTreeChunk(Box<AccountsTreeChunkMessage>),
    GetTransactionsProof(Box<GetTransactionsProofMessage>),
    TransactionsProof(Box<TransactionsProofMessage>),
    GetTransactionReceipts(Box<GetTransactionReceiptsMessage>),
    TransactionReceipts(Box<TransactionReceiptsMessage>),

    GetHead,

    VerAck(Box<VerAckMessage>),

    // Albatross
    BlockAlbatross(Box<Block>),
    HeaderAlbatross(Box<BlockHeader>),
    ForkProof(Box<ForkProof>),
    ViewChange(Box<LevelUpdateMessage<MultiSignature, ViewChange>>),
    ViewChangeProof(Box<ViewChangeProofMessage>),
    GetMacroBlocks(Box<GetBlocksMessage>),
    GetEpochTransactions(Box<GetEpochTransactionsMessage>),
    EpochTransactions(Box<EpochTransactionsMessage>),
}

impl Message {
    pub fn ty(&self) -> MessageType {
        match self {
            Message::Version(_) => MessageType::Version,
            Message::Inv(_) => MessageType::Inv,
            Message::GetData(_) => MessageType::GetData,
            Message::GetHeader(_) => MessageType::GetHeader,
            Message::NotFound(_) => MessageType::NotFound,
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
            Message::GetAccountsProof(_) => MessageType::GetAccountsProof,
            Message::AccountsProof(_) => MessageType::AccountsProof,
            Message::GetAccountsTreeChunk(_) => MessageType::GetAccountsTreeChunk,
            Message::AccountsTreeChunk(_) => MessageType::AccountsTreeChunk,
            Message::GetTransactionsProof(_) => MessageType::GetTransactionsProof,
            Message::TransactionsProof(_) => MessageType::TransactionsProof,
            Message::GetTransactionReceipts(_) => MessageType::GetTransactionReceipts,
            Message::TransactionReceipts(_) => MessageType::TransactionReceipts,
            Message::GetHead => MessageType::GetHead,
            Message::VerAck(_) => MessageType::VerAck,
            // Albatross
            Message::BlockAlbatross(_) => MessageType::BlockAlbatross,
            Message::HeaderAlbatross(_) => MessageType::HeaderAlbatross,
            Message::ViewChange(_) => MessageType::ViewChange,
            Message::ViewChangeProof(_) => MessageType::ViewChangeProof,
            Message::ForkProof(_) => MessageType::ForkProof,
            Message::GetMacroBlocks(_) => MessageType::GetMacroBlocks,
            Message::GetEpochTransactions(_) => MessageType::GetEpochTransactions,
            Message::EpochTransactions(_) => MessageType::EpochTransactions,
        }
    }

    pub fn peek_length(buffer: &[u8]) -> Result<usize, SerializingError> {
        let mut c = Cursor::new(buffer);

        // skip 4 bytes of magic
        c.seek(SeekFrom::Start(4))?;

        // skip type (uvar)
        let _ = uvar::deserialize(&mut c)?;
        let n = u32::deserialize(&mut c)?;

        Ok(n as usize)
    }
}

const MAGIC: u32 = 0x4204_2042;

impl Deserialize for Message {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        pub struct ReaderComputeCrc32<'a, T: 'a + ReadBytesExt> {
            reader: &'a mut T,
            crc32: Crc32Computer,
            at_checksum: bool,
        }

        impl<'a, T: ReadBytesExt> ReaderComputeCrc32<'a, T> {
            pub fn new(reader: &'a mut T) -> ReaderComputeCrc32<T> {
                ReaderComputeCrc32 {
                    reader,
                    crc32: Crc32Computer::default(),
                    at_checksum: false,
                }
            }
        }

        impl<'a, T: ReadBytesExt> io::Read for ReaderComputeCrc32<'a, T> {
            fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
                let size = self.reader.read(buf)?;
                if size > 0 {
                    if self.at_checksum {
                        // We're at the checksum, so'll just ignore it
                        let zeros = [0u8; 4];
                        self.crc32.update(&zeros);
                    } else {
                        self.crc32.update(&buf[..size]);
                    }
                }
                Ok(size)
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
        crc32_reader.at_checksum = true;
        let checksum: u32 = Deserialize::deserialize(&mut crc32_reader)?;
        crc32_reader.at_checksum = false;

        let message: Message = match ty {
            MessageType::Version => Message::Version(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Inv => Message::Inv(DeserializeWithLength::deserialize_with_limit::<
                u16,
                ReaderComputeCrc32<R>,
            >(
                &mut crc32_reader,
                Some(InvVector::VECTORS_MAX_COUNT),
            )?),
            MessageType::GetData => {
                Message::GetData(DeserializeWithLength::deserialize_with_limit::<
                    u16,
                    ReaderComputeCrc32<R>,
                >(
                    &mut crc32_reader, Some(InvVector::VECTORS_MAX_COUNT)
                )?)
            }
            MessageType::GetHeader => {
                Message::GetHeader(DeserializeWithLength::deserialize_with_limit::<
                    u16,
                    ReaderComputeCrc32<R>,
                >(
                    &mut crc32_reader, Some(InvVector::VECTORS_MAX_COUNT)
                )?)
            }
            MessageType::NotFound => {
                Message::NotFound(DeserializeWithLength::deserialize_with_limit::<
                    u16,
                    ReaderComputeCrc32<R>,
                >(
                    &mut crc32_reader, Some(InvVector::VECTORS_MAX_COUNT)
                )?)
            }
            MessageType::Tx => Message::Tx(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetBlocks => {
                Message::GetBlocks(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::Mempool => Message::Mempool,
            MessageType::Reject => Message::Reject(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Subscribe => {
                Message::Subscribe(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::Addr => Message::Addr(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetAddr => Message::GetAddr(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Ping => Message::Ping(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Pong => Message::Pong(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::Signal => Message::Signal(Deserialize::deserialize(&mut crc32_reader)?),
            MessageType::GetAccountsProof => {
                Message::GetAccountsProof(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::AccountsProof => {
                Message::AccountsProof(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::GetAccountsTreeChunk => {
                Message::GetAccountsTreeChunk(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::AccountsTreeChunk => {
                Message::AccountsTreeChunk(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::GetTransactionsProof => {
                Message::GetTransactionsProof(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::TransactionsProof => {
                Message::TransactionsProof(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::GetTransactionReceipts => {
                Message::GetTransactionReceipts(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::TransactionReceipts => {
                Message::TransactionReceipts(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::GetHead => Message::GetHead,
            MessageType::VerAck => Message::VerAck(Deserialize::deserialize(&mut crc32_reader)?),
            // Albatross
            MessageType::BlockAlbatross => {
                Message::BlockAlbatross(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::HeaderAlbatross => {
                Message::HeaderAlbatross(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::ForkProof => {
                Message::ForkProof(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::ViewChange => {
                Message::ViewChange(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::ViewChangeProof => {
                Message::ViewChangeProof(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::GetMacroBlocks => {
                Message::GetMacroBlocks(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::GetEpochTransactions => {
                Message::GetEpochTransactions(Deserialize::deserialize(&mut crc32_reader)?)
            }
            MessageType::EpochTransactions => {
                Message::EpochTransactions(Deserialize::deserialize(&mut crc32_reader)?)
            }
        };

        // XXX Consume any leftover bytes in the message before computing the checksum.
        // This is consistent with the JS implementation.
        crc32_reader.read_to_end(&mut Vec::new()).unwrap();

        let crc_comp = crc32_reader.crc32.result();
        if crc_comp != checksum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Message deserialization: Bad checksum",
            )
            .into());
        }

        Ok(message)
    }
}

impl Serialize for Message {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
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
            Message::Tx(tx) => tx.serialize(&mut v)?,
            Message::GetBlocks(get_blocks_message) => get_blocks_message.serialize(&mut v)?,
            Message::Mempool => 0,
            Message::Reject(reject_message) => reject_message.serialize(&mut v)?,
            Message::Subscribe(subscribe_message) => subscribe_message.serialize(&mut v)?,
            Message::Addr(addr_message) => addr_message.serialize(&mut v)?,
            Message::GetAddr(get_addr_message) => get_addr_message.serialize(&mut v)?,
            Message::Ping(nonce) => nonce.serialize(&mut v)?,
            Message::Pong(nonce) => nonce.serialize(&mut v)?,
            Message::Signal(signal_message) => signal_message.serialize(&mut v)?,
            Message::GetAccountsProof(get_accounts_proof_message) => {
                get_accounts_proof_message.serialize(&mut v)?
            }
            Message::AccountsProof(accounts_proof_message) => {
                accounts_proof_message.serialize(&mut v)?
            }
            Message::GetAccountsTreeChunk(get_accounts_tree_chunk_message) => {
                get_accounts_tree_chunk_message.serialize(&mut v)?
            }
            Message::AccountsTreeChunk(accounts_tree_chunk_message) => {
                accounts_tree_chunk_message.serialize(&mut v)?
            }
            Message::GetTransactionsProof(msg) => msg.serialize(&mut v)?,
            Message::TransactionsProof(msg) => msg.serialize(&mut v)?,
            Message::GetTransactionReceipts(msg) => msg.serialize(&mut v)?,
            Message::TransactionReceipts(msg) => msg.serialize(&mut v)?,
            Message::GetHead => 0,
            Message::VerAck(verack_message) => verack_message.serialize(&mut v)?,
            // Albatross
            Message::BlockAlbatross(block) => block.serialize(&mut v)?,
            Message::HeaderAlbatross(header) => header.serialize(&mut v)?,
            Message::ViewChange(view_change_message) => view_change_message.serialize(&mut v)?,
            Message::ViewChangeProof(view_change_proof) => view_change_proof.serialize(&mut v)?,
            Message::ForkProof(fork_proof) => fork_proof.serialize(&mut v)?,
            Message::GetMacroBlocks(get_blocks_message) => get_blocks_message.serialize(&mut v)?,
            Message::GetEpochTransactions(get_epoch_transactions) => {
                get_epoch_transactions.serialize(&mut v)?
            }
            Message::EpochTransactions(epoch_transactions) => {
                epoch_transactions.serialize(&mut v)?
            }
        };

        // write checksum to placeholder
        let mut v_crc = Vec::with_capacity(4);
        Crc32Computer::default()
            .update(v.as_slice())
            .result()
            .serialize(&mut v_crc)?;

        v[checksum_start..(4 + checksum_start)].clone_from_slice(&v_crc[..4]);

        writer.write_all(v.as_slice())?;
        Ok(size)
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
            Message::Tx(tx) => tx.serialized_size(),
            Message::GetBlocks(get_blocks_message) => get_blocks_message.serialized_size(),
            Message::Mempool => 0,
            Message::Reject(reject_message) => reject_message.serialized_size(),
            Message::Subscribe(subscribe_message) => subscribe_message.serialized_size(),
            Message::Addr(addr_message) => addr_message.serialized_size(),
            Message::GetAddr(get_addr_message) => get_addr_message.serialized_size(),
            Message::Ping(nonce) => nonce.serialized_size(),
            Message::Pong(nonce) => nonce.serialized_size(),
            Message::Signal(signal_message) => signal_message.serialized_size(),
            Message::GetAccountsProof(get_accounts_proof_message) => {
                get_accounts_proof_message.serialized_size()
            }
            Message::AccountsProof(accounts_proof_message) => {
                accounts_proof_message.serialized_size()
            }
            Message::GetAccountsTreeChunk(get_accounts_tree_chunk_message) => {
                get_accounts_tree_chunk_message.serialized_size()
            }
            Message::AccountsTreeChunk(accounts_tree_chunk_message) => {
                accounts_tree_chunk_message.serialized_size()
            }
            Message::GetTransactionsProof(msg) => msg.serialized_size(),
            Message::TransactionsProof(msg) => msg.serialized_size(),
            Message::GetTransactionReceipts(msg) => msg.serialized_size(),
            Message::TransactionReceipts(msg) => msg.serialized_size(),
            Message::GetHead => 0,
            Message::VerAck(verack_message) => verack_message.serialized_size(),
            // Albatross
            Message::BlockAlbatross(block) => block.serialized_size(),
            Message::HeaderAlbatross(header) => header.serialized_size(),
            Message::ForkProof(fork_proof) => fork_proof.serialized_size(),
            Message::ViewChange(view_change_message) => view_change_message.serialized_size(),
            Message::ViewChangeProof(view_change_proof) => view_change_proof.serialized_size(),
            Message::GetMacroBlocks(get_blocks_message) => get_blocks_message.serialized_size(),
            Message::GetEpochTransactions(get_epoch_transactions) => {
                get_epoch_transactions.serialized_size()
            }
            Message::EpochTransactions(epoch_transactions) => epoch_transactions.serialized_size(),
        };
        size
    }
}

impl MessageInterface for Message {
    const TYPE_ID: u64 = 0;

    fn serialize_message<W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        self.serialize(writer)
    }

    fn serialized_message_size(&self) -> usize {
        let mut serialized_size = 4 + 4 + 4; // magic + serialized_size + checksum
        serialized_size += self.ty().serialized_size();
        serialized_size += self.serialized_size();
        serialized_size
    }

    fn deserialize_message<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Deserialize::deserialize(reader)
    }
}

#[derive(Default)]
pub struct MessageNotifier {
    pub version: RwLock<PassThroughNotifier<'static, VersionMessage>>,
    pub ver_ack: RwLock<PassThroughNotifier<'static, VerAckMessage>>,
    pub inv: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub get_data: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub get_header: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
    pub not_found: RwLock<PassThroughNotifier<'static, Vec<InvVector>>>,
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
    pub get_accounts_proof: RwLock<PassThroughNotifier<'static, GetAccountsProofMessage>>,
    pub get_accounts_tree_chunk: RwLock<PassThroughNotifier<'static, GetAccountsTreeChunkMessage>>,
    pub accounts_tree_chunk: RwLock<PassThroughNotifier<'static, AccountsTreeChunkMessage>>,
    pub accounts_proof: RwLock<PassThroughNotifier<'static, AccountsProofMessage>>,
    pub get_transactions_proof: RwLock<PassThroughNotifier<'static, GetTransactionsProofMessage>>,
    pub transactions_proof: RwLock<PassThroughNotifier<'static, TransactionsProofMessage>>,
    pub get_transaction_receipts:
        RwLock<PassThroughNotifier<'static, GetTransactionReceiptsMessage>>,
    pub transaction_receipts: RwLock<PassThroughNotifier<'static, TransactionReceiptsMessage>>,
    pub get_head: RwLock<PassThroughNotifier<'static, ()>>,
    // Albatross
    pub block_albatross: RwLock<PassThroughNotifier<'static, Block>>,
    pub header_albatross: RwLock<PassThroughNotifier<'static, BlockHeader>>,
    pub fork_proof: RwLock<PassThroughNotifier<'static, ForkProof>>,
    pub view_change:
        RwLock<PassThroughNotifier<'static, LevelUpdateMessage<MultiSignature, ViewChange>>>,
    pub view_change_proof: RwLock<PassThroughNotifier<'static, ViewChangeProofMessage>>,
    pub get_macro_blocks: RwLock<PassThroughNotifier<'static, GetBlocksMessage>>,
    pub get_epoch_transactions: RwLock<PassThroughNotifier<'static, GetEpochTransactionsMessage>>,
    pub epoch_transactions: RwLock<PassThroughNotifier<'static, EpochTransactionsMessage>>,
}

impl MessageNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notify(&self, msg: Message) {
        match msg {
            Message::Version(msg) => self.version.read().notify(*msg),
            Message::VerAck(msg) => self.ver_ack.read().notify(*msg),
            Message::Inv(vector) => self.inv.read().notify(vector),
            Message::GetData(vector) => self.get_data.read().notify(vector),
            Message::GetHeader(vector) => self.get_header.read().notify(vector),
            Message::NotFound(vector) => self.not_found.read().notify(vector),
            Message::Tx(msg) => self.tx.read().notify(*msg),
            Message::GetBlocks(msg) => self.get_blocks.read().notify(*msg),
            Message::Mempool => self.mempool.read().notify(()),
            Message::Reject(msg) => self.reject.read().notify(*msg),
            Message::Subscribe(msg) => self.subscribe.read().notify(*msg),
            Message::Addr(msg) => self.addr.read().notify(*msg),
            Message::GetAddr(msg) => self.get_addr.read().notify(*msg),
            Message::Ping(nonce) => self.ping.read().notify(nonce),
            Message::Pong(nonce) => self.pong.read().notify(nonce),
            Message::Signal(msg) => self.signal.read().notify(*msg),
            Message::GetAccountsProof(msg) => self.get_accounts_proof.read().notify(*msg),
            Message::AccountsProof(msg) => self.accounts_proof.read().notify(*msg),
            Message::GetAccountsTreeChunk(msg) => self.get_accounts_tree_chunk.read().notify(*msg),
            Message::AccountsTreeChunk(msg) => self.accounts_tree_chunk.read().notify(*msg),
            Message::GetTransactionsProof(msg) => self.get_transactions_proof.read().notify(*msg),
            Message::TransactionsProof(msg) => self.transactions_proof.read().notify(*msg),
            Message::GetTransactionReceipts(msg) => {
                self.get_transaction_receipts.read().notify(*msg)
            }
            Message::TransactionReceipts(msg) => self.transaction_receipts.read().notify(*msg),
            Message::GetHead => self.get_head.read().notify(()),
            // Albatross
            Message::BlockAlbatross(block) => self.block_albatross.read().notify(*block),
            Message::HeaderAlbatross(header) => self.header_albatross.read().notify(*header),
            Message::ViewChange(view_change) => self.view_change.read().notify(*view_change),
            Message::ViewChangeProof(view_change_proof) => {
                self.view_change_proof.read().notify(*view_change_proof)
            }
            Message::ForkProof(fork_proof) => self.fork_proof.read().notify(*fork_proof),
            Message::GetMacroBlocks(msg) => self.get_macro_blocks.read().notify(*msg),
            Message::GetEpochTransactions(msg) => self.get_epoch_transactions.read().notify(*msg),
            Message::EpochTransactions(msg) => self.epoch_transactions.read().notify(*msg),
        }
    }
}

pub trait MessageAdapter<B, H> {
    fn register_block_listener<T: PassThroughListener<B> + 'static>(
        notifier: &MessageNotifier,
        listener: T,
    );
    fn register_header_listener<T: PassThroughListener<H> + 'static>(
        notifier: &MessageNotifier,
        listener: T,
    );
    fn new_block_message(block: B) -> Message;
    fn new_header_message(header: H) -> Message;
}

pub struct AlbatrossMessageAdapter {}
impl MessageAdapter<Block, BlockHeader> for AlbatrossMessageAdapter {
    fn register_block_listener<T: PassThroughListener<Block> + 'static>(
        notifier: &MessageNotifier,
        listener: T,
    ) {
        notifier.block_albatross.write().register(listener)
    }

    fn register_header_listener<T: PassThroughListener<BlockHeader> + 'static>(
        notifier: &MessageNotifier,
        listener: T,
    ) {
        notifier.header_albatross.write().register(listener)
    }

    fn new_block_message(block: Block) -> Message {
        Message::BlockAlbatross(Box::new(block))
    }

    fn new_header_message(header: BlockHeader) -> Message {
        Message::HeaderAlbatross(Box::new(header))
    }
}

create_typed_array!(ChallengeNonce, u8, 32);
add_hex_io_fns_typed_arr!(ChallengeNonce, ChallengeNonce::SIZE);

impl ChallengeNonce {
    pub fn generate() -> Self {
        let mut nonce = Self::default();
        OsRng.fill(&mut nonce.0);
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
                Err(SerializingError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => None,
                Err(e) => return Err(e),
            },
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
        if let Some(u) = &self.user_agent {
            size += SerializeWithLength::serialize::<u8, W>(u, writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.version);
        size += Serialize::serialized_size(&self.peer_address);
        size += Serialize::serialized_size(&self.genesis_hash);
        size += Serialize::serialized_size(&self.head_hash);
        size += Serialize::serialized_size(&self.challenge_nonce);
        if let Some(u) = &self.user_agent {
            size += SerializeWithLength::serialized_size::<u8>(u);
        }
        size
    }
}

impl VersionMessage {
    pub fn new(
        peer_address: PeerAddress,
        head_hash: Blake2bHash,
        genesis_hash: Blake2bHash,
        challenge_nonce: ChallengeNonce,
        user_agent: Option<String>,
    ) -> Message {
        Message::Version(Box::new(Self {
            version: version::CODE,
            peer_address,
            genesis_hash,
            head_hash,
            challenge_nonce,
            user_agent,
        }))
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

    pub fn from_block_hash(hash: Blake2bHash) -> Self {
        Self::new(InvVectorType::Block, hash)
    }

    pub fn from_tx_hash(hash: Blake2bHash) -> Self {
        Self::new(InvVectorType::Transaction, hash)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxMessage {
    pub transaction: Transaction,
    pub accounts_proof: Option<AccountsProof<Account>>,
}
impl TxMessage {
    pub fn new(transaction: Transaction) -> Message {
        Message::Tx(Box::new(Self {
            transaction,
            accounts_proof: None,
        }))
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

    pub fn new(
        locators: Vec<Blake2bHash>,
        max_inv_size: u16,
        direction: GetBlocksDirection,
    ) -> Message {
        Message::GetBlocks(Box::new(Self {
            locators,
            max_inv_size,
            direction,
        }))
    }

    pub fn new_with_macro(
        locators: Vec<Blake2bHash>,
        max_inv_size: u16,
        direction: GetBlocksDirection,
    ) -> Message {
        Message::GetMacroBlocks(Box::new(Self {
            locators,
            max_inv_size,
            direction,
        }))
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
    pub fn new(
        message_type: MessageType,
        code: RejectMessageCode,
        reason: String,
        extra_data: Option<Vec<u8>>,
    ) -> Message {
        Message::Reject(Box::new(Self {
            message_type,
            code,
            reason,
            extra_data: extra_data.unwrap_or_else(Vec::new),
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddrMessage {
    #[beserial(len_type(u16))]
    pub addresses: Vec<PeerAddress>,
}

impl AddrMessage {
    pub fn new(addresses: Vec<PeerAddress>) -> Message {
        Message::Addr(Box::new(Self { addresses }))
    }
}

#[derive(Clone, Debug)]
pub struct GetAddrMessage {
    pub protocol_mask: ProtocolFlags,
    pub service_mask: ServiceFlags,
    pub max_results: Option<u16>,
}

impl GetAddrMessage {
    pub fn new(
        protocol_mask: ProtocolFlags,
        service_mask: ServiceFlags,
        max_results: Option<u16>,
    ) -> Message {
        Message::GetAddr(Box::new(Self {
            protocol_mask,
            service_mask,
            max_results,
        }))
    }
}

impl Serialize for GetAddrMessage {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.protocol_mask.bits().serialize(writer)?;
        size += self.service_mask.bits().serialize(writer)?;
        if let Some(max_results) = self.max_results {
            size += max_results.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.protocol_mask.bits().serialized_size();
        size += self.service_mask.bits().serialized_size();
        if let Some(max_results) = self.max_results {
            size += max_results.serialized_size();
        }
        size
    }
}

impl Deserialize for GetAddrMessage {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let protocol_mask = ProtocolFlags::from_bits_truncate(Deserialize::deserialize(reader)?);
        let service_mask = ServiceFlags::from_bits_truncate(Deserialize::deserialize(reader)?);
        let max_results = Deserialize::deserialize(reader).ok();
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
        const UNROUTABLE  = 0b0000_0001;
        const TTL_EXCEEDED = 0b0000_0010;
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
        let sender_public_key = if !payload.is_empty() {
            Some(Deserialize::deserialize(reader)?)
        } else {
            None
        };
        let signature = if !payload.is_empty() {
            Some(Deserialize::deserialize(reader)?)
        } else {
            None
        };

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
        if !self.payload.is_empty() {
            size += Serialize::serialize(&self.sender_public_key.unwrap(), writer)?;
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
        if !self.payload.is_empty() {
            size += Serialize::serialized_size(&self.sender_public_key.unwrap());
            size += self
                .signature
                .as_ref()
                .map(Serialize::serialized_size)
                .unwrap();
        }
        size
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetAccountsProofMessage {
    pub block_hash: Blake2bHash,
    #[beserial(len_type(u16))]
    pub addresses: Vec<Address>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsProofMessage {
    pub block_hash: Blake2bHash,
    pub proof: Option<AccountsProof<Account>>,
}

impl AccountsProofMessage {
    pub fn new(block_hash: Blake2bHash, proof: Option<AccountsProof<Account>>) -> Message {
        Message::AccountsProof(Box::new(AccountsProofMessage { block_hash, proof }))
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
    Structured(AccountsTreeChunk<Account>),
}

impl AccountsTreeChunkData {
    pub fn into_serialized(self) -> Self {
        match self {
            data @ AccountsTreeChunkData::Serialized(_) => data,
            AccountsTreeChunkData::Structured(chunk) => {
                AccountsTreeChunkData::Serialized(chunk.serialize_to_vec())
            }
        }
    }
}

impl Serialize for AccountsTreeChunkData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            AccountsTreeChunkData::Serialized(ref buf) => {
                writer.write_all(buf.as_slice())?;
                Ok(buf.len())
            }
            AccountsTreeChunkData::Structured(ref chunk) => chunk.serialize(writer),
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            AccountsTreeChunkData::Serialized(ref buf) => buf.len(),
            AccountsTreeChunkData::Structured(ref chunk) => chunk.serialized_size(),
        }
    }
}

impl Deserialize for AccountsTreeChunkData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(AccountsTreeChunkData::Structured(Deserialize::deserialize(
            reader,
        )?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeChunkMessage {
    pub block_hash: Blake2bHash,
    pub chunk: Option<AccountsTreeChunkData>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetTransactionsProofMessage {
    pub block_hash: Blake2bHash,
    #[beserial(len_type(u16))]
    pub addresses: Vec<Address>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionsProofMessage {
    pub block_hash: Blake2bHash,
    pub transactions_proof: Option<TransactionsProof>,
}

impl TransactionsProofMessage {
    pub fn new(block_hash: Blake2bHash, transactions_proof: Option<TransactionsProof>) -> Message {
        Message::TransactionsProof(Box::new(TransactionsProofMessage {
            block_hash,
            transactions_proof,
        }))
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
        Message::TransactionReceipts(Box::new(TransactionReceiptsMessage {
            receipts: Some(receipts),
        }))
    }

    pub fn empty() -> Message {
        Message::TransactionReceipts(Box::new(TransactionReceiptsMessage { receipts: None }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerAckMessage {
    pub public_key: PublicKey,
    pub signature: Signature,
}

impl VerAckMessage {
    pub fn new(
        peer_id: &PeerId,
        peer_challenge_nonce: &ChallengeNonce,
        key_pair: &KeyPair,
    ) -> Message {
        let mut data = peer_id.serialize_to_vec();
        peer_challenge_nonce.serialize(&mut data).unwrap();
        let signature = key_pair.sign(&data[..]);
        Message::VerAck(Box::new(Self {
            public_key: key_pair.public,
            signature,
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewChangeProofMessage {
    pub view_change: ViewChange,
    pub proof: ViewChangeProof,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetEpochTransactionsMessage {
    pub epoch: u32,
}
impl GetEpochTransactionsMessage {
    pub fn new(epoch: u32) -> Message {
        Message::GetEpochTransactions(Box::new(Self { epoch }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EpochTransactionsMessage {
    pub epoch: u32,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    pub tx_proof: Blake2bPartialMerkleProof,
}
impl EpochTransactionsMessage {
    pub const MAX_TRANSACTIONS: usize = 1000;

    pub fn new(
        epoch: u32,
        transactions: Vec<Transaction>,
        tx_proof: Blake2bPartialMerkleProof,
    ) -> Message {
        Message::EpochTransactions(Box::new(Self {
            epoch,
            transactions,
            tx_proof,
        }))
    }
}
