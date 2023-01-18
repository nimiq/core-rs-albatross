use crate::{BlsKeyPair, SchnorrKeyPair};
use nimiq_account::Inherent;
use nimiq_block::{
    Block, ForkProof, MacroBlock, MacroBody, MacroHeader, MicroBlock, MicroBody, MicroHeader,
    MicroJustification, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo, SkipBlockProof,
    TendermintIdentifier, TendermintProof, TendermintProposal, TendermintStep, TendermintVote,
};
use nimiq_blockchain::{Blockchain, ExtendedTransaction};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::AggregateSignature;
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_primitives::policy::Policy;
use nimiq_transaction::Transaction;
use nimiq_vrf::VrfSeed;

#[derive(Clone, Default)]
pub struct BlockConfig {
    pub version: Option<u16>,
    pub block_number_offset: i32,
    pub timestamp_offset: i64,
    pub parent_hash: Option<Blake2bHash>,
    pub seed: Option<VrfSeed>,
    pub missing_body: bool,
    pub body_hash: Option<Blake2bHash>,
    pub state_root: Option<Blake2bHash>,
    pub history_root: Option<Blake2bHash>,

    // Skip only
    pub skip_block_proof: Option<SkipBlockProof>,

    // Micro only
    pub micro_only: bool,
    pub fork_proofs: Vec<ForkProof>,
    pub transactions: Vec<Transaction>,
    pub extra_data: Vec<u8>,

    // Macro only
    pub macro_only: bool,
    pub parent_election_hash: Option<Blake2bHash>,
    pub tendermint_round: Option<u32>,
}

