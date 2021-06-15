# Trie
This crate implements a Merkle Radix Trie which is a hybrid between a Merkle tree and a Radix trie.
Like a Merkle tree each node contains the hashes of all its children. That creates a tree that is resistant to
unauthorized modification and allows proofs of inclusion and exclusion.
Like a Radix trie each node position is determined by its key, and its space optimized by having each "only child" node
merged with its parent.
We keep all values at the edges of the trie, at the leaf nodes. The branch nodes keep only references to its children. In
this respect it is different from the Patricia Merkle Trie used on other chains.
It is generic over the values and makes use of Nimiq's database for storage.

## License
Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).

### Contribution
Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as
defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.