use anyhow::Error;
use nimiq_jsonrpc_client::{
    websocket::WebsocketClient, ArcClient, Client as RPCclient, Credentials,
};
// Re-export the interfaces and types from the RPC interface subcrate
pub use nimiq_rpc_interface::{
    blockchain::BlockchainInterface, consensus::ConsensusInterface, mempool::MempoolInterface,
    network::NetworkInterface, oracle::OracleInterface, policy::PolicyInterface, types,
    validator::ValidatorInterface, wallet::WalletInterface, zkp_component::ZKPComponentInterface,
};
use nimiq_rpc_interface::{
    blockchain::BlockchainProxy, consensus::ConsensusProxy, mempool::MempoolProxy,
    network::NetworkProxy, oracle::OracleProxy, policy::PolicyProxy, validator::ValidatorProxy,
    wallet::WalletProxy, zkp_component::ZKPComponentProxy,
};
use url::Url;

/// RPC Client that exposes functionality trough its various proxys.
/// The interface that is implemented by each of its components is defined in
/// nimiq-rpc-interface
pub struct Client {
    /// The websocket client
    pub ws_client: ArcClient<WebsocketClient>,
    /// Policy Proxy used to handle policy requests
    pub policy: PolicyProxy<ArcClient<WebsocketClient>>,
    /// Blockchain Proxy used to handle blockchain requests
    pub blockchain: BlockchainProxy<ArcClient<WebsocketClient>>,
    /// Consensus Proxy used to handle consensus requests
    pub consensus: ConsensusProxy<ArcClient<WebsocketClient>>,
    /// Mempool Proxy used to handle mempool requests
    pub mempool: MempoolProxy<ArcClient<WebsocketClient>>,
    /// Wallet Proxy used to handle wallet requests
    pub wallet: WalletProxy<ArcClient<WebsocketClient>>,
    /// Validator Proxy used to handle validator requests
    pub validator: ValidatorProxy<ArcClient<WebsocketClient>>,
    /// Network Proxy used to handle network requests
    pub network: NetworkProxy<ArcClient<WebsocketClient>>,
    /// ZKP Proxy used to handle zkp requests
    pub zkp_component: ZKPComponentProxy<ArcClient<WebsocketClient>>,
    /// Oracle Proxy used to handle oracle contract requests
    pub oracle: OracleProxy<ArcClient<WebsocketClient>>,
}

impl Client {
    /// Establishes a new connection to a RPC server
    pub async fn new(url: Url, credentials: Option<Credentials>) -> Result<Self, Error> {
        let client = ArcClient::new(WebsocketClient::new(url, credentials).await?);

        Ok(Self {
            policy: PolicyProxy::new(client.clone()),
            blockchain: BlockchainProxy::new(client.clone()),
            consensus: ConsensusProxy::new(client.clone()),
            mempool: MempoolProxy::new(client.clone()),
            wallet: WalletProxy::new(client.clone()),
            validator: ValidatorProxy::new(client.clone()),
            network: NetworkProxy::new(client.clone()),
            zkp_component: ZKPComponentProxy::new(client.clone()),
            oracle: OracleProxy::new(client.clone()),
            ws_client: client,
        })
    }

    /// Closes the WS connection
    pub async fn close(&mut self) {
        self.ws_client.close().await;
    }
}
