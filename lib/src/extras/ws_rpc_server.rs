use ws_rpc_server::WsRpcServer;
use consensus::ConsensusProtocol;

use crate::error::Error;
use crate::client::Client;
use crate::extras::block_producer::BlockProducerFactory;
use crate::config::config::WsRpcServerConfig;
use crate::config::consts::default_bind;

pub fn initialize_ws_rcp_server<P: ConsensusProtocol + BlockProducerFactory>(client: &Client<P>, config: WsRpcServerConfig) -> Result<WsRpcServer, Error> {
    let ip = config.bind_to.unwrap_or_else(default_bind);

    info!("Initializing websocket RPC server: {}:{}", ip, config.port);

    let server = WsRpcServer::new(ip, config.port)?;
    server.register_blockchain(client.consensus());
    #[cfg(feature="validator")] {
        if let Some(validator) = client.validator() {
            server.register_validator(validator)
        }
    }

    Ok(server)
}
