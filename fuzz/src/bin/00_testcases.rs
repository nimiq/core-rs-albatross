use std::{fs, io, iter};

use nimiq_collections::BitSet;
use nimiq_primitives::{
    key_nibbles::KeyNibbles,
    trie::trie_node::{TrieNode, TrieNodeChild},
};
use nimiq_serde::Serialize;

fn write_any<T: Serialize>(filename: &str, object: T) -> io::Result<()> {
    let bytes = object.serialize_to_vec();
    fs::write(filename, bytes)
}

fn trie_node_with_value() -> TrieNode {
    let mut result = TrieNode::new_empty("".parse().unwrap());
    result.value = Some(vec![123]);
    result
}

fn trie_node_with_child() -> TrieNode {
    let mut result = TrieNode::new_empty("".parse().unwrap());
    result.children[7] = Some(TrieNodeChild {
        suffix: "ab".parse().unwrap(),
        hash: Default::default(),
    });
    result
}

fn main() -> io::Result<()> {
    let write =
        |name: &str, object: KeyNibbles| write_any(&format!("in/key_nibbles/{name}"), object);
    fs::create_dir_all("in/key_nibbles")?;
    write("root", KeyNibbles::ROOT)?;
    write("one0", "0".parse().unwrap())?;
    write("one1", "1".parse().unwrap())?;
    write("onef", "f".parse().unwrap())?;
    write("two", "9a".parse().unwrap())?;
    write("longer1", "68656c6c6f2c20776f726c6421".parse().unwrap())?;
    write("longer2", "68656c6c6f2c20776f726c64215".parse().unwrap())?;

    let write = |name: &str, object: BitSet| write_any(&format!("in/bitset/{name}"), object);
    fs::create_dir_all("in/bitset")?;
    write("empty", iter::empty().collect())?;
    write("one0", iter::once(0).collect())?;
    write("one1", iter::once(1).collect())?;
    write("one511", iter::once(511).collect())?;
    write("one512", iter::once(512).collect())?;
    write("full512", (0..512).collect())?;

    let write = |name: &str, object: TrieNode| write_any(&format!("in/trie_node/{name}"), object);
    fs::create_dir_all("in/trie_node")?;
    write("root", TrieNode::new_root())?;
    write("root_incomplete", TrieNode::new_root_incomplete())?;
    write("empty", TrieNode::new_empty("".parse().unwrap()))?;
    write("with_child", trie_node_with_child())?;
    write("with_value", trie_node_with_value())?;

    Ok(())
}
