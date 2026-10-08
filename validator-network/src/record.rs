//! Validator records: the messages that carry them and their verification against the active
//! validator set.

use std::{fmt, marker::PhantomData};

use nimiq_keys::KeyPair;
use nimiq_network_interface::{
    request::{MessageMarker, RequestCommon, RequestMarker},
    validator_record::ValidatorRecord,
};
use nimiq_primitives::slots_allocation::Validators;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::TaggedSigned;

/// A record with the validator's signature.
pub type SignedValidatorRecord<TPeerId> = TaggedSigned<ValidatorRecord<TPeerId>, KeyPair>;

/// Type IDs of the messages below. They are sent raw, outside the `ValidatorMessage` wrapper,
/// whose IDs start at 10,000.
pub const ANNOUNCEMENT_TYPE_ID: u16 = 130;
pub const RECORDS_REQUEST_TYPE_ID: u16 = 131;

/// Our own signed record, sent to a validator peer right after connecting and whenever it
/// changes. On the wire it is exactly the value the DHT stores.
#[derive(Clone, Serialize, Deserialize)]
#[serde(bound = "TPeerId: Serialize + Deserialize")]
pub struct RecordAnnouncement<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub record: SignedValidatorRecord<TPeerId>,
}

impl<TPeerId> fmt::Debug for RecordAnnouncement<TPeerId>
where
    TPeerId: Serialize + Deserialize + fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordAnnouncement")
            .field("record", &self.record.record)
            .field("signature", &self.record.signature)
            .finish()
    }
}

impl<TPeerId> RequestCommon for RecordAnnouncement<TPeerId>
where
    TPeerId: Serialize + Deserialize + fmt::Debug + Send + Sync + Unpin + 'static,
{
    type Kind = MessageMarker;
    type Response = ();
    const TYPE_ID: u16 = ANNOUNCEMENT_TYPE_ID;
    /// Honest peers send one per connection and one per change; the network drops the rest.
    const MAX_REQUESTS: u32 = 8;
}

/// Asks a validator peer for the newest records it holds for the active validator set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordsRequest<TPeerId> {
    #[serde(skip)]
    peer_id: PhantomData<TPeerId>,
}

impl<TPeerId> Default for RecordsRequest<TPeerId> {
    fn default() -> Self {
        Self {
            peer_id: PhantomData,
        }
    }
}

impl<TPeerId> RequestCommon for RecordsRequest<TPeerId>
where
    TPeerId: Serialize + Deserialize + fmt::Debug + Send + Sync + Unpin + 'static,
{
    type Kind = RequestMarker;
    type Response = Vec<SignedValidatorRecord<TPeerId>>;
    const TYPE_ID: u16 = RECORDS_REQUEST_TYPE_ID;
    const MAX_REQUESTS: u32 = 4;
}

/// Maximum drift of a record's timestamp into the future, in ms. Matches the DHT verifier.
pub const MAX_TIMESTAMP_DRIFT_MS: u64 = 5 * 60 * 1000;

/// Why a record was rejected, in the order the checks run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// The address is not in the active validator set.
    UnknownValidator,
    /// An announced record names a peer other than its sender.
    PeerIdMismatch,
    /// The timestamp lies more than `MAX_TIMESTAMP_DRIFT_MS` in the future.
    TimestampInFuture,
    /// The signature does not verify with the validator's signing key.
    InvalidSignature,
}

/// Checks `record` against the active `validators` and returns the validator's slot band.
/// `sender` is the peer that announced the record as its own, `None` if it was relayed.
pub fn verify_record<TPeerId>(
    _record: &SignedValidatorRecord<TPeerId>,
    _validators: &Validators,
    _sender: Option<TPeerId>,
    _now_ms: u64,
) -> Result<u16, RecordError>
where
    TPeerId: PartialEq + Serialize + Deserialize,
{
    todo!()
}

#[cfg(test)]
mod tests {
    use nimiq_bls::CompressedPublicKey;
    use nimiq_keys::{Address, KeyPair, SecureGenerate};
    use std::time::Duration;

    use nimiq_network_interface::{
        request::{RequestCommon, RequestSerialize, RequestType},
        validator_record::ValidatorRecord,
    };
    use nimiq_primitives::slots_allocation::{Validator, Validators};
    use nimiq_serde::{Deserialize, Serialize};
    use nimiq_utils::tagged_signing::{
        TaggedKeyPair, TaggedSignable, TaggedSignature, TaggedSigned,
    };

