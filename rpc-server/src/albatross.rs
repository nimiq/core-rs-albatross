use std::sync::Arc;

use parking_lot::RwLock;

use consensus::{Consensus, AlbatrossConsensusProtocol};

use crate::{AbstractRpcHandler, JsonRpcConfig, JsonRpcServerState};
use crate::common::RpcHandler;
use crate::handlers::blockchain_albatross::BlockchainAlbatrossHandler;
use crate::handlers::Handler;
use crate::handlers::mempool_albatross::MempoolAlbatrossHandler;
use crate::handlers::network::NetworkHandler;
use crate::handlers::wallet::WalletHandler;

impl AbstractRpcHandler<AlbatrossConsensusProtocol> for RpcHandler {
    fn new(consensus: Arc<Consensus<AlbatrossConsensusProtocol>>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self {
        let mut handlers: Vec<Box<dyn Handler>> = Vec::new();
        let wallet_handler = WalletHandler::new(consensus.env);
        handlers.push(Box::new(BlockchainAlbatrossHandler::new(consensus.blockchain.clone())));
        handlers.push(Box::new(MempoolAlbatrossHandler::new(consensus.mempool.clone(), Some(wallet_handler.unlocked_wallets.clone()))));
        handlers.push(Box::new(wallet_handler));
        handlers.push(Box::new(NetworkHandler::new(&consensus, state)));

        RpcHandler {
            handlers,
            config,
        }
    }
}
