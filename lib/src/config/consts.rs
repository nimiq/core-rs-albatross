use std::net::{IpAddr, Ipv4Addr};

/// The default port for `ws` and `wss`.
pub const WS_DEFAULT_PORT: u16 = 8443;

/// The default port for the reverse proxy
pub const REVERSE_PROXY_DEFAULT_PORT: u16 = 8444;

/// The default port for the RPC server
pub const RPC_DEFAULT_PORT: u16 = 8648;

/// The default port for the metrics server
pub const METRICS_DEFAULT_PORT: u16 = 8649;

/// Returns the default bind, i.e. localhost
pub fn default_bind() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
}
