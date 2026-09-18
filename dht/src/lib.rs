use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_keys::{Address, Ed25519PublicKey, KeyPair};
use nimiq_network_interface::validator_record::ValidatorRecord;
use nimiq_network_libp2p::{
    dht::{DhtRecord, DhtVerifierError, Verifier as DhtVerifier},
    discovery::{
        InvalidReason, SignedValidatorRecord, UnverifiableReason, ValidatorRecordVerifier,
        ValidatorVerification,
    },
    libp2p::kad::Record,
    PeerId,
};
use nimiq_serde::Deserialize;
use nimiq_utils::tagged_signing::{TaggedSignable, TaggedSigned};
use time::OffsetDateTime;

/// Why the signing key of a validator could not be looked up.
enum SigningKeyError {
    /// This node runs a light blockchain and has no staking contract.
    LightClient,
    /// The staking contract is not complete on this node (yet).
    StateIncomplete,
    /// The staking contract knows no validator with this address.
    UnknownValidator,
}

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

    /// Looks up a validator's current signing key in the staking contract.
    ///
    /// This takes the blockchain read lock and a database transaction, so callers must not hold
    /// any other contended lock while calling it.
    fn lookup_signing_key(
        &self,
        validator_address: &Address,
    ) -> Result<Ed25519PublicKey, SigningKeyError> {
        // Acquire blockchain read access. For now exclude Light clients.
        let BlockchainProxy::Full(ref blockchain) = self.blockchain else {
            return Err(SigningKeyError::LightClient);
        };
        let blockchain_read = blockchain.read();

        // Get the staking contract to retrieve the public key for verification.
        let staking_contract = blockchain_read
            .get_staking_contract_if_complete(None)
            .ok_or(SigningKeyError::StateIncomplete)?;

        let data_store = blockchain_read.get_staking_contract_store();
        let txn = blockchain_read.read_transaction();
        Ok(staking_contract
            .get_validator(&data_store.read(&txn), validator_address)
            .ok_or(SigningKeyError::UnknownValidator)?
            .signing_key)
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

        // Get the public key needed for verification.
        let public_key =
            self.lookup_signing_key(&validator_address)
                .map_err(|error| match error {
                    // Keep returning `UnknownTag` for light clients, as this path did before.
                    SigningKeyError::LightClient => DhtVerifierError::UnknownTag,
                    SigningKeyError::StateIncomplete => DhtVerifierError::StateIncomplete,
                    SigningKeyError::UnknownValidator => {
                        DhtVerifierError::UnknownValidator(validator_address.clone())
                    }
                })?;

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

impl ValidatorRecordVerifier for Verifier {
    /// Checks a validator claim taken from a peer contact.
    ///
    /// This only binds the validator address to its current signing key. The checks that are
    /// specific to DHT records (publisher, record key, timestamp drift) do not apply here: a peer
    /// contact has no publisher, and its timestamp is in seconds rather than milliseconds and is
    /// already bounded by the peer contact book.
    fn verify_validator_record(
        &self,
        signed_record: &SignedValidatorRecord,
    ) -> ValidatorVerification {
        let public_key = match self.lookup_signing_key(&signed_record.record.validator_address) {
            Ok(public_key) => public_key,
            // We cannot tell yet whether this claim is good, so it has to be re-checked later.
            Err(SigningKeyError::LightClient) => {
                return ValidatorVerification::Unverifiable(UnverifiableReason::LightClient)
            }
            Err(SigningKeyError::StateIncomplete) => {
                return ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete)
            }
            // The validator may still be registered later, but as things stand the claim is bogus.
            Err(SigningKeyError::UnknownValidator) => {
                return ValidatorVerification::Invalid(InvalidReason::UnknownValidator)
            }
        };

        if signed_record.verify(&public_key) {
            ValidatorVerification::Verified
        } else {
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature)
        }
    }
}
