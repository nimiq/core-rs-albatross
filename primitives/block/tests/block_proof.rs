use std::collections::HashMap;

use nimiq_block::{BlockInclusionProof, MacroBlock, MacroHeader};
use nimiq_primitives::policy::Policy;

#[test]
fn test_interlink_hops_to_block() {
    fn assert_interlink_hops(target: u32, election_head: u32, hops: Vec<u32>) {
        assert_eq!(
            BlockInclusionProof::get_interlink_hops(
                target * Policy::blocks_per_epoch(),
                election_head * Policy::blocks_per_epoch()
            ),
            hops.into_iter()
                .map(|i| i * Policy::blocks_per_epoch())
                .collect::<Vec<u32>>()
        );
    }

    assert_interlink_hops(1, 21, vec![16, 8, 4, 2]);
    assert_interlink_hops(
        100,
        19435,
        vec![16384, 8192, 4096, 2048, 1024, 512, 256, 128, 112, 104],
    );

    // If the election_head already contains an interlink or election_head to the target, the proof is empty
    assert_interlink_hops(8, 10, vec![]);
    assert_interlink_hops(9, 10, vec![]);
}

#[test]
fn test_is_block_proven() {
    // Generate some blocks that form an election-block chain
    let mut blocks = HashMap::<u32, MacroBlock>::new();
    blocks.insert(
        1,
        MacroBlock {
            header: MacroHeader {
                block_number: Policy::blocks_per_epoch() * 1,
                interlink: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    for i in 2..23 {
        let interlink = blocks[&(i - 1)].get_next_interlink().unwrap();
        blocks.insert(
            i,
            MacroBlock {
                header: MacroHeader {
                    block_number: Policy::blocks_per_epoch() * i,
                    interlink: Some(interlink),
                    parent_election_hash: blocks[&(i - 1)].hash(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    // Current election head: 22, target: 1. Claimed proof [16, 8, 4, 2]
    let block_proof = BlockInclusionProof {
        proof: vec![
            blocks[&16].clone(),
            blocks[&8].clone(),
            blocks[&4].clone(),
            blocks[&2].clone(),
        ],
    };
    assert!(block_proof.is_block_proven(&blocks[&22], &blocks[&1]));

    // Current election head: 22, target: 2. Claimed proof [16, 8, 4]
    let block_proof = BlockInclusionProof {
        proof: vec![blocks[&16].clone(), blocks[&8].clone(), blocks[&4].clone()],
    };
    assert!(block_proof.is_block_proven(&blocks[&22], &blocks[&2]));

    // Current election head: 10, target: 8. Claimed proof []
    let block_proof = BlockInclusionProof { proof: vec![] };
    assert!(block_proof.is_block_proven(&blocks[&10], &blocks[&8]));

    // Current election head: 10, target: 9. Claimed proof []
    let block_proof = BlockInclusionProof { proof: vec![] };
    assert!(block_proof.is_block_proven(&blocks[&10], &blocks[&9]));

    // Current election head: 22, target: 1. Claimed proof [17 (wrong), 8, 4, 2]
    let block_proof = BlockInclusionProof {
        proof: vec![
            blocks[&17].clone(),
            blocks[&8].clone(),
            blocks[&4].clone(),
            blocks[&2].clone(),
        ],
    };
    assert!(!block_proof.is_block_proven(&blocks[&22], &blocks[&1]));
}
