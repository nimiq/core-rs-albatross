use beserial::{Deserialize, Serialize};
use block_albatross::signed::{Message, SignedMessage, PREFIX_TENDERMINT_PROPOSAL};
use block_albatross::{
    BlockHeader, MacroBlock, MacroHeader, SignedTendermintProposal, TendermintProposal,
};
use block_production_albatross::BlockProducer;
use blockchain_albatross::Blockchain;
use bls::KeyPair;
use failure::_core::time::Duration;
use futures::stream::Stream;
use hash::{Blake2bHash, Hash, SerializeContent};
use network_interface::network::ReceiveFromAll;
use primitives::slot::SlotCollection;
use std::sync::Arc;
use tendermint::TendermintOutsideDeps;
use utils::time::OffsetTime;

use async_trait::async_trait;
use failure::Fail;
use nimiq_collections::BitSet;
use std::{fmt, io};

struct TendermintOutsideDepsImpl {
    offset_time: Arc<OffsetTime>,
    block_producer: Arc<BlockProducer>,
    chain: Arc<Blockchain>,
    validator_id: Option<usize>,
    validator_key: KeyPair,
}

// #[async_trait]
// impl TendermintOutsideDeps for TendermintOutsideDepsImpl {
//     type ProposalTy = SignedProposal;
//     type ProofTy = bool;
//     type ResultTy = MacroBlock;
//
//     fn verify_state(&self, state: &TendermintState<Self::ProposalTy, Self::ProofTy>) -> bool {
//         unimplemented!()
//     }
//
//     fn is_our_turn(&self, round: u32) -> bool {
//         let (slot, _) = match self.chain.get_slot_at(self.chain.height() + 1, round, None) {
//             Some(slot) => slot,
//             None => {
//                 return false;
//             }
//         };
//
//         let our_public_key = self.validator_key.public_key.compress();
//         if slot.public_key().compressed() == &our_public_key {
//             return true;
//         }
//
//         return false;
//     }
//
//     fn get_value(&self, round: u32) -> Result<Self::ProposalTy, TendermintError> {
//         let pk_idx = match self
//             .chain
//             .current_validators()
//             .find_idx_and_num_slots_by_public_key(&self.validator_key.public_key.compress())
//         {
//             Some((pk_idx, _)) => pk_idx,
//             None => return None,
//         };
//
//         let proposal = self
//             .block_producer
//             .next_macro_block_proposal(self.offset_time.now(), round, None, Vec::new())
//             .0;
//         let signed_proposal =
//             SignedProposal::from_message(proposal, &self.validator_key.secret_key, pk_idx);
//         Some(signed_proposal)
//     }
//
//     fn assemble_block(
//         &self,
//         proposal: Arc<Self::ProposalTy>,
//         proof: Self::ProofTy,
//     ) -> Self::ResultTy {
//         MacroBlock {
//             header: proposal.message.header.clone(),
//             justification: None,
//             extrinsics: Some(self.block_producer.next_macro_extrinsics(Vec::new())),
//         }
//     }
//
//     async fn broadcast_proposal(
//         &self,
//         round: u32,
//         proposal: Self::ProposalTy,
//         valid_round: Option<u32>,
//     ) -> Result<(), TendermintError> {
//         unimplemented!()
//     }
//
//     async fn await_proposal(
//         &self,
//         round: u32,
//     ) -> Result<ProposalResult<Self::ProposalTy>, TendermintError> {
//         let block_number = msg.message.header.block_number;
//         let view_number = msg.message.header.view_number; // TODO round
//
//         if block_number != self.chain.height() + 1 {
//             return false;
//         }
//
//         if round != round {
//             // TODO check round
//             return false;
//         }
//
//         // Get current validator
//         let (slot, slot_number) = match self.chain.get_slot_at(block_number, view_number, None) {
//             Some(s) => s,
//             None => {
//                 warn!("[Tendermint] Need slot to verify the proposal");
//                 return false;
//             }
//         };
//
//         let validator_id_opt = self
//             .chain
//             .current_validators()
//             .get_band_number_by_slot_number(slot_number);
//
//         // Check if the proposal comes from the correct validator id
//         if validator_id_opt != Some(msg.signer_idx) {
//             return false;
//         }
//
//         let public_key = slot.public_key().uncompress_unchecked();
//
//         // Check the validity of the block
//         let result = self.chain.verify_block_header(
//             &BlockHeader::Macro(msg.message.header.clone()),
//             blockchain_albatross::blockchain::OptionalCheck::Skip,
//             &public_key,
//             None,
//         );
//         if let Err(e) = result {
//             debug!("[Tendermint-PROPOSAL] Invalid macro block header: {:?}", e);
//             return false;
//         }
//
//         // Check the signature of the proposal
//         if !msg.verify(&public_key) {
//             debug!("[Tendermint-PROPOSAL] Invalid signature");
//             return false;
//         }
//
//         true
//     }
//
//     async fn broadcast_and_aggregate(
//         &mut self,
//         round: u32,
//         step: Step,
//         proposal: Option<Blake2sHash>,
//     ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
//         unimplemented!()
//     }
//
//     async fn get_aggregation(
//         &self,
//         round: u32,
//         step: Step,
//     ) -> Result<AggregationResult<Self::ProofTy>, TendermintError> {
//         unimplemented!()
//     }
// }