    use super::{
        verify_record, RecordAnnouncement, RecordError, RecordsRequest, SignedValidatorRecord,
        ANNOUNCEMENT_TYPE_ID, MAX_TIMESTAMP_DRIFT_MS, RECORDS_REQUEST_TYPE_ID,
    };

    const NOW: u64 = 1_700_000_000_000;
    const PEER: u32 = 7;
    const OTHER_PEER: u32 = 8;

    fn address(seed: u8) -> Address {
        Address::from([seed; 20])
    }

    /// An active set of `n` validators with fresh signing keys; validator `i` has address
    /// `address(i)` and slot band `i`.
    fn active_set(n: u8) -> (Validators, Vec<KeyPair>) {
        let keys: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate_default_csprng()).collect();
        let validators = keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                Validator::new(
                    address(i as u8),
                    CompressedPublicKey::default(),
                    key.public,
                    0..1,
                )
            })
            .collect();
        (Validators::new(validators), keys)
    }

    fn record(seed: u8, peer_id: u32, timestamp: u64, key: &KeyPair) -> SignedValidatorRecord<u32> {
        let record = ValidatorRecord::new(peer_id, address(seed), timestamp);
        let signature = key.tagged_sign(&record);
        TaggedSigned::new(record, signature)
    }

    fn with_signature(
        mut record: SignedValidatorRecord<u32>,
        bytes: Vec<u8>,
    ) -> SignedValidatorRecord<u32> {
        record.signature = TaggedSignature::from_bytes(bytes);
        record
    }

    #[test]
    fn a_record_signed_with_the_validators_signing_key_verifies() {
        let (validators, keys) = active_set(3);
        let record = record(1, PEER, NOW, &keys[1]);
        assert_eq!(verify_record(&record, &validators, Some(PEER), NOW), Ok(1));
    }

    #[test]
    fn a_record_signed_with_another_key_is_rejected() {
        let (validators, keys) = active_set(3);
        let rotated_away = KeyPair::generate_default_csprng();
        for key in [&keys[0], &rotated_away] {
            let record = record(1, PEER, NOW, key);
            assert_eq!(
                verify_record(&record, &validators, Some(PEER), NOW),
                Err(RecordError::InvalidSignature)
            );
        }
    }

    #[test]
    fn an_address_outside_the_active_set_is_rejected_before_the_signature_is_checked() {
        let (validators, keys) = active_set(3);
        let record = with_signature(record(9, PEER, NOW, &keys[0]), vec![1, 2, 3]);
        assert_eq!(
            verify_record(&record, &validators, Some(PEER), NOW),
            Err(RecordError::UnknownValidator)
        );
    }

    #[test]
    fn a_malformed_signature_is_rejected() {
        let (validators, keys) = active_set(1);
        for bytes in [vec![], vec![1, 2, 3], vec![0; 65]] {
            let record = with_signature(record(0, PEER, NOW, &keys[0]), bytes.clone());
            assert_eq!(
                verify_record(&record, &validators, Some(PEER), NOW),
                Err(RecordError::InvalidSignature),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn a_timestamp_beyond_the_drift_bound_is_rejected() {
        let (validators, keys) = active_set(1);
        let record = record(0, PEER, NOW + MAX_TIMESTAMP_DRIFT_MS + 1, &keys[0]);
        assert_eq!(
            verify_record(&record, &validators, Some(PEER), NOW),
            Err(RecordError::TimestampInFuture)
        );
    }

    #[test]
    fn a_timestamp_at_the_drift_bound_or_in_the_past_is_accepted() {
        let (validators, keys) = active_set(1);
        for timestamp in [NOW + MAX_TIMESTAMP_DRIFT_MS, NOW, 0] {
            let record = record(0, PEER, timestamp, &keys[0]);
            assert_eq!(
                verify_record(&record, &validators, Some(PEER), NOW),
                Ok(0),
                "{timestamp}"
            );
        }
    }

    #[test]
    fn an_announcement_naming_another_peer_is_rejected() {
        let (validators, keys) = active_set(1);
        let record = record(0, PEER, NOW, &keys[0]);
        assert_eq!(
            verify_record(&record, &validators, Some(OTHER_PEER), NOW),
            Err(RecordError::PeerIdMismatch)
        );
    }

    #[test]
    fn a_relayed_record_may_name_a_peer_other_than_the_sender() {
        let (validators, keys) = active_set(1);
        let record = record(0, PEER, NOW, &keys[0]);
        assert_eq!(verify_record(&record, &validators, None, NOW), Ok(0));
    }

    #[test]
    fn a_verified_record_maps_to_the_slot_band_of_its_address() {
        let (validators, keys) = active_set(4);
        for (band, key) in keys.iter().enumerate() {
            let record = record(band as u8, PEER, NOW, key);
            assert_eq!(
                verify_record(&record, &validators, None, NOW),
                Ok(band as u16)
            );
        }
    }

    #[test]
    fn cheap_checks_precede_the_signature_check() {
        let (validators, keys) = active_set(1);
        let garbage = |record| with_signature(record, vec![1, 2, 3]);

        let mismatch = garbage(record(0, PEER, NOW, &keys[0]));
        assert_eq!(
            verify_record(&mismatch, &validators, Some(OTHER_PEER), NOW),
            Err(RecordError::PeerIdMismatch)
        );

        let future = garbage(record(0, PEER, NOW + MAX_TIMESTAMP_DRIFT_MS + 1, &keys[0]));
        assert_eq!(
            verify_record(&future, &validators, Some(PEER), NOW),
            Err(RecordError::TimestampInFuture)
        );
    }

    #[test]
    fn the_new_type_ids_do_not_collide_with_wrapped_validator_messages() {
        let announcement = RequestType::from_request::<RecordAnnouncement<u32>>();
        let request = RequestType::from_request::<RecordsRequest<u32>>();

        // Both go out raw; the `ValidatorMessage` wrapper uses 10_000 and up.
        assert!(announcement.type_id() < 10_000);
        assert!(request.type_id() < 10_000);
        assert_ne!(announcement.type_id(), request.type_id());

        assert_eq!(announcement.type_id(), ANNOUNCEMENT_TYPE_ID);
        assert!(!announcement.requires_response());
        assert_eq!(request.type_id(), RECORDS_REQUEST_TYPE_ID);
        assert!(request.requires_response());
    }

    #[test]
    fn the_announced_record_is_byte_identical_to_the_dht_record() {
        let (_, keys) = active_set(1);
        let signed = record(0, PEER, NOW, &keys[0]);
        // The DHT stores the signed record as is.
        let dht_value = signed.serialize_to_vec();

        let announcement = RecordAnnouncement { record: signed };
        assert_eq!(announcement.serialize_to_vec(), dht_value);
        assert_eq!(
            SignedValidatorRecord::<u32>::peek_tag(&dht_value),
            Some(ValidatorRecord::<u32>::TAG)
        );
    }

    #[test]
    fn announcement_and_records_request_round_trip() {
        let (_, keys) = active_set(2);

        let announcement = RecordAnnouncement {
            record: record(0, PEER, NOW, &keys[0]),
        };
        let back =
            RecordAnnouncement::<u32>::deserialize_request(&announcement.serialize_request())
                .unwrap();
        assert_eq!(back.record.record, announcement.record.record);
        assert_eq!(
            back.record.signature.as_bytes(),
            announcement.record.signature.as_bytes()
        );

        let request = RecordsRequest::<u32>::default();
        RecordsRequest::<u32>::deserialize_request(&request.serialize_request()).unwrap();

        let response = vec![
            record(0, PEER, NOW, &keys[0]),
            record(1, OTHER_PEER, NOW, &keys[1]),
        ];
        let back: Vec<SignedValidatorRecord<u32>> =
            Deserialize::deserialize_from_vec(&response.serialize_to_vec()).unwrap();
        assert_eq!(back.len(), response.len());
        for (received, sent) in back.iter().zip(&response) {
            assert_eq!(received.record, sent.record);
            assert_eq!(received.signature.as_bytes(), sent.signature.as_bytes());
        }
    }

    #[test]
    fn announcements_are_limited_per_peer_by_the_network_layer() {
        // Enforced by libp2p before dispatch, per peer and message type.
        assert_eq!(RecordAnnouncement::<u32>::MAX_REQUESTS, 8);
        assert_eq!(
            RecordAnnouncement::<u32>::TIME_WINDOW,
            Duration::from_secs(10)
        );
        assert_eq!(RecordsRequest::<u32>::MAX_REQUESTS, 4);
    }
}
