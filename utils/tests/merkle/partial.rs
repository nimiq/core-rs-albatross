use std::cmp;

use nimiq_hash::{Blake2bHash, Blake2bHasher};
use nimiq_test_log::test;
use nimiq_utils::merkle::{compute_root_from_content, partial::*};

#[test]
fn it_correctly_computes_a_simple_proof() {
    let values = vec!["1", "2", "3", "4"];
    /*
     *            h6
     *         /      \
     *      h4         h5
     *     / \         / \
     *   h0  h1      h2  h3
     *   |    |      |    |
     *  v0   v1     v2   v3
     */

    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);

    // ------------
    // Chunk size 1
    // ------------
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 1);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[0].len(), 2);
    assert_eq!(chunks[1].len(), 1);
    assert_eq!(chunks[2].len(), 1);
    assert_eq!(chunks[3].len(), 0);

    // Chunk number 1
    let proof_result = chunks[0].compute_root_from_values(&values[..1], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 1);
    assert_eq!(proof_result.next_index(), 1);

    // Chunk number 2
    let proof_result = chunks[1].compute_root_from_values(&values[1..2], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 1);
    assert_eq!(proof_result.next_index(), 2);

    // Chunk number 3
    let proof_result = chunks[2].compute_root_from_values(&values[2..3], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 2);
    assert_eq!(proof_result.next_index(), 3);

    // Chunk number 4
    let proof_result = chunks[3].compute_root_from_values(&values[3..4], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 0);
    assert_eq!(proof_result.next_index(), 4);

    // ------------
    // Chunk size 2
    // ------------
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 2);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 1);
    assert_eq!(chunks[1].len(), 0);

    // Chunk number 1
    let proof_result = chunks[0].compute_root_from_values(&values[..2], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 1);
    assert_eq!(proof_result.next_index(), 2);

    // Chunk number 2
    let proof_result = chunks[1].compute_root_from_values(&values[2..], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 0);
    assert_eq!(proof_result.next_index(), 4);

    // ------------
    // Chunk size 3
    // ------------
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 3);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 1);
    assert_eq!(chunks[1].len(), 0);

    // Chunk number 1
    let proof_result = chunks[0].compute_root_from_values(&values[..3], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 2);
    assert_eq!(proof_result.next_index(), 3);

    // Chunk number 2
    let proof_result = chunks[1].compute_root_from_values(&values[3..], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 0);
    assert_eq!(proof_result.next_index(), 4);

    // ------------
    // Chunk size 4
    // ------------
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 4);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), 0);

    // Chunk number 1
    let proof_result = chunks[0].compute_root_from_values(&values, None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 0);
    assert_eq!(proof_result.next_index(), 4);
}

