use std::vec;

use curve25519_dalek::{
    constants::ED25519_BASEPOINT_TABLE,
    edwards::{CompressedEdwardsY, EdwardsPoint},
    scalar::Scalar,
};
use nimiq_hash::{sha512::Sha512Hasher, Hasher};
use nimiq_utils::key_rng::SecureGenerate;
use rand::Rng;
use rand_core::{CryptoRng, RngCore};
use sha2::{self, Digest, Sha512};

use self::{
    commitment::{Commitment, CommitmentPair, Nonce},
    error::PartialSignatureError,
    partial_signature::PartialSignature,
    public_key::DelinearizedPublicKey,
};
use crate::{Ed25519PublicKey, KeyPair};

pub mod address;
pub mod commitment;
pub mod error;
pub mod partial_signature;
pub mod public_key;

/// Parameter used in Musig2
/// This describes the number of commitments required by each co-signer.
/// It must be greater than 1 for security reasons.
pub const MUSIG2_PARAMETER_V: usize = 2;

#[cfg(feature = "serde-derive")]
fn deserialize_scalar<'de, D>(deserializer: D) -> Result<Scalar, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let buf: [u8; commitment::Nonce::SIZE] =
        nimiq_serde::FixedSizeByteArray::deserialize(deserializer)?.into_inner();
    Ok(Scalar::from_bytes_mod_order(buf))
}

#[cfg(feature = "serde-derive")]
fn serialize_scalar<S>(value: &Scalar, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize as _;
    nimiq_serde::FixedSizeByteArray::from(value.to_bytes()).serialize(serializer)
}

/// This struct contains the data related to the commitments step of MuSig2.
/// It can be passed into the signing/verification process.
///
/// The `CommitmentsBuilder` can be used as a tool to generate the necessary data.
#[derive(Clone)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct CommitmentsData {
    /// Our own nonces.
    /// This is only needed for signing, not for calculating aggregate keys/commitments.
    pub nonces: Option<[Nonce; MUSIG2_PARAMETER_V]>,
    /// Our own commitments.
    /// This is only needed for verification.
    pub commitments: [Commitment; MUSIG2_PARAMETER_V],
    /// The aggregate public key corresponding to the list of public keys.
    pub aggregate_public_key: Ed25519PublicKey,
    /// The aggregate commitment over all signers.
    pub aggregate_commitment: Commitment,
    /// All public keys (including our own, need to be sorted).
    pub all_public_keys: Vec<Ed25519PublicKey>,
    /// b = H(aggregate_public_key || (R_1, ..., R_v) || m)
    #[cfg_attr(feature = "serde-derive", serde(serialize_with = "serialize_scalar"))]
    #[cfg_attr(
        feature = "serde-derive",
        serde(deserialize_with = "deserialize_scalar")
    )]
    pub b: Scalar,
}

pub struct CommitmentsBuilder {
    /// The secret part of our commitments.
    /// This is only needed for signing, not for calculating aggregate keys/commitments.
    nonces: Option<[Nonce; MUSIG2_PARAMETER_V]>,
    /// All public keys (including our own, not sorted yet).
    all_public_keys: Vec<Ed25519PublicKey>,
    /// Commitments from all signers (including ours).
    all_commitments: Vec<[Commitment; MUSIG2_PARAMETER_V]>,
}

impl CommitmentsBuilder {
    /// Creates new commitment pairs from an Rng.
    pub fn new<R: Rng + RngCore + CryptoRng>(
        own_public_key: Ed25519PublicKey,
        rng: &mut R,
    ) -> CommitmentsBuilder {
        let mut own_commitments = Vec::with_capacity(MUSIG2_PARAMETER_V);
        for _ in 0..MUSIG2_PARAMETER_V {
            own_commitments.push(CommitmentPair::generate(rng));
        }
        Self::with_private_commitments(own_public_key, own_commitments.try_into().unwrap())
    }

    /// Sets own commitments.
    pub fn with_private_commitments(
        own_public_key: Ed25519PublicKey,
        own_commitments: [CommitmentPair; MUSIG2_PARAMETER_V],
    ) -> CommitmentsBuilder {
        let mut nonces = Vec::with_capacity(MUSIG2_PARAMETER_V);
        let mut commitments = Vec::with_capacity(MUSIG2_PARAMETER_V);
        for pair in own_commitments.iter() {
            nonces.push(pair.nonce());
            commitments.push(pair.commitment());
        }
        CommitmentsBuilder {
            nonces: Some(nonces.try_into().unwrap()),
            all_public_keys: vec![own_public_key],
            all_commitments: vec![commitments.try_into().unwrap()],
        }
    }

