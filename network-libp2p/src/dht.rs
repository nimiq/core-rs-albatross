use libp2p::{kad::Record, PeerId};
use nimiq_keys::Address;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_serde::DeserializeError;
use nimiq_validator_network::validator_record::ValidatorRecord;

pub use crate::network_types::DhtRecord;
use crate::{discovery::peer_contacts::ValidatorInfoError, Network};

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
