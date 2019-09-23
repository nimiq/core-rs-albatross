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
pub mod block_production_albatross;
pub mod blockchain;
pub mod blockchain_nimiq;
pub mod blockchain_albatross;
pub mod mempool;
pub mod mempool_albatross;
pub mod network;
pub mod wallet;

pub trait Module: Send + Sync {
    fn methods(self) -> Vec<(&'static str, Method)>;
}
