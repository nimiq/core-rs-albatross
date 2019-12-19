use std::net::Ipv4Addr;

use crate::config::config::MetricsServerConfig;
use crate::client::Client;
use crate::error::Error;

use metrics_server::metrics_server;
use metrics_server::metrics::AlbatrossChainMetrics;


impl Client {
    pub async fn metrics_server(self, config: MetricsServerConfig) -> Result<(), Error> {
        let consensus = self.consensus();

        let tls = config.tls_credentials
            .ok_or_else(|| Error::config_error("TLS credentials for metrics server missing."))?;
        let key_file = tls.key_file.to_str()
            .ok_or_else(|| Error::config_error(format!("Invalid key file path: {}", tls.key_file.display())))?;

        let (username, password) = if let Some(credentials) = config.credentials {
            (Some(credentials.username), Some(credentials.password))
        }
        else {
            (None, None)
        };

        metrics_server::<_, AlbatrossChainMetrics>(
            config.bind_to.unwrap_or_else(|| Ipv4Addr::new(127, 0, 0, 1).into()),
            config.port,
            username,
            password,
            key_file,
            &tls.passphrase,
            consensus,
        ).await?;

        Ok(())
    }
}