use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_libp2p::{
    dht::{DhtRecord, DhtVerifierError, Verifier as DhtVerifier},
    libp2p::kad::Record,
    PeerId,
};
use nimiq_serde::Deserialize;
use nimiq_utils::tagged_signing::{TaggedSignable, TaggedSigned};
use nimiq_validator_network::validator_record::ValidatorRecord;
use time::OffsetDateTime;

/// Maximum allowed future drift for `ValidatorRecord::timestamp` (milliseconds).
/// Records timestamped beyond `now + MAX_TIMESTAMP_DRIFT_MS` are rejected to
/// prevent an attacker with signing-key access from publishing a record with
/// an absurdly high timestamp that permanently wins `DhtRecord` ordering.
const MAX_TIMESTAMP_DRIFT_MS: u64 = 5 * 60 * 1000;

pub struct Verifier {
    blockchain: BlockchainProxy,
}

impl Verifier {
    pub fn new(blockchain: BlockchainProxy) -> Self {
        Self { blockchain }
    }

    fn verify_validator_record(&self, record: &Record) -> Result<DhtRecord, DhtVerifierError> {
        // Deserialize the value of the record, which is a ValidatorRecord. If it fails return an error.
        let validator_record =
            TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(&record.value)
                .map_err(DhtVerifierError::MalformedValue)?;

        // Make sure the peer who signed the record is also the one presented in the record.
        if let Some(publisher) = record.publisher {
            if validator_record.record.peer_id != publisher {
                return Err(DhtVerifierError::PublisherMismatch(
                    publisher,
                    validator_record.record.peer_id,
                ));
            }
        } else {
            log::warn!("Validating a dht record without a publisher");
            return Err(DhtVerifierError::PublisherMissing);
        }

        // Deserialize the key of the record which is an Address. If it fails return an error.
        let validator_address = Address::deserialize_from_vec(record.key.as_ref())
            .map_err(DhtVerifierError::MalformedKey)?;

        // Make sure the validator address used as key is identical to the one in the record.
        if validator_record.record.validator_address != validator_address {
            return Err(DhtVerifierError::AddressMismatch(
                validator_address,
                validator_record.record.validator_address,
            ));
        }

        // Reject records whose timestamp is too far in the future. Without this
        // bound, a signer could publish `timestamp = u64::MAX` and pin their
        // record against any future update, since `DhtRecord` ordering is by
        // timestamp alone.
        let now_ms = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64;
        if validator_record.record.timestamp > now_ms.saturating_add(MAX_TIMESTAMP_DRIFT_MS) {
            return Err(DhtVerifierError::InvalidTimestamp);
        }

        // Acquire blockchain read access. For now exclude Light clients.
        let blockchain = match self.blockchain {
            BlockchainProxy::Light(ref _light_blockchain) => {
                return Err(DhtVerifierError::UnknownTag)
            }
            BlockchainProxy::Full(ref full_blockchain) => full_blockchain,
        };
        let blockchain_read = blockchain.read();

        // Get the staking contract to retrieve the public key for verification.
        let staking_contract = blockchain_read
            .get_staking_contract_if_complete(None)
            .ok_or(DhtVerifierError::StateIncomplete)?;

        // Get the public key needed for verification.
        let data_store = blockchain_read.get_staking_contract_store();
        let txn = blockchain_read.read_transaction();
        let public_key = staking_contract
            .get_validator(&data_store.read(&txn), &validator_address)
            .ok_or(DhtVerifierError::UnknownValidator(validator_address))?
            .signing_key;

        // Verify the record.
        validator_record
            .verify(&public_key)
            .then(|| {
                DhtRecord::Validator(
                    record.publisher.unwrap(),
                    validator_record.record,
                    record.clone(),
                )
            })
            .ok_or(DhtVerifierError::InvalidSignature)
    }
}

impl DhtVerifier for Verifier {
    fn verify(&self, record: &Record) -> Result<DhtRecord, DhtVerifierError> {
        // Peek the tag to know what kind of record this is.
        let Some(tag) = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::peek_tag(&record.value)
        else {
            log::warn!(?record, "DHT Tag not peekable.");
            return Err(DhtVerifierError::MalformedTag);
        };

        // Depending on tag perform the verification.
        match tag {
            ValidatorRecord::<PeerId>::TAG => self.verify_validator_record(record),
            _ => {
                log::error!(tag, "DHT invalid record tag received");
                Err(DhtVerifierError::UnknownTag)
            }
        }
    }
}
