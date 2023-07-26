use ark_ec::Group;
use nimiq_block::{
    Block, BlockError, BlockHeader, EquivocationProof, ForkProof, MacroBlock, MacroBody,
    MacroHeader, MicroBlock, MicroBody, MicroHeader, MicroJustification, MultiSignature,
    SkipBlockProof,
};
use nimiq_bls::{AggregateSignature, G2Projective, PublicKey as BlsPublicKey};
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::{Address, KeyPair, PublicKey as SchnorrPublicKey, Signature};
use nimiq_primitives::{networks::NetworkId, policy::Policy, slots_allocation::ValidatorsBuilder};
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::generate_transactions;
use nimiq_transaction::ExecutedTransaction;
use nimiq_vrf::VrfSeed;

#[test]
fn test_verify_header_version() {
    let mut micro_header = MicroHeader {
        version: Policy::VERSION - 1,
        block_number: 1,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: [].to_vec(),
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };
    let header = BlockHeader::Micro(micro_header.clone());

    // Check version at header level
    assert_eq!(header.verify(false), Err(BlockError::UnsupportedVersion));

    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: None,
        body: None,
    });

    // Error should remain at block level
    assert_eq!(block.verify(), Err(BlockError::UnsupportedVersion));

    // Fix the version and check that it passes
    micro_header.version = Policy::VERSION;
    let header = BlockHeader::Micro(micro_header);
    assert_eq!(header.verify(false), Ok(()));
}

#[test]
fn test_verify_header_extra_data() {
    let mut micro_header = MicroHeader {
        version: Policy::VERSION,
        block_number: 1,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![0; 33],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };
    let header = BlockHeader::Micro(micro_header.clone());

    // Check extra data field at header level
    assert_eq!(header.verify(false), Err(BlockError::ExtraDataTooLarge));
    // Error should remain for a skip block
    assert_eq!(header.verify(true), Err(BlockError::ExtraDataTooLarge));

    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: None,
        body: None,
    });

    // Error should remain at block level
    assert_eq!(block.verify(), Err(BlockError::ExtraDataTooLarge));

    // Fix the extra data field and check that it passes
    micro_header.extra_data = vec![0; 32];
    let header = BlockHeader::Micro(micro_header.clone());
    assert_eq!(header.verify(false), Ok(()));
    // Error should remain for a skip block
    assert_eq!(header.verify(true), Err(BlockError::ExtraDataTooLarge));

    // Fix the extra data field for a skip block and check that it passes
    micro_header.extra_data = [].to_vec();
    let header = BlockHeader::Micro(micro_header);
    assert_eq!(header.verify(true), Ok(()));
}

#[test]
fn test_verify_body_root() {
    let mut micro_header = MicroHeader {
        version: Policy::VERSION,
        block_number: 1,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![0; 30],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let micro_justification = MicroJustification::Micro(Signature::default());

    let micro_body = MicroBody {
        equivocation_proofs: [].to_vec(),
        transactions: [].to_vec(),
    };

    // Build a block with body
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body.clone()),
    });

    // The body root check must fail
    assert_eq!(block.verify(), Err(BlockError::BodyHashMismatch));

    // Fix the body root and check that it passes
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    assert_eq!(block.verify(), Ok(()));
}

#[test]
fn test_verify_skip_block() {
    let mut micro_header = MicroHeader {
        version: Policy::VERSION,
        block_number: 1,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let micro_justification = MicroJustification::Skip(SkipBlockProof {
        sig: MultiSignature {
            signature: AggregateSignature::new(),
            signers: BitSet::default(),
        },
    });

    let transactions: Vec<ExecutedTransaction> =
        generate_transactions(&KeyPair::default(), 1, NetworkId::UnitAlbatross, 1, 0)
            .iter()
            .map(|tx| ExecutedTransaction::Ok(tx.clone()))
            .collect();

    let mut micro_body = MicroBody {
        equivocation_proofs: vec![],
        transactions,
    };

    // Build a block with body
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body.clone()),
    });

    // The skip block body should fail
    assert_eq!(block.verify(), Err(BlockError::InvalidSkipBlockBody));

    // Fix the body with empty transactions and check that it passes
    micro_body.transactions = vec![];
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    assert_eq!(block.verify(), Ok(()));
}

#[test]
fn test_verify_micro_block_body_txns() {
    let mut micro_header = MicroHeader {
        version: Policy::VERSION,
        block_number: 1,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let micro_justification = MicroJustification::Micro(Signature::default());

    let txns: Vec<ExecutedTransaction> =
        generate_transactions(&KeyPair::default(), 1, NetworkId::UnitAlbatross, 5, 0)
            .iter()
            .map(|tx| ExecutedTransaction::Ok(tx.clone()))
            .collect();

    // Lets have a duplicate transaction
    let mut txns_dup = txns.clone();
    txns_dup.push(txns.first().unwrap().clone());

    let mut micro_body = MicroBody {
        equivocation_proofs: [].to_vec(),
        transactions: txns_dup.clone(),
    };

    // Build a block with body with a duplicate transaction
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body.clone()),
    });

    // The body check should fail
    assert_eq!(block.verify(), Err(BlockError::DuplicateTransaction));

    // Fix the body with empty transactions and check that it passes
    txns_dup.pop();
    micro_body.transactions = txns_dup;
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    assert_eq!(block.verify(), Ok(()));

    // Now modify the validity start height
    let txns: Vec<ExecutedTransaction> = generate_transactions(
        &KeyPair::default(),
        Policy::blocks_per_epoch(),
        NetworkId::UnitAlbatross,
        5,
        0,
    )
    .iter()
    .map(|tx| ExecutedTransaction::Ok(tx.clone()))
    .collect();

    let micro_body = MicroBody {
        equivocation_proofs: vec![],
        transactions: txns,
    };

    // Build a block with body with the expired transactions
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    // The body check should fail
    assert_eq!(block.verify(), Err(BlockError::ExpiredTransaction));
}

