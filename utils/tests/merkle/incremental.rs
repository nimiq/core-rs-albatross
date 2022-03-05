use std::cmp;

use nimiq_hash::{Blake3Hash, Blake3Hasher};
use nimiq_utils::merkle::compute_root_from_content;
use nimiq_utils::merkle::incremental::*;

fn incremental(values: &[&str], chunk_size: usize) -> Vec<IncrementalMerkleProof<Blake3Hash>> {
    let mut builder = IncrementalMerkleProofBuilder::<Blake3Hash>::new(chunk_size).unwrap();
    for value in values {
        builder.push_item(value);
    }
    builder.chunks()
}

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

    let root = compute_root_from_content::<Blake3Hasher, &str>(&values);

    // ------------
    // Chunk size 1
    // ------------
    let chunks = incremental(&values, 1);

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
    let chunks = incremental(&values, 2);

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
    let chunks = incremental(&values, 3);

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
    let chunks = incremental(&values, 4);

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
    let root = compute_root_from_content::<Blake3Hasher, &str>(&values);

    // ------------
    // Chunk size 1
    // ------------
    let chunks = incremental(&values, 1);

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
    let chunks = incremental(&values, 2);

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
    let values = vec!["1", "2", "3", "4", "5", "6", "7"];

    // Vary different trees.
    for end in 5usize..7 {
        // Vary chunk sizes.
        for chunk_size in 1..end {
            let chunks = incremental(&values[..end], chunk_size);

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
fn it_can_compute_incremental_proofs() {
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
    let actions = vec![1i32, 1, -1, 1, 1, 1, -1, -1, -1];

    let mut builder = IncrementalMerkleProofBuilder::<Blake3Hash>::new(1).unwrap();
    let mut num = 0;
    for action in actions {
        num = (num as i32 + action) as usize;

        // For the small trees used here, the root for all our Merkle computations is the same.
        // The two techniques only start to differ for larger trees.
        let root = compute_root_from_content::<Blake3Hasher, &str>(&values[..num]);

        if action > 0 {
            builder.push_item(&values[num - 1]);
        } else {
            builder.pop();
        }

        assert_eq!(builder.root().unwrap(), &root);
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

    let root = compute_root_from_content::<Blake3Hasher, &str>(&values);

    let chunks = incremental(&values, 2);

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
        Err(IncrementalMerkleProofError::InvalidProof) => {}
        _ => assert!(false, "Case 2 gave invalid response: {:?}", proof_result),
    }

    // Case 3: Invalid previous proof.
    // First create a proof result for a different chunk size.
    let other_chunks = incremental(&values, 1);
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
    let builder = IncrementalMerkleProofBuilder::<Blake3Hash>::new(0);
    match builder {
        Err(IncrementalMerkleProofError::InvalidChunkSize) => {}
        _ => assert!(false, "Case 4 gave invalid response: {:?}", proof_result),
    }
}
