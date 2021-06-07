use beserial::{BigEndian, Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_bls::{AggregateSignature, Signature};
use nimiq_collections::bitset::BitSet;
use nimiq_handel::contribution::{AggregatableContribution, ContributionError};

/*
This does not really belong here, but as there would otherwise be a cyclic dependency it needs to be here for now.
TODO: Move this out of primitives and into validator/aggregation once the messages crate is no longer required.
*/

#[derive(Clone, Debug)]
pub struct IndividualSignature {
    pub signature: Signature,
    pub signer: usize,
}

impl Serialize for IndividualSignature {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let n = self.signature.serialize(writer)?;
        writer.write_u16::<BigEndian>(self.signer as u16)?;
        Ok(n + 2)
    }

    fn serialized_size(&self) -> usize {
        self.signature.serialized_size() + 2
    }
}

impl Deserialize for IndividualSignature {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        Ok(Self {
            signature: Deserialize::deserialize(reader)?,
            signer: reader.read_u16::<BigEndian>()? as usize,
        })
    }
}

impl IndividualSignature {
    pub fn new(signature: Signature, signer: usize) -> Self {
        Self { signature, signer }
    }

    pub fn as_multisig(&self) -> MultiSignature {
        let mut aggregate = AggregateSignature::new();
        let mut signers = BitSet::new();

        aggregate.aggregate(&self.signature);
        signers.insert(self.signer);

        MultiSignature::new(aggregate, signers)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct MultiSignature {
    pub signature: AggregateSignature,
    pub signers: BitSet,
}

impl MultiSignature {
    pub fn new(signature: AggregateSignature, signers: BitSet) -> Self {
        Self { signature, signers }
    }
}

impl AggregatableContribution for MultiSignature {
    fn contributors(&self) -> BitSet {
        self.signers.clone()
    }

    fn combine(&mut self, other: &MultiSignature) -> Result<(), ContributionError> {
        // TODO: If we don't need the overlapping IDs for the error, we can use `intersection_size`
        let overlap = &self.signers & &other.signers;

        if overlap.is_empty() {
            self.signature.merge_into(&other.signature);
            self.signers = &self.signers | &other.signers;
            Ok(())
        } else {
            Err(ContributionError::Overlapping(overlap))
        }
    }
}

impl From<IndividualSignature> for MultiSignature {
    fn from(signature: IndividualSignature) -> Self {
        signature.as_multisig()
    }
}
