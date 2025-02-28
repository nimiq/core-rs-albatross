use libp2p::{kad::Record, PeerId};
use nimiq_keys::Address;
use nimiq_network_interface::{
    network::Network as NetworkInterface, validator_record::ValidatorRecord,
};
use nimiq_serde::DeserializeError;

pub use crate::network_types::DhtRecord;
use crate::Network;

#[derive(Debug)]
pub enum DhtVerifierError {
    MalformedTag,
    UnknownTag,
    MalformedKey(DeserializeError),
    MalformedValue(DeserializeError),
    UnknownValidator(Address),
    AddressMismatch(Address, Address),
    PublisherMissing,
    PublisherMismatch(
        <Network as NetworkInterface>::PeerId,
        <Network as NetworkInterface>::PeerId,
    ),
    StateIncomplete,
    InvalidSignature,
    ValidatorInfoError,
    MissingPublisher,
}

pub trait Verifier: Send + Sync {
    fn verify(&self, record: &Record) -> Result<DhtRecord, DhtVerifierError>;
}

/// Dummy implementation for testcases
impl Verifier for () {
    fn verify(&self, record: &Record) -> Result<DhtRecord, DhtVerifierError> {
        let peer_id = PeerId::random();
        Ok(DhtRecord::Validator(
            peer_id,
            ValidatorRecord::<PeerId>::new(peer_id, Address::default(), 0u64),
            record.clone(),
        ))
    }
}
