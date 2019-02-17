use failure::Fail;

use consensus::error::Error as ConsensusError;
use network::error::Error as NetworkError;


/// Prototype for a Error returned by these futures
/// Errors can occur, when e.g. the bind port is already used
#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "{}", _0)]
    NetworkError(#[cause] NetworkError),
    #[fail(display = "Reverse Proxy can only be configured on Ws")]
    ConfigureReverseProxyError,
    #[fail(display = "{}", _0)]
    ConsensusError(#[cause] ConsensusError),
    #[fail(display = "Rtc is not implemented")]
    RtcNotImplemented,
    #[fail(display = "Protocol expects a hostname")]
    MissingHostname,
    #[fail(display = "Protocol expects a port")]
    MissingPort,
    #[fail(display = "Protocol doesn't expect a hostname")]
    UnexpectedHostname,
    #[fail(display = "Protocol doesn't expect a port")]
    UnexpectedPort,
    #[fail(display = "TLS identity file is missing")]
    MissingIdentityFile,

}

impl From<NetworkError> for ClientError {
    fn from(e: NetworkError) -> Self {
        ClientError::NetworkError(e)
    }
}

impl From<ConsensusError> for ClientError {
    fn from(e: ConsensusError) -> Self {
        ClientError::ConsensusError(e)
    }
}