use tungstenite::protocol::frame::coding::CloseCode;

use beserial::Deserialize;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[repr(u16)]
pub enum CloseType {
    Unknown = 0,
    GetBlocksTimeout = 1,
    GetChainProofTimeout = 2,
    GetAccountsTreeChunkTimeout = 3,
    GetHeaderTimeout = 4,
    InvalidAccountsTreeChunk = 5,
    AccountsTreeChunckRootHashMismatch = 6,
    ReceivedWrongHeader = 8,
    DidNotGetRequestedHeader = 9,

    GetAccountsProofTimeout = 11,
    GetTransactionsProofTimeout = 12,
    GetTransactionReceiptsTimeout = 13,
    InvalidAccountsProof = 14,
    AccountsProofRootHashMismatch = 15,
    IncompleteAccountsProof = 16,
    InvalidBlock = 17,
    InvalidChainProof = 18,
    InvalidTransactionProof = 19,
    InvalidBlockProof = 20,

    SendFailed = 21,
    SendingPingMessageFailed = 22,
    SendingVersionMessageFailed = 23,

    SimultaneousConnection = 29,
    DuplicateConnection = 30,
    PeerIsBanned = 31,
    ManualNetworkDisconnect = 33,
    ManualWebsocketDisconnect = 34,
    MaxPeerCountReached = 35,

    PeerConnectionRecycled = 36,
    PeerConnectionRecycledInboundExchange = 37,
    InboundConnectionsBlocked = 38,

    InvalidConnectionState = 40,

    ManualPeerDisconnect = 90,

    // Ban Close Types

    ReceivedInvalidBlock = 100,
    BlockchainSyncFailed = 101,
    ReceivedInvalidHeader = 102,
    ReceivedTransactionNotMatchingOurSubscription = 103,
    AddrMessageTooLarge = 104,
    InvalidAddr = 105,
    AddrNotGloballyReachable = 106,
    InvalidSignalTtl = 107,
    InvalidSignature = 108,
    ReceivedBlockNotMatchingOurSubscription = 109,

    IncompatibleVersion = 110,
    DifferentGenesisBlock = 111,
    InvalidPeerAddressInVersionMessage = 112,
    UnexpectedPeerAddressInVersionMessage = 113,
    InvalidPublicKeyInVerackMessage = 114,
    InvalidSignatureInVerackMessage = 115,
    BannedIp = 116,

    UnexpectedEpochTransactions = 117,
    InvalidEpochTransactions = 118,

    RateLimitExceeded = 120,

    ManualPeerBan = 190,

    // Fail Close Types

    ClosedByRemote = 200,
    PingTimeout = 201,
    ConnectionFailed = 202,
    NetworkError = 203,
    VersionTimeout = 204,
    VerackTimeout = 205,
    AbortedSync = 206,
    FailedToParseMessageType = 207,
    ConnectionLimitPerIp = 208,
    ChannelClosing = 209,
    ConnectionLimitDumb = 210,

    ManualPeerFail = 290,
}

impl CloseType {
    pub fn is_banning_type(self) -> bool {
        (self as u16) >= 100 && (self as u16) < 200
    }

    pub fn is_failing_type(self) -> bool {
        (self as u16) >= 200
    }
}

impl Into<CloseCode> for CloseType {
    fn into(self) -> CloseCode {
        CloseCode::Library(4000 + (self as u16)) // Library specific is 4000-4999.
    }
}

impl From<CloseCode> for CloseType {
    fn from(code: CloseCode) -> Self {
        match code {
            CloseCode::Library(code) => Deserialize::deserialize_from_vec(&(code - 4000).to_be_bytes().to_vec()).unwrap_or(CloseType::Unknown),
            _ => CloseType::Unknown,
        }
    }
}

impl From<Option<CloseCode>> for CloseType {
    fn from(code: Option<CloseCode>) -> Self {
        match code {
            Some(code) => code.into(),
            _ => CloseType::Unknown,
        }
    }
}
