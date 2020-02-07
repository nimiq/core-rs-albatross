use failure::Fail;

use beserial::{BigEndian, Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls;
use collections::bitset::BitSet;

#[derive(Clone, Debug, Fail)]
pub enum SignatureError {
    #[fail(display = "Signatures are overlapping: {:?}", _0)]
    Overlapping(BitSet),
    #[fail(display = "Individual signature is already contained: {:?}", _0)]
    Contained(usize),
}

#[derive(Clone, Debug)]
pub struct IndividualSignature {
    pub signature: bls::Signature,
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
    pub fn new(signature: bls::Signature, signer: usize) -> Self {
        Self { signature, signer }
    }

    pub fn as_multisig(&self) -> MultiSignature {
        let mut aggregate = bls::AggregateSignature::new();
        let mut signers = BitSet::new();

        aggregate.aggregate(&self.signature);
        signers.insert(self.signer);

        MultiSignature::new(aggregate, signers)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MultiSignature {
    pub signature: bls::AggregateSignature,
    pub signers: BitSet,
}

impl MultiSignature {
    pub fn new(signature: bls::AggregateSignature, signers: BitSet) -> Self {
        Self { signature, signers }
    }

    pub fn len(&self) -> usize {
        self.signers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signers.is_empty()
    }

    pub fn add_multisig(&mut self, other: &MultiSignature) -> Result<(), SignatureError> {
        // TODO: If we don't need the overlapping IDs for the error, we can use `intersection_size`
        let overlap = &self.signers & &other.signers;

        if overlap.is_empty() {
            self.signature.merge_into(&other.signature);
            self.signers = &self.signers | &other.signers;
            Ok(())
        } else {
            Err(SignatureError::Overlapping(overlap))
        }
    }

    pub fn add_individual(&mut self, other: &IndividualSignature) -> Result<(), SignatureError> {
        if self.signers.contains(other.signer) {
            Err(SignatureError::Contained(other.signer))
        } else {
            self.signature.aggregate(&other.signature);
            self.signers.insert(other.signer);
            Ok(())
        }
    }

    pub fn add(&mut self, other: &Signature) -> Result<(), SignatureError> {
        match other {
            Signature::Individual(individual) => self.add_individual(individual),
            Signature::Multi(multisig) => self.add_multisig(multisig),
        }
    }
}

impl From<IndividualSignature> for MultiSignature {
    fn from(signature: IndividualSignature) -> Self {
        signature.as_multisig()
    }
}

#[derive(Debug, Clone)]
pub enum Signature {
    Individual(IndividualSignature),
    Multi(MultiSignature),
}

impl Signature {
    pub fn signers_bitset(&self) -> BitSet {
        match self {
            Signature::Individual(individual) => {
                let mut s = BitSet::new();
                s.insert(individual.signer);
                s
            }
            Signature::Multi(multisig) => multisig.signers.clone(),
        }
    }

    // TODO: Don't use `Box`. We need a specific type for the `BitSet` iterator. Then we can create an
    // enum that is either a `Once` iterator or an iterator over the `BitSet`.
    pub fn signers<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> {
        match self {
            Signature::Individual(individual) => Box::new(std::iter::once(individual.signer)),
            Signature::Multi(multisig) => Box::new(multisig.signers.iter()),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Signature::Individual(_) => 1,
            Signature::Multi(multisig) => multisig.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Signature::Individual(_) => false,
            Signature::Multi(multisig) => multisig.is_empty(),
        }
    }

    pub fn is_individual(&self) -> bool {
        match self {
            Signature::Individual(_) => true,
            _ => false,
        }
    }

    pub fn is_multisig(&self) -> bool {
        match self {
            Signature::Multi(_) => true,
            _ => false,
        }
    }
}

impl From<IndividualSignature> for Signature {
    fn from(individual: IndividualSignature) -> Signature {
        Signature::Individual(individual)
    }
}

impl From<MultiSignature> for Signature {
    fn from(multisig: MultiSignature) -> Signature {
        Signature::Multi(multisig)
    }
}

impl From<Signature> for MultiSignature {
    fn from(signature: Signature) -> MultiSignature {
        match signature {
            Signature::Individual(individual) => individual.into(),
            Signature::Multi(multisig) => multisig,
        }
    }
}
