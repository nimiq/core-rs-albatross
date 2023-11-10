#![feature(test)]

extern crate test;

use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_node::TrieNode};
use postcard;
use test::{black_box, Bencher};

fn serialize_trie_node_serde(b: &mut Bencher, node: TrieNode, name: &str) {
    let mut buf = [0; 150000];
    b.iter(|| {
        let _ = black_box(postcard::to_slice(black_box(&node), &mut buf).unwrap());
    });
}

fn serialize_trie_node_raw(b: &mut Bencher, node: TrieNode, name: &str) {
    let mut buf = [0; 150000];
    b.iter(|| {
        let _ = black_box(node.serialize(&mut buf.as_mut_slice()).unwrap());
    });
}

#[bench]
fn serialize_empty_node_serde(b: &mut Bencher) {
    let node = TrieNode::new_empty(KeyNibbles::default());
    serialize_trie_node_serde(b, node, "empty")
}
#[bench]
fn serialize_root_node_serde(b: &mut Bencher) {
    let node = TrieNode::new_root();
    serialize_trie_node_serde(b, node, "root")
}
#[bench]
fn serialize_incomplete_root_node_serde(b: &mut Bencher) {
    let node = TrieNode::new_root_incomplete();
    serialize_trie_node_serde(b, node, "root_incomplete")
}
#[bench]
fn serialize_leaf_node32_serde(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 32]);
    serialize_trie_node_serde(b, node, "leaf32")
}
#[bench]
fn serialize_leaf_node128_serde(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 128]);
    serialize_trie_node_serde(b, node, "leaf128")
}
#[bench]
fn serialize_leaf_node1024_serde(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 1024]);
    serialize_trie_node_serde(b, node, "leaf1024")
}
#[bench]
fn serialize_leaf_node16384_serde(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 16384]);
    serialize_trie_node_serde(b, node, "leaf16834")
}
#[bench]
fn serialize_leaf_node32768_serde(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 32768]);
    serialize_trie_node_serde(b, node, "leaf32768")
}
#[bench]
fn serialize_leaf_node131072_serde(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 131072]);
    serialize_trie_node_serde(b, node, "leaf131072")
}

#[bench]
fn serialize_empty_node_raw(b: &mut Bencher) {
    let node = TrieNode::new_empty(KeyNibbles::default());
    serialize_trie_node_raw(b, node, "empty")
}
#[bench]
fn serialize_root_node_raw(b: &mut Bencher) {
    let node = TrieNode::new_root();
    serialize_trie_node_raw(b, node, "root")
}
#[bench]
fn serialize_incomplete_root_node_raw(b: &mut Bencher) {
    let node = TrieNode::new_root_incomplete();
    serialize_trie_node_raw(b, node, "root_incomplete")
}
#[bench]
fn serialize_leaf_node32_raw(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 32]);
    serialize_trie_node_raw(b, node, "leaf32")
}
#[bench]
fn serialize_leaf_node128_raw(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 128]);
    serialize_trie_node_raw(b, node, "leaf128")
}
#[bench]
fn serialize_leaf_node1024_raw(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 1024]);
    serialize_trie_node_raw(b, node, "leaf1024")
}
#[bench]
fn serialize_leaf_node16384_raw(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 16384]);
    serialize_trie_node_raw(b, node, "leaf16834")
}
#[bench]
fn serialize_leaf_node32768_raw(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 32768]);
    serialize_trie_node_raw(b, node, "leaf32768")
}
#[bench]
fn serialize_leaf_node131072_raw(b: &mut Bencher) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 131072]);
    serialize_trie_node_raw(b, node, "leaf131072")
}