#[test]
fn it_correctly_computes_more_complex_proofs() {
    let values = vec!["1", "2", "3"];
    /*
     *            h4
     *         /      \
     *      h3         h2
     *     / \          |
     *   h0  h1        v2
     *   |    |
     *  v0   v1
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);

    // ------------
    // Chunk size 1
    // ------------
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 1);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].len(), 2);
    assert_eq!(chunks[1].len(), 1);
    assert_eq!(chunks[2].len(), 0);

    // Chunk number 1
    let proof_result = chunks[0].compute_root_from_values(&values[..1], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 1);
    assert_eq!(proof_result.next_index(), 1);

    // Chunk number 2
    let proof_result = chunks[1].compute_root_from_values(&values[1..2], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 1);
    assert_eq!(proof_result.next_index(), 2);

    // Chunk number 3
    let proof_result = chunks[2].compute_root_from_values(&values[2..3], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 0);
    assert_eq!(proof_result.next_index(), 3);

    // ------------
    // Chunk size 2
    // ------------
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 2);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 1);
    assert_eq!(chunks[1].len(), 0);

    // Chunk number 1
    let proof_result = chunks[0].compute_root_from_values(&values[..2], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 1);
    assert_eq!(proof_result.next_index(), 2);

    // Chunk number 2
    let proof_result = chunks[1].compute_root_from_values(&values[2..], Some(&proof_result));
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_eq!(proof_result.root(), &root);
    assert_eq!(proof_result.helper_nodes().len(), 0);
    assert_eq!(proof_result.next_index(), 3);

    /*
     * Automatically create and verify proofs for the following trees and varying chunk sizes.
     * We don't check all parameters individually, though.
     *                 h6
     *            /          \
     *          h4           |
     *       /      \        |
     *     h0        h1      |
     *   /   \     /   \     |
     *  v0   v1   v2   v3   v4
     *
     *                  h6
     *            /            \
     *          h4             |
     *       /      \          |
     *     h0        h1        h2
     *   /   \     /   \     /   \
     *  v0   v1   v2   v3   v4   v5
     *
     *                   h6
     *            /               \
     *          h4                 h5
     *       /      \            /   \
     *     h0        h1        h2     h3
     *   /   \     /   \     /   \    |
     *  v0   v1   v2   v3   v4   v5   v6
     */
    let values = ["1", "2", "3", "4", "5", "6", "7"];

    // Vary different trees.
    for end in 5usize..7 {
        let root = compute_root_from_content::<Blake2bHasher, &str>(&values[..end]);

        // Vary chunk sizes.
        for chunk_size in 1..end {
            let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(
                &values[..end],
                chunk_size,
            );
            assert!(
                chunks.is_ok(),
                "Proof builder errored for size {} and chunk_size {}: {:?}",
                end,
                chunk_size,
                chunks.err().unwrap()
            );
            let chunks = chunks.unwrap();

            assert_eq!(chunks.len(), (end + chunk_size - 1) / chunk_size);

            // Verify each chunk.
            let mut prev_proof = None;
            for (chunk_i, chunk) in chunks.iter().enumerate() {
                let start_i = chunk_i * chunk_size;
                let end_i = cmp::min((chunk_i + 1) * chunk_size, end);
                let proof_result =
                    chunk.compute_root_from_values(&values[start_i..end_i], prev_proof.as_ref());
                assert!(
                    proof_result.is_ok(),
                    "Proof #{} errored for size {} and chunk_size {}: {:?}",
                    chunk_i,
                    end,
                    chunk_size,
                    proof_result.err().unwrap()
                );
                let proof_result = proof_result.unwrap();

                assert_eq!(
                    proof_result.root(),
                    &root,
                    "Invalid root for size {}, chunk_size {}, chunk #{}",
                    end,
                    chunk_size,
                    chunk_i
                );
                assert_eq!(
                    proof_result.next_index(),
                    end_i,
                    "Invalid next index for size {}, chunk_size {}, chunk #{}",
                    end,
                    chunk_size,
                    chunk_i
                );
                prev_proof = Some(proof_result);
            }
        }
    }
}

#[test]
fn it_discards_invalid_proofs() {
    let values = vec!["1", "2", "3", "4"];
    /*
     *            h6
     *         /      \
     *      h4         h5
     *     / \         / \
     *   h0  h1      h2  h3
     *   |    |      |    |
     *  v0   v1     v2   v3
     */

    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);

    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 2);
    assert!(
        chunks.is_ok(),
        "Proof builder errored: {:?}",
        chunks.err().unwrap()
    );
    let chunks = chunks.unwrap();

    // Case 1: Invalid input values lead to wrong hash.
    let proof_result = chunks[0].compute_root_from_values(&values[2..], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    assert_ne!(proof_result.root(), &root);

    // Case 2: Invalid proof.
    let proof_result = chunks[1].compute_root_from_values(&values[2..], None);
    match proof_result {
        Err(PartialMerkleProofError::InvalidProof) => {}
        _ => assert!(false, "Case 2 gave invalid response: {:?}", proof_result),
    }

    // Case 3: Invalid previous proof.
    // First create a proof result for a different chunk size.
    let other_chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 1);
    assert!(
        other_chunks.is_ok(),
        "Proof builder errored: {:?}",
        other_chunks.err().unwrap()
    );
    let other_chunks = other_chunks.unwrap();
    let proof_result = other_chunks[0].compute_root_from_values(&values[..1], None);
    assert!(
        proof_result.is_ok(),
        "Proof errored: {:?}",
        proof_result.err().unwrap()
    );
    let proof_result = proof_result.unwrap();

    let proof_result = chunks[1].compute_root_from_values(&values[2..], Some(&proof_result));
    match proof_result {
        Err(_) => {}
        _ => assert!(false, "Case 3 gave invalid response: {:?}", proof_result),
    }

    // Case 4: Invalid chunk size.
    let chunks = PartialMerkleProofBuilder::from_values::<Blake2bHash, &str>(&values, 0);
    match chunks {
        Err(PartialMerkleProofError::InvalidChunkSize) => {}
        _ => assert!(false, "Case 4 gave invalid response: {:?}", proof_result),
    }
}
