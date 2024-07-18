use nimiq_account::BlockState;
use nimiq_block::{
    Block, EquivocationProof, MacroBlock, MacroBody, MacroHeader, MicroBlock, MicroBody,
    MicroHeader, MicroJustification, MultiSignature, SignedSkipBlockInfo, SkipBlockInfo,
    SkipBlockProof, TendermintProof,
};
use nimiq_blockchain::{interface::HistoryInterface, Blockchain};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair};
use nimiq_collections::BitSet;
use nimiq_database::traits::WriteTransaction;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::KeyPair as SchnorrKeyPair;
use nimiq_primitives::{
    networks::NetworkId, policy::Policy, TendermintIdentifier, TendermintStep, TendermintVote,
};
use nimiq_tendermint::ProposalMessage;
use nimiq_transaction::{
    historic_transaction::HistoricTransaction, inherent::Inherent, Transaction,
};
use nimiq_vrf::VrfSeed;

#[derive(Clone)]
pub struct BlockConfig {
    pub network: Option<NetworkId>,
    pub version: Option<u16>,
    pub block_number_offset: i32,
    pub timestamp_offset: i64,
    pub parent_hash: Option<Blake2bHash>,
    pub seed: Option<VrfSeed>,
    pub missing_body: bool,
    pub body_hash: Option<Blake2sHash>,
    pub state_root: Option<Blake2bHash>,
    pub diff_root: Option<Blake2bHash>,
    pub history_root: Option<Blake2bHash>,

    // Skip only
    pub skip_block_proof: Option<SkipBlockProof>,

    // Micro only
    pub test_micro: bool,
    pub equivocation_proofs: Vec<EquivocationProof>,
    pub transactions: Vec<Transaction>,
    pub extra_data: Vec<u8>,

    // Macro only
    pub test_macro: bool,
    pub parent_election_hash: Option<Blake2bHash>,
    pub tendermint_round: Option<u32>,

    // Election only
    pub test_election: bool,
    pub interlink: Option<Option<Vec<Blake2bHash>>>,
}

impl Default for BlockConfig {
    fn default() -> Self {
        BlockConfig {
            network: None,
            version: None,
            block_number_offset: 0,
            timestamp_offset: 0,
            parent_hash: None,
            seed: None,
            missing_body: false,
            body_hash: None,
            state_root: None,
            diff_root: None,
            history_root: None,
            skip_block_proof: None,
            test_micro: true,
            equivocation_proofs: vec![],
            transactions: vec![],
            extra_data: vec![],
            test_macro: true,
            parent_election_hash: None,
            tendermint_round: None,
            test_election: true,
            interlink: None,
        }
    }
}

