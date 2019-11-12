use crate::handler::Method;

// Generates an RPC method vec from map syntax
#[macro_export]
macro_rules! rpc_module_methods {
    // trailing comma
    ( $( $k:expr => $($v:ident).+ , )* ) => (
        fn methods(self) -> Vec<(&'static str, Method)> {
            let this_base = Arc::new(self);
            let mut vec = Vec::new();
            $(
                let this = Arc::clone(&this_base);
                let method = Method::new(move |params| this. $($v).+ (params));
                vec.push(($k, method));
            )*
            vec
        }
    );

    // no trailing comma
    ( $( $k:expr => $($v:ident).+ ),* ) => (
        rpc_module_methods!( $( $k => $($v).+ ,)* );
    );
}

pub mod consensus;
pub mod block_production_nimiq;
#[cfg(feature="validator")]
pub mod block_production_albatross;
pub mod blockchain;
pub mod blockchain_nimiq;
pub mod blockchain_albatross;
pub mod mempool;
pub mod mempool_albatross;
pub mod network;
pub mod wallet;


pub use self::consensus::ConsensusHandler;
pub use self::block_production_nimiq::BlockProductionNimiqHandler;
#[cfg(feature="validator")]
pub use self::block_production_albatross::BlockProductionAlbatrossHandler;
pub use self::blockchain::BlockchainHandler;
pub use self::blockchain_nimiq::BlockchainNimiqHandler;
pub use self::blockchain_albatross::BlockchainAlbatrossHandler;
pub use self::mempool::MempoolHandler;
pub use self::mempool_albatross::MempoolAlbatrossHandler;
pub use self::network::NetworkHandler;
pub use self::wallet::{WalletHandler, UnlockedWalletManager};


pub trait Module: Send + Sync {
    fn methods(self) -> Vec<(&'static str, Method)>;
}

