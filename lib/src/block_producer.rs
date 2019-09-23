use std::sync::Arc;

use consensus::{ConsensusProtocol, Consensus};
use crate::error::ClientError;


pub trait BlockProducer<P: ConsensusProtocol + 'static>: Sized + Send + Sync {
    type Config: Clone + Sized + Send + Sync;

    fn new(config: Self::Config, consensus: Arc<Consensus<P>>) -> Result<Self, ClientError>;
}


pub struct DummyBlockProducer {}
impl<P: ConsensusProtocol + 'static> BlockProducer<P> for DummyBlockProducer {
    type Config = ();

    fn new(_config: (), _consensus: Arc<Consensus<P>>) -> Result<Self, ClientError> {
        Ok(DummyBlockProducer{})
    }
}



#[cfg(feature = "validator")]
pub mod albatross {
    use std::sync::Arc;

    use consensus::{AlbatrossConsensusProtocol, Consensus};
    use validator::validator::Validator;
    use validator::error::Error as ValidatorError;
    use bls::bls12_381::KeyPair;

    use super::BlockProducer;
    use crate::error::ClientError;

    #[derive(Clone)]
    pub struct ValidatorConfig {
        pub validator_key: KeyPair,
    }

    pub struct AlbatrossBlockProducer {
        pub validator: Arc<Validator>,
    }

    impl BlockProducer<AlbatrossConsensusProtocol> for AlbatrossBlockProducer {
        type Config = ValidatorConfig;

        fn new(config: Self::Config, consensus: Arc<Consensus<AlbatrossConsensusProtocol>>) -> Result<Self, ClientError> {
            Ok(Self {
                validator: Validator::new(consensus, config.validator_key)?
            })
        }
    }

    impl From<ValidatorError> for ClientError {
        fn from(_e: ValidatorError) -> Self {
            ClientError::BlockProducerError
        }
    }
}
