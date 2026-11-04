//! Pre-fork mitigations for padded `BitSet` encodings.
//!
//! Trailing zero limbs signal no slot and are invisible to every semantic check, yet a macro
//! header's punished set is hash-committed, so a padded one finalizes as permanent bloat until
//! `upgrades::v3` rejects it. Until then a validator declines to prevote for such a proposal on its
//! own account (`TendermintProtocol::endorses_proposal`), while the proposal stays valid and `push`
//! keeps accepting a block the network finalized anyway, so nodes never disagree about the chain
//! and no height can stall on the refusal. Justification signer sets are not hash-committed and are
//! normalized when decoded, so a padded proof never reaches the store.

use nimiq_block::{Block, MacroBlock, MicroJustification};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError, PushResult};
use nimiq_collections::BitSet;
use nimiq_database::traits::WriteTransaction;
use nimiq_primitives::policy::{upgrades, Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_tendermint::ProposalMessage;
use nimiq_test_log::test;
use nimiq_test_utils::{
    bitset::with_trailing_zero_limbs,
    block_production::TemporaryBlockProducer,
    blockchain::{fill_micro_blocks, produce_macro_blocks},
};

const PADDING_LIMBS: usize = 64;

fn punished_set(block: &Block) -> &BitSet {
    &block
        .unwrap_macro_ref()
        .header
        .next_batch_initial_punished_set
}

/// A blockchain at `version` with its first batch filled, so the next block is a checkpoint macro
/// block.
fn producer_at_batch_end(version: u16) -> TemporaryBlockProducer {
    let producer = TemporaryBlockProducer::new_with_protocol_version(version);
    fill_micro_blocks(&producer.producer, &producer.blockchain);
    producer
}

/// The checkpoint macro block proposal closing the current batch, as the proposer builds it.
fn next_macro_block_proposal(producer: &TemporaryBlockProducer) -> MacroBlock {
    let blockchain = producer.blockchain.read();
    producer
        .producer
        .next_macro_block_proposal(
            &blockchain,
            blockchain.timestamp() + Policy::BLOCK_SEPARATION_TIME,
            0,
            vec![],
            None,
        )
        .unwrap()
}

/// `proposal` with its punished set padded, which is what a malicious proposer would gossip.
fn pad_punished_set(mut proposal: MacroBlock) -> MacroBlock {
    let padded = with_trailing_zero_limbs(
        &proposal.header.next_batch_initial_punished_set,
        PADDING_LIMBS,
    );
    // Semantically the same set, so the post-commit check against the staking contract's
    // punished slots lets it through.
    assert_eq!(padded, proposal.header.next_batch_initial_punished_set);
    assert!(!padded.is_canonical());
    proposal.header.next_batch_initial_punished_set = padded;
    proposal.header.cached_hash = None;
    proposal
}

/// Finalizes `proposal` with a Tendermint proof signed over its header, padded or not.
fn finalize(mut proposal: MacroBlock) -> Block {
    let block_hash = proposal.hash_blake2s();
    let body = proposal.body.take().unwrap_or_default();
    let (block, _) = TemporaryBlockProducer::finalize_macro_block(
        ProposalMessage {
            valid_round: None,
            proposal: proposal.header,
            round: 0,
        },
        body,
        block_hash,
    );
    Block::Macro(block)
}

/// Runs the validator's proposal verification: an invalid proposal is dropped, a valid one is
/// stored, and only a stored one is subject to the prevote policy.
fn verify_proposal(
    producer: &TemporaryBlockProducer,
    proposal: &MacroBlock,
) -> Result<(), PushError> {
    let as_proposed = MacroBlock {
        header: proposal.header.clone(),
        body: proposal.body.clone(),
        justification: None,
    };
    producer
        .blockchain
        .read()
        .verify_macro_block_proposal(as_proposed, 0, None)
        .map(|_| ())
}

/// A padded proposal is as valid as its canonical twin, at a checkpoint and at an election block,
/// the permanent carrier. The refusal deliberately does not live here: a proposal this node
/// rejected would never be precommitted, so once an unpatched 2f+1 locked on it the height could
/// not decide (the proposer withholds its own precommit, patched validators never give theirs).
/// Instead the validator keeps the proposal and only withholds its prevote, see
/// `TendermintProtocol::endorses_proposal`. Pinned at today's version: from `upgrades::v3` on the
/// block itself becomes invalid.
#[test]
fn it_still_verifies_a_padded_proposal() {
    let checkpoint = producer_at_batch_end(upgrades::v2::VERSION);
    let election = TemporaryBlockProducer::new_with_protocol_version(upgrades::v2::VERSION);
    produce_macro_blocks(
        &election.producer,
        &election.blockchain,
        Policy::batches_per_epoch() as usize - 1,
    );
    fill_micro_blocks(&election.producer, &election.blockchain);

    for (producer, is_election) in [(checkpoint, false), (election, true)] {
        let proposal = next_macro_block_proposal(&producer);
        assert_eq!(proposal.is_election(), is_election);
        assert!(proposal
            .header
            .next_batch_initial_punished_set
            .is_canonical());
        assert_eq!(verify_proposal(&producer, &proposal), Ok(()));
        assert_eq!(
            verify_proposal(&producer, &pad_punished_set(proposal)),
            Ok(())
        );
    }
}

/// The same padded block, finalized by the network anyway, is still accepted: the policy decides
/// what this node votes for, not which chain it follows. Pinned at today's version only, since from
/// `upgrades::v3` on the block itself becomes invalid.
#[test]
fn it_still_pushes_a_finalized_padded_punished_set() {
    let producer = producer_at_batch_end(upgrades::v2::VERSION);
    let block = finalize(pad_punished_set(next_macro_block_proposal(&producer)));
    let hash = block.hash();

    assert_eq!(producer.push(block), Ok(PushResult::Extended));

    // Hash-committed, so the padding is stored as is.
    let stored = producer
        .blockchain
        .read()
        .get_block(&hash, true, None)
        .unwrap();
    assert!(!punished_set(&stored).is_canonical());
}

/// A padded justification on a finalized macro block is normalized on decode: the node stores and
/// serves the canonical proof. The hash-committed header is untouched.
#[test]
fn it_normalizes_a_padded_macro_justification_on_decode() {
    let producer = producer_at_batch_end(upgrades::v2::VERSION);
    let mut block = finalize(next_macro_block_proposal(&producer));
    let canonical_bytes = block.serialize_to_vec();

    let justification = block.unwrap_macro_ref_mut().justification.as_mut().unwrap();
    justification.sig.signers = with_trailing_zero_limbs(&justification.sig.signers, PADDING_LIMBS);
    let padded_bytes = block.serialize_to_vec();
    assert_eq!(padded_bytes.len(), canonical_bytes.len() + PADDING_LIMBS);

    let decoded = Block::deserialize_from_vec(&padded_bytes).unwrap();
    assert_eq!(decoded.serialize_to_vec(), canonical_bytes);
    assert_eq!(decoded.hash(), block.hash());

    let hash = decoded.hash();
    assert_eq!(producer.push(decoded), Ok(PushResult::Extended));

    let stored = producer
        .blockchain
        .read()
        .get_block(&hash, true, None)
        .unwrap();
    assert!(stored
        .unwrap_macro_ref()
        .justification
        .as_ref()
        .unwrap()
        .sig
        .signers
        .is_canonical());
    assert_eq!(stored.serialize_to_vec(), canonical_bytes);
}

/// Same for a skip block proof, the other justification that carries a signer set.
#[test]
fn it_normalizes_a_padded_skip_block_justification_on_decode() {
    let producer = TemporaryBlockProducer::new_with_protocol_version(upgrades::v2::VERSION);
    let mut block = producer.next_block_no_push(vec![], true);
    let canonical_bytes = block.serialize_to_vec();

    let Some(MicroJustification::Skip(proof)) = block.unwrap_micro_ref_mut().justification.as_mut()
    else {
        panic!("expected a skip block");
    };
    proof.sig.signers = with_trailing_zero_limbs(&proof.sig.signers, PADDING_LIMBS);
    let padded_bytes = block.serialize_to_vec();
    assert_eq!(padded_bytes.len(), canonical_bytes.len() + PADDING_LIMBS);

    let decoded = Block::deserialize_from_vec(&padded_bytes).unwrap();
    assert_eq!(decoded.serialize_to_vec(), canonical_bytes);
    assert_eq!(producer.push(decoded), Ok(PushResult::Extended));
}

/// A padded justification written to disk before this change comes back canonical: the chain store
/// decodes it through the same normalizer, so it is re-served canonical and rewritten canonical the
/// next time its `ChainInfo` is put.
#[test]
fn it_normalizes_a_padded_macro_justification_already_on_disk() {
    let producer = producer_at_batch_end(upgrades::v2::VERSION);
    let block = finalize(next_macro_block_proposal(&producer));
    let hash = block.hash();
    let canonical_bytes = block.serialize_to_vec();
    assert_eq!(producer.push(block.clone()), Ok(PushResult::Extended));

    // Overwrite the stored `ChainInfo` with a padded copy, as a node that took a padded gossip or
    // sync copy before this change would have written it. `BitSet` serializes verbatim, so the
    // padding reaches the disk.
    let mut padded = block;
    let justification = padded
        .unwrap_macro_ref_mut()
        .justification
        .as_mut()
        .unwrap();
    justification.sig.signers = with_trailing_zero_limbs(&justification.sig.signers, PADDING_LIMBS);
    assert_eq!(
        padded.serialize_to_vec().len(),
        canonical_bytes.len() + PADDING_LIMBS
    );
    {
        let blockchain = producer.blockchain.read();
        let mut chain_info = blockchain.get_chain_info(&hash, false, None).unwrap();
        chain_info.head = padded;
        let mut txn = blockchain.write_transaction();
        blockchain
            .chain_store
            .put_chain_info(&mut txn, &hash, &chain_info, false);
        txn.commit();
    }

    let stored = producer
        .blockchain
        .read()
        .get_block(&hash, true, None)
        .unwrap();
    assert!(stored
        .unwrap_macro_ref()
        .justification
        .as_ref()
        .unwrap()
        .sig
        .signers
        .is_canonical());
    assert_eq!(stored.serialize_to_vec(), canonical_bytes);
}
