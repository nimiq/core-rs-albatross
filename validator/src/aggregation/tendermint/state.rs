use std::{collections::BTreeMap, fmt::Debug, io};

use beserial::{Deserialize, DeserializeWithLength, Serialize, SerializeWithLength};
use nimiq_block::{MacroBody, MacroHeader};
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Blake2sHash};
use nimiq_keys::Signature as SchnorrSignature;
use nimiq_tendermint::{State as TendermintState, Step};
use nimiq_validator_network::ValidatorNetwork;

use crate::tendermint::TendermintProtocol;

use super::{
    contribution::TendermintContribution,
    proposal::{Body, Header, SignedProposal},
};

#[derive(Clone)]
pub struct MacroState {
    pub(crate) block_number: u32,
    round_number: u32,
    step: Step,
    known_proposals: BTreeMap<Blake2sHash, MacroHeader>,
    round_proposals: BTreeMap<u32, BTreeMap<Blake2sHash, (Option<u32>, (SchnorrSignature, u16))>>,
    votes: BTreeMap<(u32, Step), Option<Blake2sHash>>,
    best_votes: BTreeMap<(u32, Step), TendermintContribution>,
    inherents: BTreeMap<Blake2bHash, MacroBody>,
    locked: Option<(u32, Blake2sHash)>,
    valid: Option<(u32, Blake2sHash)>,
}

impl Debug for MacroState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("MacroState");
        dbg.field("round", &self.round_number);
        dbg.field("step", &self.step);
        dbg.field("locked", &self.locked);
        dbg.field("valid", &self.valid);
        dbg.field("best_votes", &self.best_votes);
        dbg.field("votes", &self.votes);

        dbg.finish()
    }
}

impl MacroState {
    pub fn from_tendermint_state<TValidatorNetwork>(
        block_number: u32,
        state: TendermintState<TendermintProtocol<TValidatorNetwork>>,
    ) -> Self
    where
        TValidatorNetwork: ValidatorNetwork + 'static,
        <TValidatorNetwork as ValidatorNetwork>::PubsubId: Unpin,
    {
        let mut known_proposals = BTreeMap::default();
        for (proposal_hash, Header(proposal, _)) in state.known_proposals.into_iter() {
            known_proposals.insert(proposal_hash, proposal);
        }
        let mut inherents = BTreeMap::default();
        for (inherent_hash, Body(macro_body)) in state.inherents.into_iter() {
            inherents.insert(inherent_hash, macro_body);
        }

        Self {
            block_number,
            round_number: state.current_round,
            step: state.current_step,
            known_proposals,
            round_proposals: state.round_proposals,
            votes: state.votes,
            best_votes: state.best_votes,
            inherents,
            locked: state.locked,
            valid: state.valid,
        }
    }

    pub fn into_tendermint_state<TValidatorNetwork>(
        self,
        reference_height: u32,
    ) -> Option<TendermintState<TendermintProtocol<TValidatorNetwork>>>
    where
        TValidatorNetwork: ValidatorNetwork + 'static,
        <TValidatorNetwork as ValidatorNetwork>::PubsubId: Unpin,
    {
        if self.block_number != reference_height {
            return None;
        }

        let mut known_proposals = BTreeMap::default();
        for (proposal_hash, proposal) in self.known_proposals.iter() {
            known_proposals.insert(
                proposal_hash.clone(),
                Header::<<TValidatorNetwork as ValidatorNetwork>::PubsubId>(proposal.clone(), None),
            );
        }
        let mut inherents = BTreeMap::default();
        for (inherent_hash, inherent) in self.inherents.iter() {
            inherents.insert(inherent_hash.clone(), Body(inherent.clone()));
        }
        Some(TendermintState {
            current_round: self.round_number,
            current_step: self.step,
            known_proposals,
            round_proposals: self.round_proposals,
            votes: self.votes,
            best_votes: self.best_votes,
            inherents,
            locked: self.locked,
            valid: self.valid,
        })
    }

    pub fn get_proposal_for(
        &self,
        block_number: u32,
        round_number: u32,
        proposal_hash: &Blake2sHash,
    ) -> Option<SignedProposal> {
        if self.block_number != block_number {
            return None;
        }

        let (valid_round, signature) = self
            .round_proposals
            .get(&round_number)?
            .get(proposal_hash)?
            .clone();

        let proposal = self
            .known_proposals
            .get(proposal_hash)
            .expect("Proposal which exists in round_proposals must also exist in known_proposals.")
            .clone();

        Some(SignedProposal {
            proposal,
            round: round_number,
            valid_round,
            signature: signature.0,
            signer: signature.1,
        })
    }
}

struct State<TValidatorNetwork: ValidatorNetwork + 'static>(
    pub u32,
    pub TendermintState<TendermintProtocol<TValidatorNetwork>>,
)
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin;

impl<TValidatorNetwork: ValidatorNetwork + 'static> Clone for State<TValidatorNetwork>
where
    <TValidatorNetwork as ValidatorNetwork>::PubsubId: std::fmt::Debug + Unpin,
{
    fn clone(&self) -> Self {
        Self(self.0, self.1.clone())
    }
}

