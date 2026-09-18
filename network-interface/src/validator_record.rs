use nimiq_keys::{Address, KeyPair};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSignable, TaggedSigned};

impl<TPeerId> TaggedSignable for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    const TAG: u8 = 0x03;
}

/// Validator record that is going to be stored into the DHT
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(bound = "TPeerId: Serialize + Deserialize")]
pub struct ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    /// Validator Peer ID
    pub peer_id: TPeerId,
    /// The Address of the validator. This is the unique identifier of a validator.
    pub validator_address: Address,
    /// Record timestamp in milliseconds since 1970-01-01 00:00:00 UTC, excluding leap seconds (Unix time)
    pub timestamp: u64,
}

impl<TPeerId> ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    pub fn new(peer_id: TPeerId, validator_address: Address, timestamp: u64) -> Self {
        Self {
            peer_id,
            validator_address,
            timestamp,
        }
    }
}

impl<TPeerId> PartialOrd for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize + PartialEq,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.timestamp.partial_cmp(&other.timestamp)
    }
}

impl<TPeerId> Ord for ValidatorRecord<TPeerId>
where
    TPeerId: Serialize + Deserialize + PartialEq + Eq,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}

/// Produces signed [`ValidatorRecord`]s for our own validator.
///
/// This is handed to the network layer so that it can attach a freshly signed record to our own
/// peer contact whenever that contact is re-signed. A signer (rather than a pre-computed record)
/// is required because the record covers the contact's timestamp, which changes on every refresh.
#[derive(Clone)]
pub struct ValidatorRecordSigner {
    validator_address: Address,
    signing_key: KeyPair,
}

impl ValidatorRecordSigner {
    pub fn new(validator_address: Address, signing_key: KeyPair) -> Self {
        Self {
            validator_address,
            signing_key,
        }
    }

    /// The address of the validator this signer signs for.
    pub fn validator_address(&self) -> &Address {
        &self.validator_address
    }

    /// Signs a [`ValidatorRecord`] binding `peer_id` and `timestamp` to our validator address.
    ///
    /// The unit of `timestamp` is defined by the caller: DHT records use milliseconds, while peer
    /// contacts use seconds. The signature covers whatever is passed in, and the verifier
    /// reconstructs the record from the same source.
    pub fn sign<TPeerId>(
        &self,
        peer_id: TPeerId,
        timestamp: u64,
    ) -> TaggedSigned<ValidatorRecord<TPeerId>, KeyPair>
    where
        TPeerId: Serialize + Deserialize,
    {
        let record = ValidatorRecord::new(peer_id, self.validator_address.clone(), timestamp);
        let signature = self.signing_key.tagged_sign(&record);
        TaggedSigned::new(record, signature)
    }
}

impl std::fmt::Debug for ValidatorRecordSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatorRecordSigner")
            .field("validator_address", &self.validator_address)
            .finish_non_exhaustive()
    }
}
