use metrics_server::MetricsServer;
use metrics_server::error::Error;
use metrics_server::AlbatrossChainMetrics;

use crate::config::config::MetricsServerConfig;
use crate::client::Client;
use crate::config::consts::default_bind;


pub fn initialize_metrics_server(client: &Client, config: MetricsServerConfig) -> Result<MetricsServer, Error> {
    let (username, password) = if let Some(credentials) = config.credentials {
        (Some(credentials.username), Some(credentials.password))
    } else {
        (None, None)
    };

     Ok(MetricsServer::new::<_, AlbatrossChainMetrics>(
        config.bind_to.unwrap_or_else(default_bind),
        config.port,
        username,
        password,
        client.consensus()
    )?)
}