    /// Sets own commitments.
    pub fn with_public_commitments(
        own_public_key: Ed25519PublicKey,
        own_commitments: [Commitment; MUSIG2_PARAMETER_V],
    ) -> CommitmentsBuilder {
        CommitmentsBuilder {
            nonces: None,
            all_public_keys: vec![own_public_key],
            all_commitments: vec![own_commitments],
        }
    }

    /// Returns own commitments.
    pub fn own_commitments(&self) -> [Commitment; MUSIG2_PARAMETER_V] {
        self.all_commitments[0]
    }

    /// Adds another co-signer to the signing process.
    pub fn with_signer(
        mut self,
        signer_public_key: Ed25519PublicKey,
        signer_commitments: [Commitment; MUSIG2_PARAMETER_V],
    ) -> Self {
        self.push_signer(signer_public_key, signer_commitments);
        self
    }

    /// Adds another co-signer to the signing process.
    pub fn push_signer(
        &mut self,
        signer_public_key: Ed25519PublicKey,
        signer_commitments: [Commitment; MUSIG2_PARAMETER_V],
    ) {
        self.all_public_keys.push(signer_public_key);
        self.all_commitments.push(signer_commitments);
    }

    /// Creates the aggregate commitment and additional data for the content to be signed.
    pub fn build(mut self, content: &[u8]) -> CommitmentsData {
        self.all_public_keys.sort();
        let aggregate_public_key = DelinearizedPublicKey::sum_delinearized(&self.all_public_keys);

        let mut partial_agg_commitments = Vec::with_capacity(MUSIG2_PARAMETER_V);

        // Sum up commitments for each i.
        for i in 0..MUSIG2_PARAMETER_V {
            partial_agg_commitments.push(Commitment(
                self.all_commitments
                    .iter()
                    .map(|commitments| commitments[i].0)
                    .sum(),
            ));
        }

        // Compute hash value b = H(aggregated_public_key || (R_1, ..., R_v) || m)
        let mut hasher = Sha512Hasher::new();
        hasher.hash(aggregate_public_key.as_bytes());
        for partial_agg_commitment in partial_agg_commitments.iter() {
            hasher.hash(&partial_agg_commitment.to_bytes());
        }

        hasher.hash(&content);

        let hash = hasher.finish();
        let b = Scalar::from_bytes_mod_order_wide(&hash.into());

        let mut agg_commitment_edwards = partial_agg_commitments[0].0;
        for (i, partial_agg_commitment) in partial_agg_commitments.iter().enumerate().skip(1) {
            let mut scale = b;
            for _j in 1..i {
                scale *= b;
            }
            agg_commitment_edwards += partial_agg_commitment.0 * scale;
        }

        CommitmentsData {
            nonces: self.nonces,
            commitments: self.all_commitments[0],
            aggregate_public_key,
            aggregate_commitment: Commitment(agg_commitment_edwards),
            all_public_keys: self.all_public_keys,
            b,
        }
    }
}

impl KeyPair {
    /// This function creates a partial signature that can be aggregated to a full signature.
    /// It receives the following arguments:
    /// - `commitments_data`: A struct containing all relevant information (can be constructed from the `CommitmentsBuilder`)
    /// - `data`: The data to be signed
    pub fn partial_sign(
        &self,
        commitments_data: &CommitmentsData,
        data: &[u8],
    ) -> Result<PartialSignature, PartialSignatureError> {
        // Check nonces are present.
        let nonces = commitments_data
            .nonces
            .ok_or(PartialSignatureError::MissingNonces)?;

        // Hash public keys.
        let public_keys_hash = hash_public_keys(&commitments_data.all_public_keys);
        // And delinearize them.
        // Note that here we delinearize as p^{H(H(pks), p)}, e.g., with an additional hash due to the function delinearize_private_key
        let delinearized_private_key = self.delinearize_private_key(&public_keys_hash);

        // Compute c = H(R, apk, m)
        let mut hasher = Sha512Hasher::new();
        hasher.hash(&commitments_data.aggregate_commitment.to_bytes());
        hasher.hash(commitments_data.aggregate_public_key.as_bytes());
        hasher.hash(&data);

        let hash = hasher.finish();
        let c = Scalar::from_bytes_mod_order_wide(&hash.into());

        // Compute partial signatures
        // s_j = \sk_j \cdot c \cdot a_j + \sum_{k=1}^{MUSIG2_PARAMETER_V} r_{j,k}\cdot b^{k-1}
        let mut secret = nonces[0].0;
        for (i, nonce) in nonces.iter().enumerate().skip(1) {
            let mut scale = commitments_data.b;
            for _j in 1..i {
                scale *= commitments_data.b;
            }
            secret += nonce.0 * scale;
        }

        let partial_signature_scalar: Scalar = c * delinearized_private_key + secret;
        let partial_signature = PartialSignature::from(partial_signature_scalar.as_bytes());
        Ok(partial_signature)
    }

