use std::{borrow::Cow, str::FromStr};

use log::error;
use nimiq_hash::{
    sha512::{Sha512Hash, Sha512Hasher},
    Blake2bHash, Blake2bHasher, Hasher, Sha256Hash, Sha256Hasher,
};
use nimiq_keys::Address;
use nimiq_macros::{
    add_hex_io_fns_typed_arr, add_serde_serialization_fns_typed_arr, create_typed_array,
};
use nimiq_primitives::account::AccountType;
use nimiq_serde::{Deserialize, Serialize};

use crate::{
    account::AccountTransactionVerification, SignatureProof, Transaction, TransactionError,
    TransactionFlags,
};

/// The verifier trait for a hash time locked contract. This only uses data available in the transaction.
pub struct HashedTimeLockedContractVerifier {}

impl AccountTransactionVerification for HashedTimeLockedContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::HTLC);

        if !transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            error!(
                "Only contract creation is allowed for the following transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.flags.contains(TransactionFlags::SIGNALING) {
            error!(
                "Signaling not allowed for the following transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Recipient address must match contract creation address for the following transaction:\n{:?}",
                transaction);
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient_data.len() != (20 * 2 + 1 + 32 + 1 + 8)
            && transaction.recipient_data.len() != (20 * 2 + 1 + 64 + 1 + 8)
        {
            warn!(
                data_len = transaction.recipient_data.len(),
                ?transaction,
                "Invalid data length. For the following transaction",
            );
            return Err(TransactionError::InvalidData);
        }

        CreationTransactionData::parse(transaction)?.verify()
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::HTLC);

        let proof = OutgoingHTLCTransactionProof::parse(transaction)?;
        proof.verify(transaction)?;

        Ok(())
    }
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum AnyHash {
    Blake2b(AnyHash32),
    Sha256(AnyHash32),
    Sha512(AnyHash64),
}

impl AnyHash {
    /// Returns the hex string representation of the hash
    pub fn to_hex(&self) -> String {
        match self {
            AnyHash::Blake2b(hash) => hash.to_hex(),
            AnyHash::Sha256(hash) => hash.to_hex(),
            AnyHash::Sha512(hash) => hash.to_hex(),
        }
    }

    /// Returns the raw bytes of the hash
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            AnyHash::Blake2b(hash) => &hash.0,
            AnyHash::Sha256(hash) => &hash.0,
            AnyHash::Sha512(hash) => &hash.0,
        }
    }
}

impl Default for AnyHash {
    fn default() -> Self {
        AnyHash::Blake2b(AnyHash32::default())
    }
}

impl From<Blake2bHash> for AnyHash {
    fn from(value: Blake2bHash) -> Self {
        AnyHash::Blake2b(AnyHash32(value.into()))
    }
}

impl From<Sha256Hash> for AnyHash {
    fn from(value: Sha256Hash) -> Self {
        AnyHash::Sha256(AnyHash32(value.into()))
    }
}

impl From<Sha512Hash> for AnyHash {
    fn from(value: Sha512Hash) -> Self {
        AnyHash::Sha512(AnyHash64(value.into()))
    }
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum PreImage {
    PreImage32(AnyHash32),
    PreImage64(AnyHash64),
}

impl PreImage {
    /// Returns the hex string representation of the pre-image
    pub fn to_hex(&self) -> String {
        match self {
            PreImage::PreImage32(hash) => hash.to_hex(),
            PreImage::PreImage64(hash) => hash.to_hex(),
        }
    }

    /// Returns the raw bytes of the pre-image
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            PreImage::PreImage32(hash) => &hash.0,
            PreImage::PreImage64(hash) => &hash.0,
        }
    }
}

impl Default for PreImage {
    fn default() -> Self {
        PreImage::PreImage32(AnyHash32::default())
    }
}

impl From<Blake2bHash> for PreImage {
    fn from(value: Blake2bHash) -> Self {
        PreImage::PreImage32(AnyHash32(value.into()))
    }
}

impl From<Sha256Hash> for PreImage {
    fn from(value: Sha256Hash) -> Self {
        PreImage::PreImage32(AnyHash32(value.into()))
    }
}

impl From<Sha512Hash> for PreImage {
    fn from(value: Sha512Hash) -> Self {
        PreImage::PreImage64(AnyHash64(value.into()))
    }
}

