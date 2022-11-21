# Light Blockchain

This crate implements the structures and methods necessary to allow Light nodes to:

- Sync using the zero-knowledge proofs (recursive SNARKs) that are created using the Light Sync crate. Useful to sync to
  the latest election block. It is very fast since the proof allow us to "_jump_" all the way from the genesis block to
  the latest election block. 
- Sync by pushing macro blocks into the chain. Useful to sync to the latest checkpoint block.
- Push blocks into the chain. This can be used either to sync to the latest micro block or to simply keep following the chain.
- Push election blocks _backwards_. This is when we already have an election block and want to add that block's parent
  election block to the chain. It is very fast since we only need to verify the hash of the past election block. Useful
  if you need to verify transaction proofs for previous epochs.
- Store blocks and other essential information for the Light node.
- Verify Merkle proofs of the inclusion of an account or transaction in the Account Tree or History Tree, respectively.

Note that this crate doesn't interact with other nodes, or with the external world, at all. It requires some other crate
to pass to it all the blocks and proofs mentioned above. What this crate does is verify, process and store those blocks
and proofs.

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be licensed as above, without any additional terms or
conditions.