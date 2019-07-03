use std::sync::Arc;

use json::{Array, JsonValue, Null};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use consensus::{Consensus, NimiqConsensusProtocol};

use crate::{AbstractRpcHandler, JsonRpcConfig, JsonRpcServerState};
use crate::common::RpcHandler;
use crate::error::AuthenticationError;
use crate::handlers::block_production::BlockProductionHandler;
use crate::handlers::blockchain::BlockchainHandler;
use crate::handlers::Handler;
use crate::handlers::mempool::MempoolHandler;
use crate::handlers::network::NetworkHandler;
use crate::handlers::wallet::WalletHandler;

impl AbstractRpcHandler<NimiqConsensusProtocol> for RpcHandler {
    fn new(consensus: Arc<Consensus<NimiqConsensusProtocol>>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self {
        let mut handlers: Vec<Box<Handler>> = Vec::new();
        let wallet_handler = WalletHandler::new(consensus.env);
        handlers.push(Box::new(BlockchainHandler::new(consensus.blockchain.clone())));
        handlers.push(Box::new(MempoolHandler::<NimiqConsensusProtocol>::new(consensus.mempool.clone(), Some(wallet_handler.unlocked_wallets.clone()))));
        handlers.push(Box::new(wallet_handler));
        handlers.push(Box::new(NetworkHandler::new(&consensus, state)));
        handlers.push(Box::new(BlockProductionHandler::new(
                consensus.blockchain.clone(),
                consensus.mempool.clone())
        ));

        RpcHandler {
            handlers,
            config,
        }
    }
}
