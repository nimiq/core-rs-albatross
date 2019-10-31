use std::convert::TryFrom;
use std::sync::{Arc, Weak};

#[cfg(feature="validator")]
use validator::validator::Validator;
use consensus::{
    Consensus as AbstractConsensus,
    AlbatrossConsensusProtocol,
};

use crate::error::Error;
use crate::config::ClientConfig;


/// Alias for the Consensus specialized over Albatross
pub type Consensus = AbstractConsensus<AlbatrossConsensusProtocol>;


/// Holds references to the relevant structs. This is then Arc'd in `Client` and a nice API is
/// exposed.
struct _Client {
    consensus: Arc<Consensus>,
    #[cfg(feature="validator")]
    validator: Option<Arc<Validator>>
}


impl TryFrom<ClientConfig> for _Client {
    type Error = Error;

    fn try_from(config: ClientConfig) -> Result<Self, Self::Error> {
        let consensus = unimplemented!();
        let validator = unimplemented!();

        Ok(_Client {
            consensus,
            validator,
        })
    }
}



/// Entry point for the Nimiq client API
pub struct Client {
    inner: Arc<_Client>
}


impl Client {
    /// Returns a reference to the *Consensus*.
    pub fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.inner.consensus)
    }

    /// Returns a reference to the *Validator* or `None`.
    #[cfg(feature="validator")]
    pub fn validator(&self) -> Option<Arc<Validator>> {
        self.inner.validator.as_ref().map(|v| Arc::clone(v))
    }

    fn weak(&self) -> Weak<_Client> {
        Arc::downgrade(&self.inner)
    }
}

impl TryFrom<ClientConfig> for Client {
    type Error = Error;

    fn try_from(config: ClientConfig) -> Result<Self, Self::Error> {
        let inner = _Client::try_from(config)?;
        Ok(Client { inner: Arc::new(inner) })
    }
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client { inner: Arc::clone(&self.inner)}
    }
}
