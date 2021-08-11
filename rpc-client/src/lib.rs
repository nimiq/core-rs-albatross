pub use url::Url;

pub use nimiq_jsonrpc_client::websocket::Error;
use nimiq_jsonrpc_client::{websocket::WebsocketClient, ArcClient};
pub use nimiq_jsonrpc_core::Credentials;

pub use nimiq_rpc_interface::{
    blockchain::BlockchainProxy, consensus::ConsensusProxy, mempool::MempoolProxy,
    wallet::WalletProxy,
};

pub struct Client {
    pub blockchain: BlockchainProxy<ArcClient<WebsocketClient>>,
    pub consensus: ConsensusProxy<ArcClient<WebsocketClient>>,
    pub mempool: MempoolProxy<ArcClient<WebsocketClient>>,
    pub wallet: WalletProxy<ArcClient<WebsocketClient>>,
}

impl Client {
    pub async fn new(url: Url, credentials: Option<Credentials>) -> Result<Self, Error> {
        let client = ArcClient::new(WebsocketClient::new(url, credentials).await?);

        Ok(Self {
            blockchain: BlockchainProxy::new(client.clone()),
            consensus: ConsensusProxy::new(client.clone()),
            mempool: MempoolProxy::new(client.clone()),
            wallet: WalletProxy::new(client),
        })
    }
}