impl Serialize for MacroState {
    fn serialize<W: beserial::WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, beserial::SerializingError> {
        let mut size = 0;

        size += Serialize::serialize(&self.block_number, writer)?;
        size += Serialize::serialize(&self.round_number, writer)?;
        size += Serialize::serialize(&(self.step as u8), writer)?;
        size += SerializeWithLength::serialize::<u32, _>(&self.known_proposals, writer)?;
        size += Serialize::serialize(&(self.round_proposals.len() as u32), writer)?;
        for (round, proposals) in self.round_proposals.iter() {
            size += Serialize::serialize(round, writer)?;
            size += SerializeWithLength::serialize::<u32, _>(proposals, writer)?;
        }
        size += Serialize::serialize(&(self.votes.len() as u32), writer)?;
        for ((round, step), vote) in self.votes.iter() {
            size += Serialize::serialize(round, writer)?;
            size += Serialize::serialize(&(*step as u8), writer)?;
            size += Serialize::serialize(vote, writer)?;
        }
        size += Serialize::serialize(&(self.best_votes.len() as u32), writer)?;
        for ((round, step), vote) in self.best_votes.iter() {
            size += Serialize::serialize(round, writer)?;
            size += Serialize::serialize(&(*step as u8), writer)?;
            size += Serialize::serialize(vote, writer)?;
        }
        size += SerializeWithLength::serialize::<u32, _>(&self.inherents, writer)?;
        size += Serialize::serialize(&self.locked, writer)?;
        size += Serialize::serialize(&self.valid, writer)?;

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        // (height is u32) + (round is u32) + (step is u8) + 3 * (BTreeMap.len() is u32)
        let mut size = Serialize::serialized_size(&self.block_number)
            + Serialize::serialized_size(&self.round_number)
            + 1; // = Serialize::serialized_size(&self.step as u8)

        size += SerializeWithLength::serialized_size::<u32>(&self.known_proposals);

        size += 4; // = Serialize::serialized_size(&self.round_proposals.len() as u32)
        for (_round, proposals) in self.round_proposals.iter() {
            // round is u32
            size += 4 + SerializeWithLength::serialized_size::<u32>(proposals);
        }

        size += 4; // = Serialize::serialized_size(&self.votes.len() as u32)
        for ((_round, _step), vote) in self.votes.iter() {
            // round is u32, step is u8
            size += 5 + Serialize::serialized_size(vote);
        }

        size += 4; // = Serialize::serialized_size(&self.best_votes.len() as u32)
        for ((_round, _step), vote) in self.best_votes.iter() {
            // round is u32, step is u8
            size += 5 + Serialize::serialized_size(vote);
        }

        size += Serialize::serialized_size(&self.locked);
        size += Serialize::serialized_size(&self.valid);

        size
    }
}

impl Deserialize for MacroState {
    fn deserialize<R: beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, beserial::SerializingError> {
        let block_number = Deserialize::deserialize(reader)?;

        let round_number = Deserialize::deserialize(reader)?;
        let step: u8 = Deserialize::deserialize(reader)?;
        let step = Step::try_from(step).map_err(|_| beserial::SerializingError::InvalidValue)?;
        let known_proposals = DeserializeWithLength::deserialize::<u32, _>(reader)?;

        let num_round_proposals: u32 = Deserialize::deserialize(reader)?;
        let mut round_proposals = BTreeMap::new();
        for _ in 0..num_round_proposals {
            let key = Deserialize::deserialize(reader)?;
            let value = DeserializeWithLength::deserialize::<u32, _>(reader)?;
            round_proposals.insert(key, value);
        }

        let num_votes: u32 = Deserialize::deserialize(reader)?;
        let mut votes = BTreeMap::new();
        for _ in 0..num_votes {
            let round = Deserialize::deserialize(reader)?;
            let step: u8 = Deserialize::deserialize(reader)?;
            let step =
                Step::try_from(step).map_err(|_| beserial::SerializingError::InvalidValue)?;
            let value = Deserialize::deserialize(reader)?;
            votes.insert((round, step), value);
        }
        let num_best_votes: u32 = Deserialize::deserialize(reader)?;
        let mut best_votes = BTreeMap::new();
        for _ in 0..num_best_votes {
            let round = Deserialize::deserialize(reader)?;
            let step: u8 = Deserialize::deserialize(reader)?;
            let step =
                Step::try_from(step).map_err(|_| beserial::SerializingError::InvalidValue)?;
            let value = Deserialize::deserialize(reader)?;
            best_votes.insert((round, step), value);
        }
        let inherents = DeserializeWithLength::deserialize::<u32, _>(reader)?;
        let locked = Deserialize::deserialize(reader)?;
        let valid = Deserialize::deserialize(reader)?;

        let state = MacroState {
            block_number,
            round_number,
            step,
            known_proposals,
            round_proposals,
            votes,
            best_votes,
            inherents,
            locked,
            valid,
        };

        Ok(state)
    }
}

impl IntoDatabaseValue for MacroState {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for MacroState {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
