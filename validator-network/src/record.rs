//! Verification of validator records against the active validator set.

use nimiq_keys::KeyPair;
use nimiq_network_interface::validator_record::ValidatorRecord;
use nimiq_primitives::slots_allocation::Validators;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::TaggedSigned;

/// A record with the validator's signature.
pub type SignedValidatorRecord<TPeerId> = TaggedSigned<ValidatorRecord<TPeerId>, KeyPair>;

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
    use nimiq_network_interface::validator_record::ValidatorRecord;
    use nimiq_primitives::slots_allocation::{Validator, Validators};
    use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSignature, TaggedSigned};

    use super::{verify_record, RecordError, SignedValidatorRecord, MAX_TIMESTAMP_DRIFT_MS};

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
}
