#[cfg(feature="validator")]
use std::sync::Arc;

#[cfg(feature="validator")]
use bls::bls12_381::KeyPair;
#[cfg(feature="validator")]
use consensus::{AlbatrossConsensusProtocol, Consensus as GenericConsensus, ConsensusProtocol, NimiqConsensusProtocol};
#[cfg(feature="validator")]
use validator::error::Error as ValidatorError;
#[cfg(feature="validator")]
use validator::validator::Validator;

#[cfg(feature="validator")]
use crate::config::config::ValidatorConfig;


#[cfg(feature="validator")]
pub trait BlockProducerFactory where Self: ConsensusProtocol + Sized {
    fn validator_constructor() -> fn(Option<ValidatorConfig>, &Arc<GenericConsensus<Self>>, KeyPair)
        -> Option<Result<Arc<Validator>, ValidatorError>>;
}

#[cfg(feature="validator")]
impl BlockProducerFactory for AlbatrossConsensusProtocol {
    fn validator_constructor() -> fn(validator_config: Option<ValidatorConfig>, consensus: &Arc<GenericConsensus<Self>>, validator_key: KeyPair)
        -> Option<Result<Arc<Validator>, ValidatorError>> {
        |validator_config: Option<ValidatorConfig>, consensus, validator_key: KeyPair| {
            validator_config.map(|_config| {
                Validator::new(Arc::clone(&consensus), validator_key)
            })
        }
    }
}

#[cfg(feature="validator")]
impl BlockProducerFactory for NimiqConsensusProtocol {
    fn validator_constructor() -> fn(validator_config: Option<ValidatorConfig>, consensus: &Arc<GenericConsensus<Self>>, validator_key: KeyPair)
        -> Option<Result<Arc<Validator>, ValidatorError>> {
        |_validator_config: Option<ValidatorConfig>, _consensus, _validator_key: KeyPair| {
            None
        }
    }
}

// Without the validator feature enabled, make sure the function signatures are not be broken.
#[cfg(not(feature="validator"))]
use consensus::{AlbatrossConsensusProtocol, ConsensusProtocol, NimiqConsensusProtocol};

#[cfg(not(feature="validator"))]
pub trait BlockProducerFactory where Self: ConsensusProtocol + Sized {}
#[cfg(not(feature="validator"))]
impl BlockProducerFactory for AlbatrossConsensusProtocol {}
#[cfg(not(feature="validator"))]
impl BlockProducerFactory for NimiqConsensusProtocol {}
