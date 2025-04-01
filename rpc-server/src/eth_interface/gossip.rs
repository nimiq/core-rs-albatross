use async_trait::async_trait;

use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::ConsensusProxy;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_libp2p::Network;
use nimiq_rpc_interface::eth_interface::gossip::GossipInterface;
use nimiq_rpc_interface::types::RPCResult;

use nimiq_serde::Deserialize;
use nimiq_transaction::Transaction;

use crate::error::Error;

pub struct GossipDispatcher {
    pub consensus: ConsensusProxy<Network>,
    pub blockchain: BlockchainProxy,
}

impl GossipDispatcher {
    pub fn new(consensus: ConsensusProxy<Network>, blockchain: BlockchainProxy) -> Self {
        Self {
            consensus,
            blockchain,
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl GossipInterface for GossipDispatcher {
    type Error = Error;

    async fn eth_blockNumber(&mut self) -> RPCResult<u32, (), Self::Error> {
        Ok(self.blockchain.read().block_number().into())
    }

    async fn eth_sendRawTransaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let tx = Transaction::deserialize_from_vec(&hex::decode(&raw_tx)?)?;
        let txid = tx.hash::<Blake2bHash>();

        match self.consensus.send_transaction(tx).await {
            Ok(_) => Ok(txid.into()),
            Err(e) => Err(Error::NetworkError(e)),
        }
    }
}
