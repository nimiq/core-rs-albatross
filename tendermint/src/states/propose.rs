use crate::{
    protocol::{Inherent, Proposal, ProposalMessage, Protocol, SignedProposalMessage},
    utils::{Return, Step},
    ProtocolError, Tendermint,
};

impl<TProtocol: Protocol> Tendermint<TProtocol> {
    /// Creates a new proposal, or re-uses the previously valid proposal.
    ///
    /// Proposing is a two step process.
    /// on the first poll the proposal is created and stored. The state is yielded, such that the upcoming state transition
    /// is set and immutable. After that the next poll will lead to the actual state transition taking place, sending the proposal
    /// and moving on to the prevote step. That way even if anything happens at any point in time during this process the persisted
    /// state is always valid to act on by any means, and a different action to the same state does not constitute a breach of protocol.
    ///
    /// One example would be a case where the preceding block to the one the instance is supposed to produce changing. I.e a fork.
    /// In that case, once a proposal is produced, it will only be acted on once it has already been persisted. Meaning if anything goes
    /// wrong recovering from the persisted state will lead to the exact same behavior.
    /// If the node fails to persist the state and crashes, restarting from the previous state and receiving the new (forked) predecessor
    /// will lead to a different proposal, but since the former proposal was not broadcast and was not acted on no harm is done, and the protocol
    /// is not breached.
    pub(crate) fn propose(&mut self) -> Result<Return<TProtocol>, ProtocolError> {
        // Retrieve the set of proposals for the current round. Create the set if it does not exist yet.
        let proposals = self
            .state
            .round_proposals
            .entry(self.state.current_round)
            .or_default();

        // At least one proposal exists. Broadcast it.
        if let Some((proposal_hash, (vr, signature))) = proposals.iter().next() {
            // A proposal exist. Broadcast it and progress to prevote step.
            log::debug!(
                current_round = self.state.current_round,
                ?proposal_hash,
                "Our turn, broadcasting previously set proposal",
            );

            // Get the proposal.
            let proposal = self
                .state
                .known_proposals
                .get(proposal_hash)
                .expect("proposal must be known")
                .clone();

            // Assemble the proposal message.
            let message = ProposalMessage {
                proposal,
                round: self.state.current_round,
                valid_round: *vr,
            };

            // Create the signed proposal message which can be broadcast.
            let signed_proposal_message = SignedProposalMessage {
                message,
                signature: signature.clone(),
            };

            // Broadcast the signed proposal message.
            self.protocol.broadcast_proposal(signed_proposal_message);

            // advance step to prevote aggregation
            self.state.current_step = Step::Prevote;

            // Store the vote for this round and step. It will always be for the proposal, except for when it is
            // locked on a value which is not the proposal.
            // Note that even though the node proposes a proposal, it might not vote for it, as valid and locked are not identical.
            let vote = self.state.locked.as_ref().map_or(
                // If no value is locked, the node is free to vote for `proposal_hash`.
                Some(proposal_hash.clone()),
                // If a locked value does exists
                |(locked_round, locked_hash)| {
                    // it must either be the same as `proposal_hash` or `vr` must exist and be more recent than `locked_round`.
                    if locked_hash == proposal_hash || &Some(*locked_round) <= vr {
                        Some(proposal_hash.clone())
                    } else {
                        None
                    }
                },
            );

            // If a vote already existed, that is a breach of protocol.
            assert!(self
                .state
                .votes
                .insert((self.state.current_round, Step::Prevote), vote)
                .is_none());

            // yield state
            return Ok(Return::Update(self.state.clone()));
        }

        // At this point `proposals` is empty.

        // Check if a valid proposal exists
        if let Some((valid_round, proposal_hash)) = &self.state.valid {
            // A valid proposal exists. Re-propose it referencing the round it was last valid for.
            log::debug!(
                current_round = self.state.current_round,
                valid_round,
                "Our turn, setting former valid proposal",
            );

            // Get the proposal.
            let proposal = self
                .state
                .known_proposals
                .get(proposal_hash)
                .expect("proposal must be known")
                .clone();

            // Assemble the proposal message.
            let message = ProposalMessage {
                proposal,
                round: self.state.current_round,
                valid_round: Some(*valid_round),
            };

            // Sign the proposal message
            let signature = self.protocol.sign_proposal(&message);

            // Store the proposal for the current round.
            proposals.insert(proposal_hash.clone(), (Some(*valid_round), signature));

            // Yield the state as it has changed.
            Ok(Return::Update(self.state.clone()))
        } else {
            // No valid proposal is known.
            log::debug!(
                current_round = self.state.current_round,
                "Our turn, setting fresh proposal",
            );

            // Create a new proposal.
            let (message, inherent) = self.protocol.create_proposal(self.state.current_round)?;

            // Sign the proposal message
            let signature = self.protocol.sign_proposal(&message);

            // Hash it for identification and voting.
            let proposal_hash = message.proposal.hash();

            // Cache the inherents created for the proposal. If they already exist overwrite them, as they must be identical.
            if let Some(_inherent) = self.state.inherents.insert(inherent.hash(), inherent) {
                // Log in case of duplicates. There might be optimization potential.
                log::trace!("Created new proposal whose inherent existed previously.");
            }

            // Store the new proposal.
            self.state
                .known_proposals
                .insert(proposal_hash.clone(), message.proposal);

            // Store the proposal for the current round.
            proposals.insert(proposal_hash, (None, signature));

            // Yield the state as it has changed.
            Ok(Return::Update(self.state.clone()))
        }
    }
}