#[test]
fn test_verify_micro_block_body_fork_proofs() {
    let genesis_block_number = Policy::genesis_block_number();
    let mut micro_header = MicroHeader {
        version: Policy::VERSION,
        block_number: 1 + genesis_block_number,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        seed: VrfSeed::default(),
        extra_data: vec![],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let mut micro_header_1 = micro_header.clone();
    micro_header_1.block_number += 1;

    let mut micro_header_2 = micro_header_1.clone();
    micro_header_2.block_number += 1;

    let fork_proof_1 = ForkProof::new(
        micro_header.clone(),
        Signature::default(),
        micro_header_1.clone(),
        Signature::default(),
        VrfSeed::default(),
    );

    let fork_proof_2 = ForkProof::new(
        micro_header_1.clone(),
        Signature::default(),
        micro_header_2.clone(),
        Signature::default(),
        VrfSeed::default(),
    );

    let fork_proof_3 = ForkProof::new(
        micro_header.clone(),
        Signature::default(),
        micro_header_2.clone(),
        Signature::default(),
        VrfSeed::default(),
    );

    let micro_justification = MicroJustification::Micro(Signature::default());

    let mut fork_proofs = vec![fork_proof_1, fork_proof_2, fork_proof_3];
    fork_proofs.sort_by_key(|p| EquivocationProof::from(p.clone()).sort_key());
    fork_proofs.reverse();
    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    // Build a block with body with a the unsorted fork proofs
    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    // The body check should fail
    assert_eq!(block.verify(), Err(BlockError::ForkProofsNotOrdered));

    // Sort fork proofs and re-build block
    fork_proofs.sort_by_key(|p| EquivocationProof::from(p.clone()).sort_key());
    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    assert_eq!(block.verify(), Ok(()));

    // Lets have a duplicate fork proof
    fork_proofs.push(fork_proofs.last().unwrap().clone());
    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header.clone(),
        justification: Some(micro_justification.clone()),
        body: Some(micro_body),
    });

    assert_eq!(block.verify(), Err(BlockError::DuplicateForkProof));

    // Now modify the block height of the first header of the first fork proof
    let mut micro_header_large_block_number_1 = micro_header.clone();
    micro_header_large_block_number_1.block_number =
        Policy::blocks_per_epoch() + genesis_block_number;
    let mut micro_header_large_block_number_2 = micro_header_large_block_number_1.clone();
    micro_header_large_block_number_2.block_number += 1;
    fork_proofs.pop().unwrap();
    fork_proofs.push(ForkProof::new(
        micro_header_large_block_number_1,
        Signature::default(),
        micro_header_large_block_number_2,
        Signature::default(),
        VrfSeed::default(),
    ));
    fork_proofs.sort_by_key(|p| EquivocationProof::from(p.clone()).sort_key());

    let micro_body = MicroBody {
        equivocation_proofs: fork_proofs.iter().cloned().map(Into::into).collect(),
        transactions: [].to_vec(),
    };

    micro_header.body_root = micro_body.hash();
    let block = Block::Micro(MicroBlock {
        header: micro_header,
        justification: Some(micro_justification),
        body: Some(micro_body),
    });

    // The first fork proof should no longer be valid
    assert_eq!(block.verify(), Err(BlockError::InvalidForkProof));
}

#[test]
fn test_verify_election_macro_body() {
    let mut macro_header = MacroHeader {
        version: Policy::VERSION,
        block_number: Policy::genesis_block_number() + Policy::blocks_per_epoch(),
        round: 0,
        timestamp: 0,
        parent_hash: Blake2bHash::default(),
        parent_election_hash: Blake2bHash::default(),
        interlink: Some(vec![]),
        seed: VrfSeed::default(),
        extra_data: vec![0; 30],
        state_root: Blake2bHash::default(),
        body_root: Blake2sHash::default(),
        diff_root: Blake2bHash::default(),
        history_root: Blake2bHash::default(),
    };

    let mut macro_body = MacroBody {
        validators: None,
        next_batch_initial_punished_set: BitSet::default(),
        transactions: vec![],
    };
    macro_header.body_root = macro_body.hash();

    // Build an election macro block
    let block = Block::Macro(MacroBlock {
        header: macro_header.clone(),
        justification: None,
        body: Some(macro_body.clone()),
    });

    // The validators check should fail
    assert_eq!(block.verify(), Err(BlockError::InvalidValidators));

    // Fix the validators set
    let mut validators = ValidatorsBuilder::new();
    for _ in 0..Policy::SLOTS {
        validators.push(
            Address::default(),
            BlsPublicKey::new(G2Projective::generator()).compress(),
            SchnorrPublicKey::default(),
        );
    }
    macro_body.validators = Some(validators.build());

    macro_header.body_root = macro_body.hash();
    let block = Block::Macro(MacroBlock {
        header: macro_header,
        justification: None,
        body: Some(macro_body),
    });

    // Skipping the verification of the PK tree root should make the verify function to pass
    assert_eq!(block.verify(), Ok(()));
}