/// `config` can be used to generate blocks that can be invalid in some way. config == Default creates a valid block.
pub fn next_micro_block(
    signing_key: &SchnorrKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> MicroBlock {
    let network = config
        .network
        .unwrap_or_else(|| blockchain.head().network());

    let block_number = (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32;

    let timestamp = (blockchain.timestamp() as i64 + 1 + config.timestamp_offset) as u64;

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

    let inherents = blockchain.create_punishment_inherents(
        block_number,
        &config.equivocation_proofs,
        None,
        None,
    );

    let block_state = BlockState::new(block_number, timestamp);

    let (state_root, diff_root, executed_txns) = blockchain
        .state
        .accounts
        .exercise_transactions(&transactions, &inherents, &block_state)
        .expect("Failed to compute accounts hash during block production");

    let hist_txs = HistoricTransaction::from(
        blockchain.network_id,
        block_number,
        timestamp,
        executed_txns.clone(),
        inherents,
        config
            .equivocation_proofs
            .iter()
            .map(|proof| proof.locator())
            .collect(),
    );

    let mut txn = blockchain.write_transaction();

    let history_root = config.history_root.clone().unwrap_or_else(|| {
        blockchain
            .history_store
            .add_to_history(&mut txn, block_number, &hist_txs)
            .expect("Failed to compute history root during block production.")
            .0
    });

    txn.abort();

    let body = MicroBody {
        equivocation_proofs: config.equivocation_proofs.clone(),
        transactions: executed_txns,
    };

    let header = MicroHeader {
        network,
        version: config.version.unwrap_or(Policy::VERSION),
        block_number,
        timestamp,
        parent_hash,
        seed,
        extra_data: config.extra_data.clone(),
        state_root,
        body_root: config.body_hash.clone().unwrap_or_else(|| body.hash()),
        diff_root,
        history_root,
        ..Default::default()
    };

    let hash = header.hash();
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
    let network = config
        .network
        .unwrap_or_else(|| blockchain.head().network());

    let block_number = (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32;

    let timestamp = if config.timestamp_offset != 0 {
        (blockchain.timestamp() as i64 + config.timestamp_offset) as u64
    } else {
        blockchain.timestamp() + Policy::MINIMUM_PRODUCER_TIMEOUT
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
    let inherents =
        blockchain.create_punishment_inherents(block_number, &[], Some(skip_block_info), None);

    let block_state = BlockState::new(block_number, timestamp);

    let (real_state_root, real_diff_root, _) = blockchain
        .state
        .accounts
        .exercise_transactions(&[], &inherents, &block_state)
        .expect("Failed to compute accounts hash during block production");

    let state_root = config.state_root.clone().unwrap_or(real_state_root);
    let diff_root = config.diff_root.clone().unwrap_or(real_diff_root);

    let hist_txs = HistoricTransaction::from(
        blockchain.network_id,
        block_number,
        timestamp,
        vec![],
        inherents,
        vec![],
    );

    let mut txn = blockchain.write_transaction();

    let history_root = config.history_root.clone().unwrap_or_else(|| {
        blockchain
            .history_store
            .add_to_history(&mut txn, block_number, &hist_txs)
            .expect("Failed to compute history root during block production.")
            .0
    });

    txn.abort();

    let body = MicroBody {
        equivocation_proofs: vec![],
        transactions: vec![],
    };

    let header = MicroHeader {
        network,
        version: config.version.unwrap_or(Policy::VERSION),
        block_number,
        timestamp,
        parent_hash,
        seed: prev_seed,
        extra_data: config.extra_data.clone(),
        state_root,
        body_root: config.body_hash.clone().unwrap_or_else(|| body.hash()),
        diff_root,
        history_root,
        ..Default::default()
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

pub fn next_macro_block_proposal(
    signing_key: &SchnorrKeyPair,
    blockchain: &Blockchain,
    config: &BlockConfig,
) -> MacroBlock {
    let network = config
        .network
        .unwrap_or_else(|| blockchain.head().network());

    let block_number = (blockchain.block_number() as i32 + 1 + config.block_number_offset) as u32;

    let timestamp = (blockchain.timestamp() as i64 + config.timestamp_offset) as u64;

    let parent_hash = config
        .parent_hash
        .clone()
        .unwrap_or_else(|| blockchain.head_hash());

    let parent_election_hash = config
        .parent_election_hash
        .clone()
        .unwrap_or_else(|| blockchain.election_head_hash());

    let interlink = config.interlink.clone().unwrap_or_else(|| {
        if Policy::is_election_block_at(block_number) {
            Some(blockchain.election_head().get_next_interlink().unwrap())
        } else {
            None
        }
    });

    let seed = config
        .seed
        .clone()
        .unwrap_or_else(|| blockchain.head().seed().sign_next(signing_key));

    let validators = if Policy::is_election_block_at(blockchain.block_number() + 1) {
        Some(blockchain.next_validators(&seed))
    } else {
        None
    };

    // Get the staking contract PRIOR to any state changes.
    let staking_contract = blockchain.get_staking_contract();

    let next_batch_initial_punished_set = staking_contract
        .punished_slots
        .next_batch_initial_punished_set(block_number, &staking_contract.active_validators);

    let mut header = MacroHeader {
        network,
        version: config.version.unwrap_or(Policy::VERSION),
        block_number,
        round: 0,
        timestamp,
        parent_hash,
        parent_election_hash,
        interlink,
        seed,
        extra_data: config.extra_data.clone(),
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
        validators,
        next_batch_initial_punished_set,
        ..Default::default()
    };

    let reward_transactions = blockchain.create_reward_transactions(&header, &staking_contract);

    let body = MacroBody {
        transactions: reward_transactions,
    };

    header.body_root = config.body_hash.clone().unwrap_or_else(|| body.hash());

    let mut macro_block = MacroBlock {
        header,
        body: Some(body),
        justification: None,
    };

    let inherents: Vec<Inherent> = blockchain.create_macro_block_inherents(&macro_block);

    let block_state = BlockState::new(block_number, timestamp);

    let (state_root, diff_root, _) = blockchain
        .state
        .accounts
        .exercise_transactions(&[], &inherents, &block_state)
        .expect("Failed to compute accounts hash during block production.");

    macro_block.header.state_root = state_root;
    macro_block.header.diff_root = diff_root;

    let hist_txs = HistoricTransaction::from(
        blockchain.network_id,
        block_number,
        timestamp,
        vec![],
        inherents,
        vec![],
    );

    let mut txn = blockchain.write_transaction();

    macro_block.header.history_root = blockchain
        .history_store
        .add_to_history(&mut txn, block_number, &hist_txs)
        .expect("Failed to compute history root during block production.")
        .0;

    txn.abort();

    macro_block
}

pub fn finalize_macro_block(
    voting_key: &BlsKeyPair,
    proposal: ProposalMessage<MacroHeader>,
    body: MacroBody,
    block_hash: Blake2sHash,
    config: &BlockConfig,
) -> MacroBlock {
    let vote = TendermintVote {
        proposal_hash: Some(block_hash),
        id: TendermintIdentifier {
            network: proposal.proposal.network,
            block_number: proposal.proposal.block_number,
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
        header: proposal.proposal,
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

    let block_hash = macro_block_proposal.hash_blake2s();

    let validators =
        blockchain.get_validators_for_epoch(Policy::epoch_at(blockchain.block_number() + 1), None);
    assert!(validators.is_ok());

    Block::Macro(finalize_macro_block(
        voting_key,
        ProposalMessage {
            valid_round: None,
            proposal: macro_block_proposal.header,
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