impl FromStr for PreImage {
    type Err = nimiq_macros::hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() == AnyHash32::SIZE * 2 {
            Ok(PreImage::PreImage32(AnyHash32::from_str(s)?))
        } else if s.len() == AnyHash64::SIZE * 2 {
            Ok(PreImage::PreImage64(AnyHash64::from_str(s)?))
        } else {
            Err(nimiq_macros::hex::FromHexError::InvalidStringLength)
        }
    }
}

create_typed_array!(AnyHash32, u8, 32);
add_hex_io_fns_typed_arr!(AnyHash32, AnyHash32::SIZE);
add_serde_serialization_fns_typed_arr!(AnyHash32, AnyHash32::SIZE);

create_typed_array!(AnyHash64, u8, 64);
add_hex_io_fns_typed_arr!(AnyHash64, AnyHash64::SIZE);
add_serde_serialization_fns_typed_arr!(AnyHash64, AnyHash64::SIZE);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreationTransactionData {
    pub sender: Address,
    pub recipient: Address,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    #[serde(with = "nimiq_serde::fixint::be")]
    pub timeout: u64,
}

impl CreationTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Deserialize::deserialize_from_vec(
            &transaction.recipient_data[..],
        )?)
    }

    pub fn verify(&self) -> Result<(), TransactionError> {
        if self.hash_count == 0 {
            warn!("Invalid creation data: hash_count may not be zero");
            return Err(TransactionError::InvalidData);
        }
        Ok(())
    }
}

/// The `OutgoingHTLCTransactionProof` represents a serializable form of all possible proof types
/// for a transaction from a HTLC contract.
///
/// The funds can be unlocked by one of three mechanisms:
/// 1. After a blockchain height called `timeout` is reached, the `sender` can withdraw the funds.
///     (called `TimeoutResolve`)
/// 2. The contract stores a `hash_root`. The `recipient` can withdraw the funds before the
///     `timeout` has been reached by presenting a hash that will yield the `hash_root`
///     when re-hashing it `hash_count` times.
///     By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
///     times, the `recipient` can retrieve 1/k of the funds.
///     (called `RegularTransfer`)
/// 3. If both `sender` and `recipient` sign the transaction, the funds can be withdrawn at any time.
///     (called `EarlyResolve`)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutgoingHTLCTransactionProof {
    RegularTransfer {
        hash_depth: u8,
        hash_root: AnyHash,
        pre_image: PreImage,
        signature_proof: SignatureProof,
    },
    EarlyResolve {
        signature_proof_recipient: SignatureProof,
        signature_proof_sender: SignatureProof,
    },
    TimeoutResolve {
        signature_proof_sender: SignatureProof,
    },
}

impl OutgoingHTLCTransactionProof {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.proof[..];
        let (data, left_over) = Self::deserialize_take(reader)?;

        // Ensure that transaction data has been fully read.
        if !left_over.is_empty() {
            warn!("Over-long proof for the transaction");
            return Err(TransactionError::InvalidProof);
        }

        Ok(data)
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        // Verify proof.
        let tx_content = transaction.serialize_content();
        let tx_buf = tx_content.as_slice();

