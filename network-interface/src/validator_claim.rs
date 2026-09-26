use nimiq_keys::{Address, KeyPair};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSignable, TaggedSigned};

impl<TPeerId> TaggedSignable for ValidatorClaim<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    const TAG: u8 = 0x04;
}

/// A validator's claim to a peer ID, attached to that peer's own contact.
///
/// It carries the same information as a
/// [`ValidatorRecord`](crate::validator_record::ValidatorRecord), but is signed under its own tag,
/// so that a signature taken from a gossiped peer contact can never pass as a DHT record, or the
/// other way around.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(bound = "TPeerId: Serialize + Deserialize")]
pub struct ValidatorClaim<TPeerId>
where
    TPeerId: Serialize + Deserialize,
{
    /// Validator Peer ID
    pub peer_id: TPeerId,
    /// The Address of the validator. This is the unique identifier of a validator.
    pub validator_address: Address,
    /// Timestamp of the peer contact carrying the claim, in seconds since 1970-01-01 00:00:00 UTC,
    /// excluding leap seconds (Unix time)
    pub timestamp: u64,
}

impl<TPeerId> ValidatorClaim<TPeerId>
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

/// Produces signed [`ValidatorClaim`]s for our own validator.
///
/// This is handed to the network layer so that it can attach a freshly signed claim to our own
/// peer contact whenever that contact is re-signed. A signer (rather than a pre-computed claim)
/// is required because the claim covers the contact's timestamp, which changes on every refresh.
#[derive(Clone)]
pub struct ValidatorClaimSigner {
    validator_address: Address,
    signing_key: KeyPair,
}

impl ValidatorClaimSigner {
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

    /// Signs a [`ValidatorClaim`] binding `peer_id` to our validator address, for the peer contact
    /// with the given `timestamp` (in seconds). The verifier reconstructs the claim from that
    /// contact.
    pub fn sign<TPeerId>(
        &self,
        peer_id: TPeerId,
        timestamp: u64,
    ) -> TaggedSigned<ValidatorClaim<TPeerId>, KeyPair>
    where
        TPeerId: Serialize + Deserialize,
    {
        let claim = ValidatorClaim::new(peer_id, self.validator_address.clone(), timestamp);
        let signature = self.signing_key.tagged_sign(&claim);
        TaggedSigned::new(claim, signature)
    }
}

impl std::fmt::Debug for ValidatorClaimSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatorClaimSigner")
            .field("validator_address", &self.validator_address)
            .finish_non_exhaustive()
    }
}
