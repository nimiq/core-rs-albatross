use std::sync::Arc;

use json::{Array, JsonValue, Null};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use consensus::{Consensus, AlbatrossConsensusProtocol};

use crate::{AbstractRpcHandler, JsonRpcConfig, JsonRpcServerState};
use crate::common::RpcHandler;
use crate::error::AuthenticationError;
use crate::handlers::blockchain_albatross::BlockchainAlbatrossHandler;
use crate::handlers::Handler;
use crate::handlers::mempool::MempoolHandler;
use crate::handlers::network::NetworkHandler;
use crate::handlers::wallet::WalletHandler;

impl AbstractRpcHandler<AlbatrossConsensusProtocol> for RpcHandler {
    fn new(consensus: Arc<Consensus<AlbatrossConsensusProtocol>>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self {
        let mut handlers: Vec<Box<Handler>> = Vec::new();
        handlers.push(Box::new(BlockchainAlbatrossHandler::new(consensus.blockchain.clone())));
        handlers.push(Box::new(MempoolHandler::<AlbatrossConsensusProtocol>::new(consensus.mempool.clone())));
        handlers.push(Box::new(WalletHandler::new(consensus.env)));
        handlers.push(Box::new(NetworkHandler::new(&consensus, state)));

        RpcHandler {
            handlers,
            config,
        }
    }
}
