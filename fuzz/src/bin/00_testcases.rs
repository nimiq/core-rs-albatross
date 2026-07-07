use std::{fs, io, iter};

use nimiq_collections::BitSet;
use nimiq_keys::Address;
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

    let write = |name: &str, object: String| {
        fs::write(
            format!("in/user_friendly_address/{name}"),
            object.as_bytes(),
        )
    };
    fs::create_dir_all("in/user_friendly_address")?;
    let burn = Address::burn_address();
    write("burn", burn.to_user_friendly_address())?;
    write(
        "burn_nospace",
        burn.to_user_friendly_address().replace(" ", ""),
    )?;

    // Seeds for the `vrf_map_to_curve` differential target. The map accepts arbitrary bytes, so the
    // seeds just need a useful spread: empty, a single byte, and the exact 69-byte shape built at
    // the real call sites (32-byte public key + 1-byte use case + 4-byte nonce + 32-byte entropy),
    // in all-zero/all-one/counting variants to poke the branchy field arithmetic.
    let write =
        |name: &str, bytes: Vec<u8>| fs::write(format!("in/vrf_map_to_curve/{name}"), bytes);
    fs::create_dir_all("in/vrf_map_to_curve")?;
    write("empty", vec![])?;
    write("one_byte", vec![0])?;
    write("call_shape_zeros", vec![0u8; 69])?;
    write("call_shape_ones", vec![0xffu8; 69])?;
    write("call_shape_counting", (0u8..69).collect())?;

    // Seeds for the `keys_scalar_reduction` differential target. Needs >= 96 bytes:
    // [seed | h | k]. Cover all-zero, all-one, and a structured spread.
    let write =
        |name: &str, bytes: Vec<u8>| fs::write(format!("in/keys_scalar_reduction/{name}"), bytes);
    fs::create_dir_all("in/keys_scalar_reduction")?;
    write("zeros", vec![0u8; 96])?;
    write("ones", vec![0xffu8; 96])?;
    write("counting", (0u8..96).collect())?;

    Ok(())
}