        match self {
            OutgoingHTLCTransactionProof::RegularTransfer {
                hash_depth,
                hash_root,
                pre_image,
                signature_proof,
            } => {
                let mut tmp_hash = pre_image.clone();
                for _ in 0..*hash_depth {
                    match &hash_root {
                        AnyHash::Blake2b(_) => {
                            tmp_hash = PreImage::from(
                                Blake2bHasher::default().digest(tmp_hash.as_bytes()),
                            );
                        }
                        AnyHash::Sha256(_) => {
                            tmp_hash =
                                PreImage::from(Sha256Hasher::default().digest(tmp_hash.as_bytes()));
                        }
                        AnyHash::Sha512(_) => {
                            tmp_hash =
                                PreImage::from(Sha512Hasher::default().digest(tmp_hash.as_bytes()));
                        }
                    }
                }

                if hash_root.as_bytes() != tmp_hash.as_bytes() {
                    warn!(
                        "Hash algorithm mismatch for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }

                if !signature_proof.verify(tx_buf) {
                    warn!(
                        "Invalid signature for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }
            }
            OutgoingHTLCTransactionProof::EarlyResolve {
                signature_proof_recipient,
                signature_proof_sender,
            } => {
                if !signature_proof_recipient.verify(tx_buf)
                    || !signature_proof_sender.verify(tx_buf)
                {
                    warn!(
                        "Invalid signature for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }
            }
            OutgoingHTLCTransactionProof::TimeoutResolve {
                signature_proof_sender,
            } => {
                if !signature_proof_sender.verify(tx_buf) {
                    warn!(
                        "Invalid signature for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }
            }
        }

        Ok(())
    }
}

mod serde_derive {
    use std::{borrow::Cow, fmt, str::FromStr};

    use serde::{
        de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor},
        ser::{Serialize, SerializeStruct, Serializer},
    };

    use super::{AnyHash, AnyHash32, AnyHash64, PreImage};

    const ANYHASH_FIELDS: &[&str] = &["algorithm", "hash"];
    const PREIMAGE_FIELDS: &[&str] = &["type", "pre_image"];

    #[derive(nimiq_serde::Deserialize)]
    #[serde(field_identifier, rename_all = "lowercase")]
    enum AnyHashField {
        Algorithm,
        Hash,
    }

    struct PreImageVisitor;
    struct AnyHashVisitor;

    impl Serialize for AnyHash {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let human_readable = serializer.is_human_readable();
            let mut state = serializer.serialize_struct("AnyHash", ANYHASH_FIELDS.len())?;
            match self {
                AnyHash::Blake2b(hash) => {
                    if human_readable {
                        state.serialize_field(ANYHASH_FIELDS[0], &"blake2b")?;
                    } else {
                        state.serialize_field(ANYHASH_FIELDS[0], &1u8)?;
                    }
                    state.serialize_field(ANYHASH_FIELDS[1], hash)?;
                }
                AnyHash::Sha256(hash) => {
                    if human_readable {
                        state.serialize_field(ANYHASH_FIELDS[0], &"sha256")?;
                    } else {
                        state.serialize_field(ANYHASH_FIELDS[0], &3u8)?;
                    }
                    state.serialize_field(ANYHASH_FIELDS[1], hash)?;
                }
                AnyHash::Sha512(hash) => {
                    if human_readable {
                        state.serialize_field(ANYHASH_FIELDS[0], &"sha512")?;
                    } else {
                        state.serialize_field(ANYHASH_FIELDS[0], &4u8)?;
                    }
                    state.serialize_field(ANYHASH_FIELDS[1], hash)?;
                }
            }
            state.end()
        }
    }

    impl<'de> Visitor<'de> for AnyHashVisitor {
        type Value = AnyHash;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("enum AnyHash")
        }

        /// If the deserializer is not human friendly most likely will use `visit_seq` for deserializing
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let algorithm: u8 = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(0, &self))?;
            match algorithm {
                1u8 => {
                    let hash: AnyHash32 = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                    Ok(AnyHash::Blake2b(hash))
                }
                3u8 => {
                    let hash: AnyHash32 = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                    Ok(AnyHash::Sha256(hash))
                }
                4u8 => {
                    let hash: AnyHash64 = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                    Ok(AnyHash::Sha512(hash))
                }
                _ => Err(A::Error::invalid_value(
                    serde::de::Unexpected::Unsigned(algorithm as u64),
                    &"an AnyHash variant",
                )),
            }
        }

        /// If the deserializer is human friendly most likely will use `visit_map` for deserializing
        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut algorithm = None;
            let mut hash = None;
            while let Some(key) = map.next_key()? {
                match key {
                    AnyHashField::Algorithm => {
                        if algorithm.is_some() {
                            return Err(A::Error::duplicate_field("algorithm"));
                        }
                        algorithm = Some(map.next_value()?);
                    }
                    AnyHashField::Hash => {
                        if hash.is_some() {
                            return Err(A::Error::duplicate_field("hash"));
                        }
                        hash = Some(map.next_value()?);
                    }
                }
            }
            let hash: String = hash.ok_or_else(|| A::Error::missing_field("hash"))?;
            let algorithm: String =
                algorithm.ok_or_else(|| A::Error::missing_field("algorithm"))?;
            match algorithm.as_str() {
                "blake2b" => {
                    let hash = AnyHash32::from_str(hash.as_str()).map_err(A::Error::custom)?;
                    Ok(AnyHash::Blake2b(hash))
                }
                "sha256" => {
                    let hash = AnyHash32::from_str(hash.as_str()).map_err(A::Error::custom)?;
                    Ok(AnyHash::Sha256(hash))
                }
                "sha512" => {
                    let hash = AnyHash64::from_str(hash.as_str()).map_err(A::Error::custom)?;
                    Ok(AnyHash::Sha512(hash))
                }
                _ => Err(A::Error::invalid_value(
                    serde::de::Unexpected::Str(algorithm.as_str()),
                    &"an AnyHash variant",
                )),
            }
        }
    }

    impl<'de> Deserialize<'de> for AnyHash {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            // Here we are not making any distinction between a human readable deserializer and a non human readable ones
            // because regularly the first one uses `visit_map` while the latter uses `visit_seq` instead.
            deserializer.deserialize_struct("AnyHash", ANYHASH_FIELDS, AnyHashVisitor)
        }
    }

