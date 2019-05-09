use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls::bls12_381::{PublicKey as BlsPublicKey, Signature as BlsSignature};
use keys::Address;
use primitives::account::AccountType;

use crate::{Transaction, TransactionError, TransactionFlags};
use crate::account::AccountTransactionVerification;
use crate::SignatureProof;

pub struct StakingContractVerifier {}

impl AccountTransactionVerification for StakingContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::Staking);

        if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            warn!("Contract creation not allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        let parameters = StakingTransactionData::parse(transaction)?;
        parameters.verify()?;

        let is_self_transaction = transaction.sender == transaction.recipient;
        let is_retire_transaction = parameters.transaction_type() == StakingTransactionType::Retire;
        if is_self_transaction != is_retire_transaction {
            warn!("Only retire transactions can be self transactions");
            return Err(TransactionError::InvalidForRecipient);
        }

        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::Vesting);

        // Verify signature.
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;
        if !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }
}


#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum StakingTransactionType {
    Stake = 0,
    Retire = 1,
}

#[derive(Clone, Debug)]
pub enum StakingTransactionData {
    Stake {
        validator_key: BlsPublicKey,
        reward_address: Option<Address>,
        proof_of_knowledge: BlsSignature,
    },
    Retire,
}

impl StakingTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.data[..];
        let parameters = Deserialize::deserialize(reader)
            .map_err(|e| TransactionError::from(e))?;

        // Ensure that transaction data has been fully read.
        if reader.read_u8().is_ok() {
            return Err(TransactionError::InvalidData);
        }

        Ok(parameters)
    }

    /// Important: Currently, the proof of knowledge of the secret key is a signature of the public key.
    /// If an attacker A ever tricks a validator B into signing a message with content `pk_A - pk_B`,
    /// where `pk_X` is X's BLS public key, A will be able to sign aggregate messages that are valid for
    /// public keys `pk_B + (pk_A - pk_B) = pk_B`.
    /// Alternatives would be to replace the proof of knowledge by a zero-knowledge proof.
    pub fn verify(&self) -> Result<(), TransactionError> {
        match self {
            StakingTransactionData::Stake { validator_key, proof_of_knowledge, .. } => {
                if !validator_key.verify(validator_key, &proof_of_knowledge) {
                    return Err(TransactionError::InvalidData)
                }
            },
            StakingTransactionData::Retire => (),
        }
        Ok(())
    }

    pub fn transaction_type(&self) -> StakingTransactionType {
        match self {
            StakingTransactionData::Stake {..} => StakingTransactionType::Stake,
            StakingTransactionData::Retire {..} => StakingTransactionType::Retire,
        }
    }
}

impl Serialize for StakingTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = self.transaction_type().serialize(writer)?;
        match self {
            StakingTransactionData::Stake { validator_key, reward_address, proof_of_knowledge } => {
                size += validator_key.serialize(writer)?;
                size += reward_address.serialize(writer)?;
                size += proof_of_knowledge.serialize(writer)?;
            },
            StakingTransactionData::Retire => (),
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = self.transaction_type().serialized_size();
        match self {
            StakingTransactionData::Stake { validator_key, reward_address, proof_of_knowledge } => {
                size += validator_key.serialized_size();
                size += reward_address.serialized_size();
                size += proof_of_knowledge.serialized_size();
            },
            StakingTransactionData::Retire => (),
        }
        size
    }
}

impl Deserialize for StakingTransactionData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: StakingTransactionType = Deserialize::deserialize(reader)?;
        match ty {
            StakingTransactionType::Stake => {
                let validator_key = Deserialize::deserialize(reader)?;
                let reward_address= Deserialize::deserialize(reader)?;
                let proof_of_knowledge = Deserialize::deserialize(reader)?;
                Ok(StakingTransactionData::Stake { validator_key, reward_address, proof_of_knowledge })
            },
            StakingTransactionType::Retire => {
                Ok(StakingTransactionData::Retire)
            },
        }
    }
}