/// `config` can be used to generate blocks that can be invalid in some way. config == Default creates a valid block.
pub fn next_micro_block(
    signing_key: &SchnorrKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> MicroBlock {
    let block_number = (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32;

    let timestamp = (blockchain.head().timestamp() as i64 + 1 + config.timestamp_offset) as u64;

    let parent_hash = config
        .parent_hash
        .clone()
        .unwrap_or_else(|| blockchain.head_hash());

    let prev_seed = blockchain.head().seed().clone();
    let seed = config
        .seed
        .clone()
        .unwrap_or_else(|| prev_seed.sign_next(signing_key));

    let mut transactions = config.transactions.clone();
    transactions.sort_unstable();

    let inherents = blockchain.create_slash_inherents(&config.fork_proofs, None, None);

    let (state_root, executed_txns) = blockchain
        .state()
        .accounts
        .exercise_transactions(&transactions, &inherents, block_number, timestamp)
        .expect("Failed to compute accounts hash during block production");

    let ext_txs = ExtendedTransaction::from(
        blockchain.network_id,
        block_number,
        timestamp,
        executed_txns.clone(),
        inherents,
    );

    let mut txn = blockchain.write_transaction();

    let history_root = config.history_root.clone().unwrap_or_else(|| {
        blockchain
            .history_store
            .add_to_history(&mut txn, Policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.")
            .0
    });

    txn.abort();

    let body = MicroBody {
        fork_proofs: config.fork_proofs.clone(),
        transactions: executed_txns,
    };

    let header = MicroHeader {
        version: config.version.unwrap_or(Policy::VERSION),
        block_number,
        timestamp,
        parent_hash,
        seed,
        extra_data: config.extra_data.clone(),
        state_root,
        body_root: config.body_hash.clone().unwrap_or_else(|| body.hash()),
        history_root,
    };

    let hash = header.hash::<Blake2bHash>();
    let signature = signing_key.sign(hash.as_slice());

    MicroBlock {
        header,
        body: if !config.missing_body {
            Some(body)
        } else {
            None
        },
        justification: Some(MicroJustification::Micro(signature)),
    }
}

/// `config` can be used to generate blocks that can be invalid in some way. config == Default creates a valid block.
pub fn next_skip_block(
    voting_key: &BlsKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> MicroBlock {
    let block_number = (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32;

    let timestamp = if config.timestamp_offset != 0 {
        (blockchain.head().timestamp() as i64 + config.timestamp_offset) as u64
    } else {
        blockchain.head().timestamp() + Policy::BLOCK_PRODUCER_TIMEOUT
    };

    let parent_hash = config
        .parent_hash
        .clone()
        .unwrap_or_else(|| blockchain.head_hash());

    let prev_seed = blockchain.head().seed().clone();

    let skip_block_info = SkipBlockInfo {
        block_number,
        vrf_entropy: prev_seed.entropy(),
    };

    // Create the inherents from the skip block info.
    let inherents = blockchain.create_slash_inherents(&[], Some(skip_block_info), None);

    let state_root = config.state_root.clone().unwrap_or_else(|| {
        let (state_root, _) = blockchain
            .state()
            .accounts
            .exercise_transactions(&[], &inherents, block_number, timestamp)
            .expect("Failed to compute accounts hash during block production");
        state_root
    });

    let ext_txs = ExtendedTransaction::from(
        blockchain.network_id,
        block_number,
        timestamp,
        vec![],
        inherents,
    );

    let mut txn = blockchain.write_transaction();

    let history_root = config.history_root.clone().unwrap_or_else(|| {
        blockchain
            .history_store
            .add_to_history(&mut txn, Policy::epoch_at(block_number), &ext_txs)
            .expect("Failed to compute history root during block production.")
            .0
    });

    txn.abort();

    let body = MicroBody {
        fork_proofs: vec![],
        transactions: vec![],
    };

    let header = MicroHeader {
        version: config.version.unwrap_or(Policy::VERSION),
        block_number,
        timestamp,
        parent_hash,
        seed: prev_seed,
        extra_data: config.extra_data.clone(),
        state_root,
        body_root: config.body_hash.clone().unwrap_or_else(|| body.hash()),
        history_root,
    };

    let skip_block_proof = create_skip_block_proof(voting_key, blockchain, config);

    MicroBlock {
        header,
        justification: Some(MicroJustification::Skip(skip_block_proof)),
        body: if !config.missing_body {
            Some(body)
        } else {
            None
        },
    }
}

fn next_macro_block_proposal(
    signing_key: &SchnorrKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> MacroBlock {
    let block_number = (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32;

    let timestamp = (blockchain.head().timestamp() as i64 + config.timestamp_offset) as u64;

    let parent_hash = config
        .parent_hash
        .clone()
        .unwrap_or_else(|| blockchain.head_hash());

    let parent_election_hash = config
        .parent_election_hash
        .clone()
        .unwrap_or_else(|| blockchain.election_head_hash());

    let seed = config
        .seed
        .clone()
        .unwrap_or_else(|| blockchain.head().seed().sign_next(signing_key));

    let mut header = MacroHeader {
        version: config.version.unwrap_or(Policy::VERSION),
        block_number,
        round: 0,
        timestamp,
        parent_hash,
        parent_election_hash,
        seed,
        extra_data: config.extra_data.clone(),
        state_root: Blake2bHash::default(),
        body_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let state = blockchain.state();

    let disabled_set = blockchain.get_staking_contract().previous_disabled_slots();

    let lost_reward_set = blockchain.get_staking_contract().previous_lost_rewards();

    let validators = if Policy::is_election_block_at(blockchain.block_number() + 1) {
        Some(blockchain.next_validators(&header.seed))
    } else {
        None
    };

    let pk_tree_root = validators
        .as_ref()
        .and_then(|validators| MacroBlock::pk_tree_root(validators).ok());

    let body = MacroBody {
        validators,
        pk_tree_root,
        lost_reward_set,
        disabled_set,
    };

    header.body_root = config.body_hash.clone().unwrap_or_else(|| body.hash());

    let mut macro_block = MacroBlock {
        header,
        body: Some(body),
        justification: None,
    };

    let inherents: Vec<Inherent> = blockchain.create_macro_block_inherents(state, &macro_block);

    let (root, _) = state
        .accounts
        .exercise_transactions(&[], &inherents, block_number, timestamp)
        .expect("Failed to compute accounts hash during block production.");

    macro_block.header.state_root = root;

    let ext_txs = ExtendedTransaction::from(
        blockchain.network_id,
        block_number,
        timestamp,
        vec![],
        inherents,
    );

    let mut txn = blockchain.write_transaction();

    macro_block.header.history_root = blockchain
        .history_store
        .add_to_history(&mut txn, Policy::epoch_at(block_number), &ext_txs)
        .expect("Failed to compute history root during block production.")
        .0;

    txn.abort();

    macro_block
}

pub fn finalize_macro_block(
    voting_key: &BlsKeyPair,
    proposal: TendermintProposal,
    body: MacroBody,
    block_hash: Blake2sHash,
    config: &BlockConfig,
) -> MacroBlock {
    let vote = TendermintVote {
        proposal_hash: Some(block_hash),
        id: TendermintIdentifier {
            block_number: proposal.value.block_number,
            step: TendermintStep::PreCommit,
            round_number: proposal.round,
        },
    };

    let signature = AggregateSignature::from_signatures(&[voting_key
        .secret_key
        .sign(&vote)
        .multiply(Policy::SLOTS)]);

    let mut signers = BitSet::new();
    for i in 0..Policy::SLOTS {
        signers.insert(i as usize);
    }

    let justification = Some(TendermintProof {
        round: 0,
        sig: MultiSignature::new(signature, signers),
    });

    MacroBlock {
        header: proposal.value,
        justification,
        body: if config.missing_body {
            None
        } else {
            Some(body)
        },
    }
}

pub fn next_macro_block(
    signing_key: &SchnorrKeyPair,
    voting_key: &BlsKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> Block {
    let height = blockchain.block_number() + 1;

    assert!(Policy::is_macro_block_at(height));

    let macro_block_proposal = next_macro_block_proposal(signing_key, blockchain, config);

    let block_hash = macro_block_proposal.nano_zkp_hash(true);

    let validators =
        blockchain.get_validators_for_epoch(Policy::epoch_at(blockchain.block_number() + 1), None);
    assert!(validators.is_ok());

    Block::Macro(finalize_macro_block(
        voting_key,
        TendermintProposal {
            valid_round: None,
            value: macro_block_proposal.header,
            round: config.tendermint_round.unwrap_or(0),
        },
        macro_block_proposal
            .body
            .or_else(|| Some(MacroBody::default()))
            .unwrap(),
        block_hash,
        config,
    ))
}

fn create_skip_block_proof(
    voting_key_pair: &BlsKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> SkipBlockProof {
    let seed = config
        .seed
        .clone()
        .unwrap_or_else(|| blockchain.head().seed().clone());

    let skip_block_info = SkipBlockInfo {
        block_number: (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32,
        vrf_entropy: seed.entropy(),
    };

    let skip_block_info =
        SignedSkipBlockInfo::from_message(skip_block_info, &voting_key_pair.secret_key, 0);

    let signature =
        AggregateSignature::from_signatures(&[skip_block_info.signature.multiply(Policy::SLOTS)]);
    let mut signers = BitSet::new();
    for i in 0..Policy::SLOTS {
        signers.insert(i as usize);
    }

    SkipBlockProof {
        sig: MultiSignature::new(signature, signers),
    }
}