    impl Serialize for PreImage {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if serializer.is_human_readable() {
                match self {
                    PreImage::PreImage32(hash) => Serialize::serialize(hash, serializer),
                    PreImage::PreImage64(hash) => Serialize::serialize(hash, serializer),
                }
            } else {
                let mut state = serializer.serialize_struct("PreImage", PREIMAGE_FIELDS.len())?;
                match self {
                    PreImage::PreImage32(pre_image) => {
                        state.serialize_field(PREIMAGE_FIELDS[0], &32u8)?;
                        state.serialize_field(PREIMAGE_FIELDS[1], pre_image)?;
                    }
                    PreImage::PreImage64(pre_image) => {
                        state.serialize_field(PREIMAGE_FIELDS[0], &64u8)?;
                        state.serialize_field(PREIMAGE_FIELDS[1], pre_image)?;
                    }
                }
                state.end()
            }
        }
    }

    impl<'de> Visitor<'de> for PreImageVisitor {
        type Value = PreImage;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("enum PreImage")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let pre_image_type: u8 = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(0, &self))?;
            match pre_image_type {
                32u8 => {
                    let pre_image: AnyHash32 = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                    Ok(PreImage::PreImage32(pre_image))
                }
                64u8 => {
                    let pre_image: AnyHash64 = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                    Ok(PreImage::PreImage64(pre_image))
                }
                _ => Err(A::Error::invalid_value(
                    serde::de::Unexpected::Unsigned(pre_image_type as u64),
                    &"a PreImage variant",
                )),
            }
        }
    }

    impl<'de> Deserialize<'de> for PreImage {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                data.parse().map_err(Error::custom)
            } else {
                deserializer.deserialize_struct("PreImage", PREIMAGE_FIELDS, PreImageVisitor)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nimiq_serde::{Deserialize, Serialize};
    use nimiq_test_log::test;

    use super::{AnyHash, AnyHash32, AnyHash64, PreImage};

    fn sample_anyhashes() -> [AnyHash; 3] {
        let hash_32 = AnyHash32([0xC; AnyHash32::SIZE]);
        let hash_64 = AnyHash64([0xC; AnyHash64::SIZE]);
        [
            AnyHash::Sha256(hash_32.clone()),
            AnyHash::Blake2b(hash_32),
            AnyHash::Sha512(hash_64),
        ]
    }

    fn sample_preimages() -> [PreImage; 2] {
        let hash_32 = AnyHash32([0xC; AnyHash32::SIZE]);
        let hash_64 = AnyHash64([0xC; AnyHash64::SIZE]);
        [PreImage::PreImage32(hash_32), PreImage::PreImage64(hash_64)]
    }

    #[test]
    fn it_can_correctly_serialize_anyhash() {
        let hashes = sample_anyhashes();
        let bin = hashes[0].serialize_to_vec();
        assert_eq!(
            hex::decode("030C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C")
                .unwrap(),
            bin
        );
        let bin = hashes[1].serialize_to_vec();
        assert_eq!(
            hex::decode("010C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C")
                .unwrap(),
            bin
        );
        let bin = hashes[2].serialize_to_vec();
        assert_eq!(hex::decode("040C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C").unwrap(), bin);
    }

    #[test]
    fn it_can_correctly_deserialize_anyhash() {
        let hashes = sample_anyhashes();
        let bin = hex::decode("030C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C")
            .unwrap();
        let set = AnyHash::deserialize_from_vec(&bin).unwrap();
        assert_eq!(hashes[0], set);
        let bin = hex::decode("010C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C")
            .unwrap();
        let set = AnyHash::deserialize_from_vec(&bin).unwrap();
        assert_eq!(hashes[1], set);
        let bin = hex::decode("040C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C").unwrap();
        let set = AnyHash::deserialize_from_vec(&bin).unwrap();
        assert_eq!(hashes[2], set);
    }

    #[test]
    fn it_can_correctly_serialize_anyhash_human_readably() {
        let hashes = sample_anyhashes();
        assert_eq!(
            serde_json::to_string(&hashes[0]).unwrap(),
            r#"{"algorithm":"sha256","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#,
        );
        assert_eq!(
            serde_json::to_string(&hashes[1]).unwrap(),
            r#"{"algorithm":"blake2b","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#,
        );
        assert_eq!(
            serde_json::to_string(&hashes[2]).unwrap(),
            r#"{"algorithm":"sha512","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#,
        );
    }

    #[test]
    fn it_can_correctly_deserialize_anyhash_human_readably() {
        let hashes = sample_anyhashes();
        assert_eq!(
            serde_json::from_str::<AnyHash>(r#"{"algorithm":"sha256","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#,)
                .unwrap(),
            hashes[0],
        );
        assert_eq!(
            serde_json::from_str::<AnyHash>(r#"{"algorithm":"blake2b","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#)
                .unwrap(),
            hashes[1],
        );
        assert_eq!(
            serde_json::from_str::<AnyHash>(r#"{"algorithm":"sha512","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#)
                .unwrap(),
            hashes[2],
        );
    }

    #[test]
    fn it_can_error_on_human_readable_anyhash_deserialization() {
        assert!(serde_json::from_str::<AnyHash>(
            "0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"
        )
        .is_err()); // invalid type
        assert!(serde_json::from_str::<AnyHash>(r#"sha256"#).is_err()); // invalid type
        assert!(serde_json::from_str::<AnyHash>(r#"{"algorithm":"baKe2b","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#).is_err()); // invalid algorithm
        assert!(serde_json::from_str::<AnyHash>(r#"{"algorithm":"blake2b","algorithm":"blake2b","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#).is_err()); // duplicate algorithm
        assert!(serde_json::from_str::<AnyHash>(r#"{"algorithm":"sha256","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#).is_err());
        // too small hash for 32 byte hash
        assert!(serde_json::from_str::<AnyHash>(r#"{"algorithm":"sha256","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#).is_err());
        // too large hash for 32 byte hash
        assert!(serde_json::from_str::<AnyHash>(r#"{"algorithm":"sha512","hash":"0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c"}"#).is_err());
        // too large hash for 64 byte hash
    }

    #[test]
    fn it_can_correctly_serialize_preimage() {
        let hashes = sample_preimages();
        let bin = hashes[0].serialize_to_vec();
        assert_eq!(
            hex::decode("200C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C")
                .unwrap(),
            bin
        );
        let bin = hashes[1].serialize_to_vec();
        assert_eq!(hex::decode("400C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C").unwrap(), bin);
    }

    #[test]
    fn it_can_correctly_deserialize_preimage() {
        let hashes = sample_preimages();
        let bin = hex::decode("200C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C")
            .unwrap();
        let set = PreImage::deserialize_from_vec(&bin).unwrap();
        assert_eq!(hashes[0], set);
        let bin = hex::decode("400C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C0C").unwrap();
        let set = PreImage::deserialize_from_vec(&bin).unwrap();
        assert_eq!(hashes[1], set);
    }

    #[test]
    fn it_can_correctly_serialize_preimage_human_readably() {
        let hashes = sample_preimages();
        assert_eq!(
            serde_json::to_string(&hashes[0]).unwrap(),
            r#""0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c""#,
        );
        assert_eq!(
            serde_json::to_string(&hashes[1]).unwrap(),
            r#""0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c""#,
        );
    }

    #[test]
    fn it_can_correctly_deserialize_preimage_human_readably() {
        let hashes = sample_preimages();
        assert_eq!(
            serde_json::from_str::<PreImage>(
                r#""0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c""#,
            )
            .unwrap(),
            hashes[0],
        );
        assert_eq!(
            serde_json::from_str::<PreImage>(r#""0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c""#)
                .unwrap(),
            hashes[1],
        );
    }

    #[test]
    fn it_can_error_on_human_readable_preimage_deserialization() {
        assert!(serde_json::from_str::<PreImage>("[]").is_err()); // invalid type
        assert!(serde_json::from_str::<PreImage>(r#""123""#).is_err()); // too short
        assert!(serde_json::from_str::<PreImage>(
            r#""0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c""#
        )
        .is_err()); // too large for 32 byte and too small for 64 byte
        assert!(serde_json::from_str::<PreImage>(
            r#""0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c""#
        )
        .is_err()); // too large for 64 byte
    }
}