    /// Delinearized the private key in the same way as the public key delinearization.
    /// It multiplies it with a scalar derived from the hash and the public key itself.
    pub(crate) fn delinearize_private_key(&self, public_keys_hash: &[u8; 64]) -> Scalar {
        // Compute H(C||P).
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(&public_keys_hash[..]);
        h.update(self.public.as_bytes());
        let s = Scalar::from_hash::<sha2::Sha512>(h);

        // Get a scalar representation of the private key
        let sk = self.private.0.to_scalar();

        // Compute H(C||P)*sk
        s * sk
    }
}

impl Ed25519PublicKey {
    fn to_edwards_point(self) -> Option<EdwardsPoint> {
        let mut bits: [u8; Ed25519PublicKey::SIZE] = [0u8; Ed25519PublicKey::SIZE];
        bits.copy_from_slice(&self.as_bytes()[..Ed25519PublicKey::SIZE]);

        let compressed = CompressedEdwardsY(bits);
        compressed.decompress()
    }

    /// Delinearizes a public key by multiplying it with a scalar derived from the hash and the public key itself.
    /// Effective delinearization for multisigs should use the hash over all public keys as an input.
    pub(crate) fn delinearize(&self, public_keys_hash: &[u8; 64]) -> EdwardsPoint {
        // Compute H(C||P).
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.update(&public_keys_hash[..]);
        h.update(self.as_bytes());
        let s = Scalar::from_hash::<sha2::Sha512>(h);

        // Should always work, since we come from a valid public key.
        let p = self.to_edwards_point().unwrap();
        // Compute H(C||P)*P.
        s * p
    }

    pub fn verify_partial(
        &self,
        commitments_data: &CommitmentsData,
        partial_signature: &PartialSignature,
        data: &[u8],
    ) -> bool {
        // Hash public keys.
        let public_keys_hash = hash_public_keys(&commitments_data.all_public_keys);
        // And delinearize them.
        // Note that here we delinearize as p^{H(H(pks), p)}, e.g., with an additional hash due to the function delinearize_private_key
        let delinearized_public_key = self.delinearize(&public_keys_hash);

        // Compute c = H(R, apk, m)
        let mut hasher = Sha512Hasher::new();
        hasher.hash(&commitments_data.aggregate_commitment.to_bytes());
        hasher.hash(commitments_data.aggregate_public_key.as_bytes());
        hasher.hash(&data);
        let hash = hasher.finish();

        let c = Scalar::from_bytes_mod_order_wide(&hash.into());
        let p2a = c * delinearized_public_key; // pk_i^(a_i*c)

        // product over k=1..v R_{i,k}^(b^(k-1))
        let mut p2b = commitments_data.commitments[0].0;
        for i in 1..MUSIG2_PARAMETER_V {
            let mut scale = commitments_data.b;
            for _j in 1..i {
                scale *= commitments_data.b;
            }
            p2b += commitments_data.commitments[i].0 * scale;
        }

        let p1 = &partial_signature.0 * ED25519_BASEPOINT_TABLE;
        let p2 = p2a + p2b;

        p1 == p2
    }
}

/// Compute hash over public keys public_keys_hash = C = H(P_1 || ... || P_n).
pub fn hash_public_keys(public_keys: &[Ed25519PublicKey]) -> [u8; 64] {
    let mut h: sha2::Sha512 = sha2::Sha512::default();
    let mut public_keys_hash: [u8; 64] = [0u8; 64];
    for public_key in public_keys {
        h.update(public_key.as_bytes());
    }
    public_keys_hash.copy_from_slice(h.finalize().as_slice());
    public_keys_hash
}

trait ToScalar {
    fn to_scalar(&self) -> Scalar;
}

impl ToScalar for ::ed25519_zebra::SigningKey {
    fn to_scalar(&self) -> Scalar {
        // Expand the seed to a 64-byte array with SHA512.
        let h = Sha512::digest(self.as_ref());

        // Convert the low half to a scalar with Ed25519 "clamping"
        let mut scalar_bytes = [0u8; 32];
        scalar_bytes[..].copy_from_slice(&h.as_slice()[0..32]);
        scalar_bytes[0] &= 248;
        scalar_bytes[31] &= 127;
        scalar_bytes[31] |= 64;
        // The above bit operations ensure that the integer represented by
        // `scalar_bytes` is less than 2^255-19 as required by this function.
        #[allow(deprecated)]
        Scalar::from_bits(scalar_bytes)
    }
}
