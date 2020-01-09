use ws_rpc_server::WsRpcServer;

use crate::error::Error;
use crate::client::Client;
use crate::config::config::WsRpcServerConfig;
use crate::config::consts::default_bind;

pub async fn initialize_ws_rpc_server(client: &Client, config: WsRpcServerConfig) -> Result<WsRpcServer, Error> {
    let ip = config.bind_to.unwrap_or_else(default_bind);

    info!("Initializing websocket RPC server: {}:{}", ip, config.port);

    let server = WsRpcServer::new(ip, config.port).await?;
    server.register_blockchain(client.consensus());
    #[cfg(feature="validator")] {
        if let Some(validator) = client.validator() {
            server.register_validator(validator)
        }
    }

    Ok(server)
}
