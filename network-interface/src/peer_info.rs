use bitflags::bitflags;
use multiaddr::Multiaddr;
use nimiq_serde::{Deserialize, Serialize};

bitflags! {
    /// Bitmask of services
    ///
    ///
    ///  - This just serializes to its numeric value for serde, but a list of strings would be nicer.
    ///
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    pub struct Services: u32 {
        /// The node provides at least the latest [`nimiq_primitives::policy::NUM_BLOCKS_VERIFICATION`] as full blocks.
        const FULL_BLOCKS = 1 << 0;

        /// The node provides the full transaction history.
        const HISTORY = 1 << 1;

        /// The node provides inclusion and exclusion proofs for accounts that are necessary to verify active accounts as
        /// well as accounts in all transactions it provided from its mempool.
        ///
        /// However, if [`Services::ACCOUNTS_CHUNKS`] is not set, the node may occasionally not provide a proof if it
        /// decided to prune the account from local storage.
        const ACCOUNTS_PROOF = 1 << 3;

        /// The node provides the full accounts tree in form of chunks.
        /// This implies that the client stores the full accounts tree.
        const ACCOUNTS_CHUNKS = 1 << 4;

        /// The node tries to stay on sync with the network wide mempool and will provide access to it.
        ///
        /// Nodes that do not have this flag set may occasionally announce transactions from their mempool and/or reply to
        /// mempool requests to announce locally crafted transactions.
        const MEMPOOL = 1 << 5;

        /// The node provides an index of transactions allowing it to find historic transactions by address or by hash.
        /// Only history nodes will have this flag set.
        /// Nodes that have this flag set may prune any part of their transaction index at their discretion, they do not
        /// claim completeness of their results either.
        const TRANSACTION_INDEX = 1 << 6;

        /// This node is configured as a validator, so it is interested for other validator nodes.
        const VALIDATOR = 1 << 7;
    }
}

/// Enumeration for the different node types
pub enum NodeType {
    /// History node type
    History,
    /// Light node type
    Light,
    /// Full node type
    Full,
}

impl Services {
    /// Common provided service flags for a node
    pub fn provided(node_type: NodeType) -> Self {
        match node_type {
            NodeType::History => {
                Services::HISTORY
                    | Services::FULL_BLOCKS
                    | Services::ACCOUNTS_PROOF
                    | Services::ACCOUNTS_CHUNKS
                    | Services::TRANSACTION_INDEX
            }
            NodeType::Light => Services::empty(),
            NodeType::Full => Services::ACCOUNTS_PROOF | Services::FULL_BLOCKS,
        }
    }

    /// Common required service flags for a node
    pub fn required(node_type: NodeType) -> Self {
        match node_type {
            NodeType::History => Services::HISTORY | Services::FULL_BLOCKS,
            NodeType::Light => Services::ACCOUNTS_PROOF,
            NodeType::Full => Services::FULL_BLOCKS | Services::ACCOUNTS_CHUNKS,
        }
    }
}

/// Peer information. This struct contains:
///
///  - The connection address of the peer.
///  - A bitmask of the services supported by this peer.
///
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerInfo {
    /// Connection address of this peer.
    address: Multiaddr,

    /// Services supported by this peer.
    services: Services,
}

impl PeerInfo {
    pub fn new(address: Multiaddr, services: Services) -> Self {
        Self { address, services }
    }

    /// Gets the peer connection address
    pub fn get_address(&self) -> Multiaddr {
        self.address.clone()
    }

    /// Gets the peer provided services
    pub fn get_services(&self) -> Services {
        self.services
    }
}
