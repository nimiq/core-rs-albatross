fn main() {
    #[cfg(feature = "fuzz")]
    afl::fuzz!(|data: &[u8]| {
        use nimiq_primitives::trie::trie_node::TrieNode;
        use nimiq_serde::Deserialize as _;

        let _ = TrieNode::deserialize_from_vec(data);
    })
}
